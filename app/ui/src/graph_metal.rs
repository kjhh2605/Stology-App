//! Perspective, depth-tested Metal renderer for the local 3D knowledge map.
//!
//! * Spheres are analytic: each instance is one camera-facing quad whose fragment
//!   shader intersects the view ray with the sphere and writes the real depth.
//!   The silhouette is therefore round at every zoom level, and it matches the
//!   ray/sphere picking pixel for pixel.
//! * Relations and arrowheads arrive as screen-space triangles with per-vertex
//!   edge distances, so their width is constant on screen and analytically
//!   anti-aliased.
//! * Everything renders into a 4x multisampled target that resolves into the
//!   drawable. MTKView is paused and there is no idle rendering loop.
use super::math::Camera;
use objc2::rc::{Allocated, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send, Encode, Encoding};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView};
use objc2_foundation::{MainThreadMarker, NSRect, NSString};
use std::cell::{Cell, RefCell};

#[link(name = "Metal", kind = "framework")]
extern "C" {
    fn MTLCreateSystemDefaultDevice() -> *mut AnyObject;
}
#[link(name = "MetalKit", kind = "framework")]
extern "C" {}

const COLOR_FORMAT: usize = 80; // MTLPixelFormatBGRA8Unorm
const DEPTH_FORMAT: usize = 252; // MTLPixelFormatDepth32Float
const SAMPLES: usize = 4;
const NEAR: f32 = 0.5;
const FAR: f32 = 10000.0;
pub const BACKGROUND: [f64; 4] = [0.972, 0.978, 0.992, 1.0];
thread_local! {static ERROR:RefCell<Option<String>>=const{RefCell::new(None)};}
pub fn last_error() -> Option<String> {
    ERROR.with(|v| v.borrow().clone())
}
fn record(error: impl Into<String>) {
    ERROR.with(|v| *v.borrow_mut() = Some(error.into()));
}

/// Screen-space triangle vertex. `position` is normalised device x, y and depth;
/// `distance` holds up to three inside-positive edge distances in logical pixels
/// that the fragment stage turns into coverage.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FlatVertex {
    pub position: [f32; 3],
    pub distance: [f32; 3],
    pub color: [f32; 4],
}
/// One analytic sphere, or one halo ring when drawn through the halo pipeline.
/// Sphere colour is linear albedo with opacity in alpha; `params.x` is the
/// selected-material amount (clearcoat) in 0..1.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct SphereInstance {
    pub center: [f32; 3],
    pub radius: f32,
    pub color: [f32; 4],
    pub params: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Uniforms {
    right: [f32; 4],
    up: [f32; 4],
    toward: [f32; 4],
    eye: [f32; 4],
    projection: [f32; 4],
    viewport: [f32; 4],
    key: [f32; 4],
    fill: [f32; 4],
    rim: [f32; 4],
    sky: [f32; 4],
}
#[repr(C)]
#[derive(Clone, Copy)]
struct ClearColor {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}
unsafe impl Encode for ClearColor {
    const ENCODING: Encoding = Encoding::Struct(
        "?",
        &[
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
            Encoding::Double,
        ],
    );
}
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Origin {
    x: usize,
    y: usize,
    z: usize,
}
unsafe impl Encode for Origin {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[usize::ENCODING, usize::ENCODING, usize::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Size {
    width: usize,
    height: usize,
    depth: usize,
}
unsafe impl Encode for Size {
    const ENCODING: Encoding =
        Encoding::Struct("?", &[usize::ENCODING, usize::ENCODING, usize::ENCODING]);
}
#[repr(C)]
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Region {
    origin: Origin,
    size: Size,
}
unsafe impl Encode for Region {
    const ENCODING: Encoding = Encoding::Struct("?", &[Origin::ENCODING, Size::ENCODING]);
}

// Light arrangement follows the web map (prod-ui GraphScene): a hemisphere plus
// world-fixed key, fill and rim lights on an opaque satin material, sRGB out.
// The response curve is tuned for a light canvas so a concept's lit side reads
// as the brand indigo instead of sinking towards navy.
const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Uniforms {
    float4 right; float4 up; float4 toward; float4 eye;
    float4 projection;   // focal x, focal y, near, far
    float4 viewport;     // centre shift in NDC (x, y), pixels per point, focal length in pixels
    float4 key; float4 fill; float4 rim; float4 sky;   // view-space directions
};
struct Instance { packed_float3 center; float radius; packed_float4 color; packed_float4 params; };
struct FlatVertex { packed_float3 position; packed_float3 distance; packed_float4 color; };

struct SphereVarying {
    float4 position [[position]];
    float3 viewPos;
    float3 center [[flat]];
    float radius [[flat]];
    float4 color [[flat]];
    float4 params [[flat]];
};
static float3 to_view(float3 world, constant Uniforms& u) {
    float3 d = world - u.eye.xyz;
    return float3(dot(d, u.right.xyz), dot(d, u.up.xyz), dot(d, u.toward.xyz));
}
static float ndc_depth(float depth, constant Uniforms& u) {
    float n = u.projection.z, f = u.projection.w;
    // Metal's homogeneous depth is [0,w], not OpenGL's [-w,w].
    return f / (f - n) * depth - n * f / (f - n);
}
static float4 to_clip(float3 v, constant Uniforms& u) {
    float depth = -v.z;
    float4 clip = float4(v.x * u.projection.x, v.y * u.projection.y, ndc_depth(depth, u), depth);
    clip.xy += u.viewport.xy * clip.w;
    return clip;
}
// A quad on the plane through the centre, perpendicular to the centre ray, sized
// to the tangent cone: it bounds the perspective silhouette exactly.
vertex SphereVarying sphere_v(const device Instance* instances [[buffer(1)]],
                              constant Uniforms& u [[buffer(2)]],
                              uint vid [[vertex_id]], uint iid [[instance_id]]) {
    Instance in = instances[iid];
    SphereVarying out;
    float3 c = to_view(float3(in.center), u);
    float r = in.radius;
    float d2 = dot(c, c);
    out.center = c; out.radius = r; out.color = float4(in.color); out.params = float4(in.params);
    out.viewPos = float3(0.0);
    if (c.z > -u.projection.z || d2 <= r * r * 1.0404 || r <= 0.0 || in.color[3] <= 0.0) {
        out.position = float4(0.0, 0.0, -1.0, 1.0);
        return out;
    }
    float d = sqrt(d2);
    float3 fwd = c / d;
    float3 ax = cross(float3(0.0, 1.0, 0.0), fwd);
    float al = length(ax);
    ax = al > 1e-4 ? ax / al : float3(1.0, 0.0, 0.0);
    float3 ay = cross(fwd, ax);
    float pad = 1.5 * d / u.viewport.w;
    float halfSize = r * d / sqrt(d2 - r * r) + pad;
    float2 corner = float2((vid & 1) ? 1.0 : -1.0, (vid & 2) ? 1.0 : -1.0);
    float3 p = c + (ax * corner.x + ay * corner.y) * halfSize;
    out.viewPos = p;
    out.position = to_clip(p, u);
    return out;
}

static float3 linear_to_srgb(float3 c) {
    float3 lo = c * 12.92;
    float3 hi = 1.055 * pow(max(c, 0.0), float3(1.0 / 2.4)) - 0.055;
    return float3(c.x <= 0.0031308 ? lo.x : hi.x, c.y <= 0.0031308 ? lo.y : hi.y, c.z <= 0.0031308 ? lo.z : hi.z);
}
static float interleaved_noise(float2 pixel) {
    return fract(52.9829189 * fract(dot(pixel, float2(0.06711056, 0.00583715))));
}
struct SphereOut { float4 color [[color(0)]]; float depth [[depth(any)]]; };
fragment SphereOut sphere_f(SphereVarying in [[stage_in]], constant Uniforms& u [[buffer(2)]]) {
    float3 dir = normalize(in.viewPos);
    float3 c = in.center;
    float r = in.radius;
    float b = dot(dir, c);
    float h2 = max(dot(c, c) - b * b, 0.0);
    float h = sqrt(h2);
    // Distance from the view ray to the centre is smooth across the rim, so its
    // screen derivative gives exact sub-pixel coverage for the silhouette.
    float aa = max(fwidth(h), 1e-6);
    float coverage = clamp((r - h) / aa + 0.5, 0.0, 1.0);
    if (coverage <= 0.0) discard_fragment();
    float t = b - sqrt(max(r * r - h2, 0.0));
    float3 hit = dir * t;
    float3 n = normalize(hit - c);
    float3 v = -dir;

    // A satin ball whose lit side is the brand colour itself: hemisphere
    // ambient, a wrapped key so the terminator stays soft, a faint fill, one
    // broad highlight and a cool sheen along the far rim. The selected concept
    // gets a second, tighter clearcoat highlight.
    float selected = saturate(in.params.x);
    float3 albedo = in.color.rgb;
    float NoV = max(dot(n, v), 0.0);
    float sky = dot(n, u.sky.xyz) * 0.5 + 0.5;
    float3 ambient = mix(float3(0.42, 0.43, 0.53), float3(0.72, 0.74, 0.83), sky);
    float key = saturate((dot(n, u.key.xyz) + 0.30) / 1.30);
    key *= key;
    float fill = saturate(dot(n, u.fill.xyz));
    float3 color = albedo * (ambient + 0.46 * key + float3(0.074, 0.079, 0.10) * fill);
    color += albedo * 0.05 * selected;
    float keyGloss = pow(saturate(dot(n, normalize(u.key.xyz + v))), mix(34.0, 46.0, selected));
    float coatGloss = pow(saturate(dot(n, normalize(u.key.xyz + v))), 220.0) * selected;
    float fillGloss = pow(saturate(dot(n, normalize(u.fill.xyz + v))), 64.0);
    float sheen = pow(1.0 - NoV, 3.5) * saturate(dot(n, u.rim.xyz) * 0.6 + 0.4);
    color += float3(1.0) * (0.24 * keyGloss + 0.42 * coatGloss)
           + float3(0.86, 0.90, 1.0) * 0.03 * fillGloss
           + float3(0.62, 0.68, 1.0) * 0.12 * sheen;
    color = linear_to_srgb(saturate(color));
    // Break up 8-bit banding in the slow gradients of a large sphere.
    color += (interleaved_noise(in.position.xy) - 0.5) / 255.0;

    SphereOut out;
    out.color = float4(saturate(color), coverage * in.color.a);
    float depth = -hit.z;
    out.depth = saturate(ndc_depth(depth, u) / depth);
    return out;
}
// Soft glow around the selected or hovered node; same quad as the sphere. It
// starts at the surface (the sphere itself hides what lies inside) and fades out.
fragment float4 halo_f(SphereVarying in [[stage_in]]) {
    float3 dir = normalize(in.viewPos);
    float b = dot(dir, in.center);
    float h = sqrt(max(dot(in.center, in.center) - b * b, 0.0));
    float q = h / in.radius;
    if (q >= 1.0) discard_fragment();
    float fall = 1.0 - smoothstep(0.40, 1.0, q);
    float alpha = 0.46 * pow(fall, 1.6) * in.color.a;
    alpha += (interleaved_noise(in.position.xy) - 0.5) / 255.0;
    return float4(in.color.rgb, saturate(alpha));
}

struct FlatVarying { float4 position [[position]]; float3 dist; float4 color; };
vertex FlatVarying flat_v(const device FlatVertex* vertices [[buffer(0)]],
                          constant Uniforms& u [[buffer(2)]], uint vid [[vertex_id]]) {
    FlatVertex in = vertices[vid];
    FlatVarying out;
    out.position = float4(in.position[0], in.position[1], in.position[2], 1.0);
    out.dist = float3(in.distance) * u.viewport.z;
    out.color = float4(in.color);
    return out;
}
fragment float4 flat_f(FlatVarying in [[stage_in]]) {
    float d = min(in.dist.x, min(in.dist.y, in.dist.z));
    float alpha = in.color.a * clamp(d + 0.5, 0.0, 1.0);
    if (alpha <= 0.0) discard_fragment();
    return float4(in.color.rgb, alpha);
}
"#;

/// Everything one presented image needs. `size` is in logical points.
pub struct Scene<'a> {
    pub camera: &'a Camera,
    pub size: (f64, f64),
    pub spheres: &'a [SphereInstance],
    pub halos: &'a [SphereInstance],
    pub flat: &'a [FlatVertex],
}
struct Targets {
    width: usize,
    height: usize,
    color: Retained<AnyObject>,
    depth: Retained<AnyObject>,
}
pub struct Metal {
    pub view: Retained<NSView>,
    device: Retained<AnyObject>,
    queue: Retained<AnyObject>,
    sphere_pipeline: Retained<AnyObject>,
    halo_pipeline: Retained<AnyObject>,
    flat_pipeline: Retained<AnyObject>,
    opaque_depth: Retained<AnyObject>,
    overlay_depth: Retained<AnyObject>,
    targets: RefCell<Option<Targets>>,
    statistics: Cell<RenderStatistics>,
}
#[derive(Clone, Copy, Debug, Default)]
pub struct RenderStatistics {
    pub attempts: u64,
    pub presented: u64,
    pub unavailable_drawables: u64,
}
unsafe fn describe(error: *mut AnyObject, fallback: &str) -> String {
    if error.is_null() {
        return fallback.into();
    }
    let description: Retained<NSString> = msg_send![error, localizedDescription];
    format!("{fallback}: {}", description)
}
fn uniforms(camera: &Camera, width: f64, height: f64, pixel_height: f64) -> Uniforms {
    let (r, u, t) = camera.basis();
    let eye = camera.position();
    let v = |p: super::math::Vec3| [p.x as f32, p.y as f32, p.z as f32, 0.];
    let focal = 1.0 / (super::math::FOV_Y * 0.5).tan();
    let width = width.max(1.);
    let height = height.max(1.);
    // World-fixed lights, expressed in view space for the fragment stage.
    let light = |x: f64, y: f64, z: f64| {
        let l = super::math::Vec3::new(x, y, z).normalized();
        [l.dot(r) as f32, l.dot(u) as f32, l.dot(t) as f32, 0.]
    };
    Uniforms {
        right: v(r),
        up: v(u),
        toward: v(t),
        eye: v(eye),
        projection: [(focal / (width / height)) as f32, focal as f32, NEAR, FAR],
        viewport: [
            (2. * camera.shift_x / width) as f32,
            (2. * camera.shift_y / height) as f32,
            (pixel_height / height) as f32,
            (pixel_height * 0.5 * focal) as f32,
        ],
        key: light(-42., 58., 46.),
        fill: light(46., -18., 28.),
        rim: light(12., 26., -62.),
        sky: light(0., 1., 0.),
    }
}
impl Metal {
    pub unsafe fn new(frame: NSRect) -> Option<Self> {
        ERROR.with(|v| *v.borrow_mut() = None);
        match Self::try_new(frame) {
            Ok(renderer) => Some(renderer),
            Err(error) => {
                record(error);
                None
            }
        }
    }
    unsafe fn try_new(frame: NSRect) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("Metal 뷰는 메인 스레드에서 생성해야 합니다")?;
        let device = Retained::from_raw(MTLCreateSystemDefaultDevice())
            .ok_or("Metal GPU를 사용할 수 없습니다")?;
        let raw: Allocated<NSView> = msg_send![class!(MTKView), alloc];
        let view: Retained<NSView> = msg_send![raw,initWithFrame:frame,device:&*device];
        let _: () = msg_send![&*view,setPaused:true];
        let _: () = msg_send![&*view,setEnableSetNeedsDisplay:false];
        let _: () = msg_send![&*view,setFramebufferOnly:true];
        let _: () = msg_send![&*view,setColorPixelFormat:COLOR_FORMAT];
        let _: () = msg_send![&*view,setClearColor:ClearColor{red:BACKGROUND[0],green:BACKGROUND[1],blue:BACKGROUND[2],alpha:1.}];
        // Text labels live in an AppKit overlay. Present its Metal underlay in
        // the same Core Animation transaction, including one-shot redraws.
        let layer: Option<Retained<AnyObject>> = msg_send![&*view, layer];
        if let Some(layer) = layer {
            let _: () = msg_send![&*layer,setPresentsWithTransaction:true];
        }
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let _ = mtm;
        let queue: Option<Retained<AnyObject>> = msg_send![&*device, newCommandQueue];
        let queue = queue.ok_or("Metal 명령 큐 생성 실패")?;
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let library: Option<Retained<AnyObject>> = msg_send![&*device,newLibraryWithSource:&*NSString::from_str(SHADER),options:std::ptr::null::<AnyObject>(),error:&mut error];
        let library = library.ok_or_else(|| describe(error, "3D 셰이더 컴파일 실패"))?;
        // `coverage`: alpha becomes a sample mask, so opaque spheres get a smooth
        // rim (and can fade) without sorting. `blend`: ordinary alpha blending.
        let pipeline = |vertex: &str,
                        fragment: &str,
                        coverage: bool,
                        blend: bool|
         -> Result<Retained<AnyObject>, String> {
            let vertex: Option<Retained<AnyObject>> =
                msg_send![&*library,newFunctionWithName:&*NSString::from_str(vertex)];
            let fragment: Option<Retained<AnyObject>> =
                msg_send![&*library,newFunctionWithName:&*NSString::from_str(fragment)];
            let descriptor: Retained<AnyObject> =
                msg_send![class!(MTLRenderPipelineDescriptor), new];
            let _: () =
                msg_send![&*descriptor,setVertexFunction:&*vertex.ok_or("3D 정점 함수 없음")?];
            let _: () =
                msg_send![&*descriptor,setFragmentFunction:&*fragment.ok_or("3D 조명 함수 없음")?];
            let _: () = msg_send![&*descriptor,setDepthAttachmentPixelFormat:DEPTH_FORMAT];
            let _: () = msg_send![&*descriptor,setRasterSampleCount:SAMPLES];
            let _: () = msg_send![&*descriptor,setAlphaToCoverageEnabled:coverage];
            let colors: Retained<AnyObject> = msg_send![&*descriptor, colorAttachments];
            let color: Retained<AnyObject> = msg_send![&*colors,objectAtIndexedSubscript:0usize];
            let _: () = msg_send![&*color,setPixelFormat:COLOR_FORMAT];
            if blend {
                let _: () = msg_send![&*color,setBlendingEnabled:true];
                let _: () = msg_send![&*color,setSourceRGBBlendFactor:4usize]; // SourceAlpha
                let _: () = msg_send![&*color,setDestinationRGBBlendFactor:5usize]; // OneMinusSourceAlpha
                let _: () = msg_send![&*color,setSourceAlphaBlendFactor:1usize]; // One
                let _: () = msg_send![&*color,setDestinationAlphaBlendFactor:5usize];
            }
            let mut error: *mut AnyObject = std::ptr::null_mut();
            let state: Option<Retained<AnyObject>> = msg_send![&*device,newRenderPipelineStateWithDescriptor:&*descriptor,error:&mut error];
            state.ok_or_else(|| describe(error, "3D 렌더 파이프라인 생성 실패"))
        };
        let sphere_pipeline = pipeline("sphere_v", "sphere_f", true, false)?;
        let halo_pipeline = pipeline("sphere_v", "halo_f", false, true)?;
        let flat_pipeline = pipeline("flat_v", "flat_f", false, true)?;
        let depth = |compare: usize, write: bool| -> Result<Retained<AnyObject>, String> {
            let descriptor: Retained<AnyObject> = msg_send![class!(MTLDepthStencilDescriptor), new];
            let _: () = msg_send![&*descriptor,setDepthCompareFunction:compare];
            let _: () = msg_send![&*descriptor,setDepthWriteEnabled:write];
            let state: Option<Retained<AnyObject>> =
                msg_send![&*device,newDepthStencilStateWithDescriptor:&*descriptor];
            state.ok_or("깊이 검사 설정 실패".into())
        };
        Ok(Self {
            view,
            sphere_pipeline,
            halo_pipeline,
            flat_pipeline,
            opaque_depth: depth(1, true)?,   // Less
            overlay_depth: depth(3, false)?, // LessEqual, read-only
            device,
            queue,
            targets: RefCell::new(None),
            statistics: Cell::new(RenderStatistics::default()),
        })
    }
    /// Multisampled colour and depth for a resolve target of this size. They are
    /// memoryless where the GPU allows it: nothing but the resolved image is kept.
    unsafe fn targets(
        &self,
        width: usize,
        height: usize,
    ) -> Result<(Retained<AnyObject>, Retained<AnyObject>), String> {
        let mut cache = self.targets.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|t| t.width != width || t.height != height)
        {
            let make = |format: usize| -> Option<Retained<AnyObject>> {
                // Memoryless, then Private.
                for storage in [3usize, 2] {
                    let descriptor: Retained<AnyObject> = msg_send![class!(MTLTextureDescriptor),texture2DDescriptorWithPixelFormat:format,width:width,height:height,mipmapped:false];
                    let _: () = msg_send![&*descriptor,setTextureType:4usize]; // 2DMultisample
                    let _: () = msg_send![&*descriptor,setSampleCount:SAMPLES];
                    let _: () = msg_send![&*descriptor,setStorageMode:storage];
                    let _: () = msg_send![&*descriptor,setUsage:4usize]; // RenderTarget
                    let texture: Option<Retained<AnyObject>> =
                        msg_send![&*self.device,newTextureWithDescriptor:&*descriptor];
                    if texture.is_some() {
                        return texture;
                    }
                }
                None
            };
            let color = make(COLOR_FORMAT).ok_or("3D 다중 샘플 색 버퍼 생성 실패")?;
            let depth = make(DEPTH_FORMAT).ok_or("3D 다중 샘플 깊이 버퍼 생성 실패")?;
            *cache = Some(Targets {
                width,
                height,
                color,
                depth,
            });
        }
        let targets = cache.as_ref().unwrap();
        Ok((targets.color.clone(), targets.depth.clone()))
    }
    unsafe fn buffer<T>(&self, items: &[T], failure: &str) -> Result<Retained<AnyObject>, String> {
        let buffer: Option<Retained<AnyObject>> = msg_send![&*self.device,newBufferWithBytes:items.as_ptr().cast::<std::ffi::c_void>(),length:std::mem::size_of_val(items),options:0usize];
        buffer.ok_or(failure.into())
    }
    unsafe fn encode(
        &self,
        command: &AnyObject,
        resolve: &AnyObject,
        width: usize,
        height: usize,
        scene: &Scene,
    ) -> Result<(), String> {
        let (color, depth) = self.targets(width, height)?;
        let pass = render_pass(&color, resolve, &depth);
        let encoder: Option<Retained<AnyObject>> =
            msg_send![command,renderCommandEncoderWithDescriptor:&*pass];
        let encoder = encoder.ok_or("3D 렌더 인코더 생성 실패")?;
        let _: () = msg_send![&*encoder,setCullMode:0usize];
        let uniform = uniforms(scene.camera, scene.size.0, scene.size.1, height as f64);
        let bytes = (&uniform as *const Uniforms).cast::<std::ffi::c_void>();
        let length = std::mem::size_of::<Uniforms>();
        let _: () = msg_send![&*encoder,setVertexBytes:bytes,length:length,atIndex:2usize];
        let _: () = msg_send![&*encoder,setFragmentBytes:bytes,length:length,atIndex:2usize];
        let result = (|| -> Result<(), String> {
            let quads = |pipeline: &AnyObject,
                         depth: &AnyObject,
                         instances: &[SphereInstance]|
             -> Result<(), String> {
                if instances.is_empty() {
                    return Ok(());
                }
                let buffer = self.buffer(instances, "개념 인스턴스 GPU 버퍼 생성 실패")?;
                let _: () = msg_send![&*encoder,setRenderPipelineState:pipeline];
                let _: () = msg_send![&*encoder,setDepthStencilState:depth];
                let _: () =
                    msg_send![&*encoder,setVertexBuffer:&*buffer,offset:0usize,atIndex:1usize];
                let _: () = msg_send![&*encoder,drawPrimitives:4usize,vertexStart:0usize,vertexCount:4usize,instanceCount:instances.len()];
                Ok(())
            };
            // Opaque spheres first; halos and relations then blend against the
            // finished depth without writing it.
            quads(&self.sphere_pipeline, &self.opaque_depth, scene.spheres)?;
            quads(&self.halo_pipeline, &self.overlay_depth, scene.halos)?;
            if !scene.flat.is_empty() {
                let buffer = self.buffer(scene.flat, "관계 GPU 버퍼 생성 실패")?;
                let _: () = msg_send![&*encoder,setRenderPipelineState:&*self.flat_pipeline];
                let _: () = msg_send![&*encoder,setDepthStencilState:&*self.overlay_depth];
                let _: () =
                    msg_send![&*encoder,setVertexBuffer:&*buffer,offset:0usize,atIndex:0usize];
                let _: () = msg_send![&*encoder,drawPrimitives:3usize,vertexStart:0usize,vertexCount:scene.flat.len()];
            }
            Ok(())
        })();
        let _: () = msg_send![&*encoder, endEncoding];
        result
    }
    pub unsafe fn draw(&self, scene: &Scene) {
        let mut statistics = self.statistics.get();
        statistics.attempts += 1;
        self.statistics.set(statistics);
        // Acquiring CAMetalLayer.nextDrawable avoids MTKView.currentDrawable's
        // per-draw-cycle cache: this overlay-driven renderer never presents the
        // same drawable twice and does not need a continuously running delegate.
        let layer: Option<Retained<AnyObject>> = msg_send![&*self.view, layer];
        let Some(layer) = layer else {
            record("Metal 레이어가 없어 지도를 갱신하지 못했습니다");
            return;
        };
        let drawable: Option<Retained<AnyObject>> = msg_send![&*layer, nextDrawable];
        let Some(drawable) = drawable else {
            statistics.unavailable_drawables += 1;
            self.statistics.set(statistics);
            record("Metal 표시 버퍼를 사용할 수 없어 지도를 갱신하지 못했습니다");
            return;
        };
        let color: Retained<AnyObject> = msg_send![&*drawable, texture];
        let width: usize = msg_send![&*color, width];
        let height: usize = msg_send![&*color, height];
        if width == 0 || height == 0 {
            return;
        }
        let command: Option<Retained<AnyObject>> = msg_send![&*self.queue, commandBuffer];
        let Some(command) = command else {
            record("Metal 명령 생성 실패");
            return;
        };
        if let Err(error) = self.encode(&command, &color, width, height, scene) {
            record(error);
            return;
        }
        // With presentsWithTransaction, MTLCommandBuffer.presentDrawable is
        // explicitly unsuitable: its scheduled handler may run outside the
        // AppKit transaction. Keep presentation on this drawing thread.
        // https://developer.apple.com/documentation/quartzcore/cametallayer/presentswithtransaction
        let _: () = msg_send![&*command, commit];
        let _: () = msg_send![&*command, waitUntilScheduled];
        let status: usize = msg_send![&*command, status];
        if status == 5 {
            let error: *mut AnyObject = msg_send![&*command, error];
            record(describe(error, "Metal 지도 갱신 실패"));
            return;
        }
        let _: () = msg_send![&*drawable, present];
        statistics.presented += 1;
        self.statistics.set(statistics);
    }
    #[allow(dead_code)]
    pub fn statistics(&self) -> RenderStatistics {
        self.statistics.get()
    }
    /// Render into a readable texture and return tightly packed BGRA rows, top
    /// row first. Uses the same pipelines and multisampling as the screen.
    #[allow(dead_code)]
    pub unsafe fn offscreen(
        &self,
        scene: &Scene,
        width: usize,
        height: usize,
    ) -> Result<Vec<u8>, String> {
        let descriptor: Retained<AnyObject> = msg_send![class!(MTLTextureDescriptor),texture2DDescriptorWithPixelFormat:COLOR_FORMAT,width:width,height:height,mipmapped:false];
        let _: () = msg_send![&*descriptor,setStorageMode:0usize]; // Shared
        let _: () = msg_send![&*descriptor,setUsage:4usize];
        let resolve: Option<Retained<AnyObject>> =
            msg_send![&*self.device,newTextureWithDescriptor:&*descriptor];
        let resolve = resolve.ok_or("오프스크린 GPU 텍스처 생성 실패")?;
        let command: Option<Retained<AnyObject>> = msg_send![&*self.queue, commandBuffer];
        let command = command.ok_or("Metal 명령 생성 실패")?;
        self.encode(&command, &resolve, width, height, scene)?;
        let _: () = msg_send![&*command, commit];
        let _: () = msg_send![&*command, waitUntilCompleted];
        let status: usize = msg_send![&*command, status];
        if status != 4 {
            let error: *mut AnyObject = msg_send![&*command, error];
            return Err(describe(error, "실제 GPU 실행 실패"));
        }
        let mut bytes = vec![0; width * height * 4];
        let region = Region {
            origin: Origin { x: 0, y: 0, z: 0 },
            size: Size {
                width,
                height,
                depth: 1,
            },
        };
        let _: () = msg_send![&*resolve,getBytes:bytes.as_mut_ptr().cast::<std::ffi::c_void>(),bytesPerRow:width*4,fromRegion:region,mipmapLevel:0usize];
        Ok(bytes)
    }
}

unsafe fn render_pass(
    color: &AnyObject,
    resolve: &AnyObject,
    depth: &AnyObject,
) -> Retained<AnyObject> {
    let pass: Retained<AnyObject> =
        msg_send![class!(MTLRenderPassDescriptor), renderPassDescriptor];
    let colors: Retained<AnyObject> = msg_send![&*pass, colorAttachments];
    let attachment: Retained<AnyObject> = msg_send![&*colors,objectAtIndexedSubscript:0usize];
    let _: () = msg_send![&*attachment,setTexture:color];
    let _: () = msg_send![&*attachment,setResolveTexture:resolve];
    let _: () = msg_send![&*attachment,setLoadAction:2usize]; // Clear
    let _: () = msg_send![&*attachment,setStoreAction:2usize]; // MultisampleResolve
    let _: () = msg_send![&*attachment,setClearColor:ClearColor{red:BACKGROUND[0],green:BACKGROUND[1],blue:BACKGROUND[2],alpha:1.}];
    let depth_attachment: Retained<AnyObject> = msg_send![&*pass, depthAttachment];
    let _: () = msg_send![&*depth_attachment,setTexture:depth];
    let _: () = msg_send![&*depth_attachment,setLoadAction:2usize];
    let _: () = msg_send![&*depth_attachment,setStoreAction:0usize]; // DontCare
    let _: () = msg_send![&*depth_attachment,setClearDepth:1.0f64];
    pass
}

// ── Visual quality probes (offscreen; no window) ─────────────────────────
/// Radial deviation of a centred sphere silhouette, in pixels. For each of 720
/// directions the 50 % coverage crossing is located with sub-pixel sampling, so
/// both polygon facets and stair-step aliasing show up as deviation.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Silhouette {
    pub mean_radius: f64,
    pub max_deviation: f64,
    pub rms: f64,
}
#[allow(dead_code)]
fn background_distance(bgra: &[u8], width: usize, height: usize, x: f64, y: f64) -> f64 {
    let sample = |ix: isize, iy: isize| -> f64 {
        if ix < 0 || iy < 0 || ix >= width as isize || iy >= height as isize {
            return 0.;
        }
        let i = (iy as usize * width + ix as usize) * 4;
        let bg = [
            (BACKGROUND[2] * 255.).round(),
            (BACKGROUND[1] * 255.).round(),
            (BACKGROUND[0] * 255.).round(),
        ];
        (0..3).map(|c| (bgra[i + c] as f64 - bg[c]).abs()).sum()
    };
    let fx = x - 0.5;
    let fy = y - 0.5;
    let x0 = fx.floor();
    let y0 = fy.floor();
    let tx = fx - x0;
    let ty = fy - y0;
    let (x0, y0) = (x0 as isize, y0 as isize);
    sample(x0, y0) * (1. - tx) * (1. - ty)
        + sample(x0 + 1, y0) * tx * (1. - ty)
        + sample(x0, y0 + 1) * (1. - tx) * ty
        + sample(x0 + 1, y0 + 1) * tx * ty
}
#[allow(dead_code)]
pub fn silhouette_deviation(
    bgra: &[u8],
    width: usize,
    height: usize,
    center: (f64, f64),
    expected: f64,
) -> Option<Silhouette> {
    let mut radii = Vec::with_capacity(720);
    for k in 0..720 {
        let angle = std::f64::consts::TAU * (k as f64 + 0.37) / 720.;
        let (dx, dy) = (angle.cos(), angle.sin());
        let at = |rho: f64| {
            background_distance(
                bgra,
                width,
                height,
                center.0 + dx * rho,
                center.1 + dy * rho,
            )
        };
        let reference = at(expected - 5.);
        if reference < 24. {
            return None;
        }
        let mut previous = (expected - 4., at(expected - 4.) / reference);
        let mut crossing = None;
        let mut rho = expected - 4.;
        while rho < expected + 4. {
            rho += 0.05;
            let value = at(rho) / reference;
            if previous.1 >= 0.5 && value < 0.5 {
                let t = (previous.1 - 0.5) / (previous.1 - value).max(1e-9);
                crossing = Some(previous.0 + (rho - previous.0) * t);
                break;
            }
            previous = (rho, value);
        }
        radii.push(crossing?);
    }
    let mean = radii.iter().sum::<f64>() / radii.len() as f64;
    let max_deviation = radii.iter().map(|r| (r - mean).abs()).fold(0., f64::max);
    let rms = (radii.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / radii.len() as f64).sqrt();
    Some(Silhouette {
        mean_radius: mean,
        max_deviation,
        rms,
    })
}
/// Coverage-weighted width, in pixels, of a stroke crossing the vertical through
/// column `x` between rows `y0` and `y1`.
#[allow(dead_code)]
pub fn stroke_width(
    bgra: &[u8],
    width: usize,
    height: usize,
    x: usize,
    y0: usize,
    y1: usize,
) -> f64 {
    let column = (y0..y1.min(height))
        .map(|y| background_distance(bgra, width, height, x as f64 + 0.5, y as f64 + 0.5))
        .collect::<Vec<_>>();
    let peak = column.iter().copied().fold(0., f64::max);
    if peak < 8. {
        return 0.;
    }
    column.iter().sum::<f64>() / peak
}
/// An 8-bit RGBA bitmap whose pixel storage AppKit allocates and owns.
pub unsafe fn rgba_bitmap(
    width: usize,
    height: usize,
) -> Option<Retained<objc2_app_kit::NSBitmapImageRep>> {
    use objc2::AnyThread;
    use objc2_app_kit::{NSBitmapImageRep, NSDeviceRGBColorSpace};
    NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
        NSBitmapImageRep::alloc(),
        std::ptr::null_mut(),
        width as isize,
        height as isize,
        8,
        4,
        true,
        false,
        NSDeviceRGBColorSpace,
        (width * 4) as isize,
        32,
    )
}
/// Write tightly packed BGRA pixels (top row first) as a PNG file.
#[allow(dead_code)]
pub unsafe fn write_png(
    path: &str,
    bgra: &[u8],
    width: usize,
    height: usize,
) -> Result<(), String> {
    use objc2_app_kit::NSBitmapImageFileType;
    use objc2_foundation::NSDictionary;
    let rep = rgba_bitmap(width, height).ok_or("PNG 비트맵 생성 실패")?;
    let data = rep.bitmapData();
    let row = rep.bytesPerRow() as usize;
    for y in 0..height {
        for x in 0..width {
            let s = (y * width + x) * 4;
            let d = data.add(y * row + x * 4);
            *d = bgra[s + 2];
            *d.add(1) = bgra[s + 1];
            *d.add(2) = bgra[s];
            *d.add(3) = bgra[s + 3];
        }
    }
    let png = rep
        .representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        .ok_or("PNG 인코딩 실패")?;
    if png.writeToFile_atomically(&NSString::from_str(path), true) {
        Ok(())
    } else {
        Err(format!("PNG 저장 실패: {path}"))
    }
}
#[allow(dead_code)]
pub unsafe fn probe_renderer() -> Result<Metal, String> {
    let frame = NSRect::new(
        objc2_foundation::NSPoint::new(0., 0.),
        objc2_foundation::NSSize::new(64., 64.),
    );
    Metal::new(frame).ok_or_else(|| last_error().unwrap_or("Metal unavailable".into()))
}
/// Linear albedo of the map's indigo, for probes.
#[allow(dead_code)]
pub const PROBE_ALBEDO: [f32; 4] = [0.0786, 0.0618, 0.7785, 1.];
/// One sphere on the optical axis whose silhouette has the requested pixel radius.
#[allow(dead_code)]
pub unsafe fn probe_sphere(size: usize, radius_px: f64) -> Result<Vec<u8>, String> {
    let renderer = probe_renderer()?;
    let focal = size as f64 * 0.5 / (super::math::FOV_Y * 0.5).tan();
    let radius = 15.0f64;
    let camera = Camera {
        yaw: 0.,
        pitch: 0.,
        distance: radius * (1. + (focal / radius_px).powi(2)).sqrt(),
        ..Camera::default()
    };
    let sphere = SphereInstance {
        center: [0., 0., 0.],
        radius: radius as f32,
        color: PROBE_ALBEDO,
        params: [0.; 4],
    };
    renderer.offscreen(
        &Scene {
            camera: &camera,
            size: (size as f64, size as f64),
            spheres: &[sphere],
            halos: &[],
            flat: &[],
        },
        size,
        size,
    )
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct SmokeReport {
    pub sphere_technique: &'static str,
    pub samples: usize,
    pub non_background_pixels: usize,
    pub center_rgba: [u8; 4],
    pub draw_order_invariant: bool,
    pub relation_depth_tested: bool,
    pub silhouette_max_deviation_px: [f64; 3],
}
/// Real GPU test, including readback and reversed draw order. Call on macOS's
/// main thread; this does not open or operate the user's app/window.
#[allow(dead_code)]
pub unsafe fn smoke_readback() -> Result<SmokeReport, String> {
    let renderer = probe_renderer()?;
    let camera = Camera {
        yaw: 0.,
        pitch: 0.,
        distance: 400.,
        ..Camera::default()
    };
    let sphere = |z: f32, radius: f32, color: [f32; 4]| SphereInstance {
        center: [0., 0., z],
        radius,
        color,
        params: [0.; 4],
    };
    let front = sphere(45., 45., [0.02, 0.05, 0.80, 1.]);
    let rear = sphere(-30., 64., [0.80, 0.02, 0.01, 1.]);
    let hidden = sphere(500., 40., [1., 1., 0., 1.]);
    // A horizontal green band at the depth of world z = -5: both spheres are
    // nearer at the centre, nothing is nearer at the left edge.
    let band_depth = camera
        .project(super::math::Vec3::new(0., 0., -5.), 128., 128.)
        .map(|p| p.clip[2] / p.clip[3])
        .ok_or("3D 투영 실패")?;
    let vertex = |x: f32, y: f32| FlatVertex {
        position: [x, y, band_depth],
        distance: [100., 100., 100.],
        color: [0.05, 0.70, 0.10, 1.],
    };
    let band = [
        vertex(-0.95, -0.04),
        vertex(0.95, -0.04),
        vertex(0.95, 0.04),
        vertex(-0.95, -0.04),
        vertex(0.95, 0.04),
        vertex(-0.95, 0.04),
    ];
    let render = |camera: &Camera, spheres: &[SphereInstance], flat: &[FlatVertex]| {
        renderer.offscreen(
            &Scene {
                camera,
                size: (128., 128.),
                spheres,
                halos: &[],
                flat,
            },
            128,
            128,
        )
    };
    let a = render(&camera, &[front, rear, hidden], &band)?;
    let b = render(&camera, &[rear, front, hidden], &band)?;
    let pixel = |image: &[u8], x: usize, y: usize| {
        let i = (y * 128 + x) * 4;
        [image[i + 2], image[i + 1], image[i], image[i + 3]]
    };
    let center_rgba = pixel(&a, 64, 64);
    let count = a
        .chunks_exact(4)
        .filter(|v| v[2] < 220 || v[1] < 220 || v[0] < 220)
        .count();
    if count < 1000 || center_rgba[2] < center_rgba[0].saturating_add(50) || center_rgba[3] != 255 {
        return Err(format!(
            "3D GPU readback incorrect: center={center_rgba:?},pixels={count}"
        ));
    }
    if a != b {
        return Err("깊이 검사 오류: 인스턴스 순서에 따라 가림 결과가 달라졌습니다".into());
    }
    // Outside both spheres the band is visible; behind them it is not.
    let outside = pixel(&a, 4, 64);
    let relation_depth_tested =
        outside[1] > outside[0].saturating_add(60) && center_rgba[1] < center_rgba[2];
    if !relation_depth_tested {
        return Err(format!(
            "관계선 깊이 검사 오류: outside={outside:?}, center={center_rgba:?}"
        ));
    }
    let blank = render(&camera, &[], &[])?;
    let before_near = sphere(399.9, 0.05, [1., 0., 0., 1.]);
    let beyond_far = sphere(-10001., 1., [1., 0., 0., 1.]);
    if render(&camera, &[hidden, before_near, beyond_far], &[])? != blank {
        return Err(
            "3D 클리핑 오류: 카메라 뒤 또는 근거리·원거리 범위 밖의 구체가 표시됩니다".into(),
        );
    }
    let turned = Camera {
        yaw: 0.42,
        pitch: 0.28,
        ..camera
    };
    if render(&turned, &[front, rear, hidden], &band)? == a {
        return Err("3D 카메라 오류: 두 축 회전 후 투영 결과가 변경되지 않았습니다".into());
    }
    let mut silhouette = [0.; 3];
    for (slot, (size, radius)) in [(256usize, 32.0f64), (1024, 256.), (2304, 1024.)]
        .into_iter()
        .enumerate()
    {
        let image = probe_sphere(size, radius)?;
        let c = size as f64 / 2.;
        let measured = silhouette_deviation(&image, size, size, (c, c), radius)
            .ok_or("구 윤곽을 측정하지 못했습니다")?;
        if measured.max_deviation > 0.5 || (measured.mean_radius - radius).abs() > 0.75 {
            return Err(format!("구 윤곽 오류: 반지름 {radius}px에서 {measured:?}"));
        }
        silhouette[slot] = measured.max_deviation;
    }
    Ok(SmokeReport {
        sphere_technique: "analytic ray/sphere, true depth",
        samples: SAMPLES,
        non_background_pixels: count,
        center_rgba,
        draw_order_invariant: true,
        relation_depth_tested,
        silhouette_max_deviation_px: silhouette,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_shader_layout_matches_rust() {
        assert_eq!(std::mem::size_of::<FlatVertex>(), 40);
        assert_eq!(std::mem::size_of::<SphereInstance>(), 48);
        assert_eq!(std::mem::size_of::<Uniforms>(), 160);
    }
    #[test]
    fn lights_are_fixed_in_the_world_not_on_the_camera() {
        let mut camera = Camera::default();
        let a = uniforms(&camera, 800., 600., 1200.);
        camera.yaw += 1.;
        let b = uniforms(&camera, 800., 600., 1200.);
        assert!((a.key[0] - b.key[0]).abs() > 0.1);
        assert!((a.viewport[2] - 2.).abs() < 1e-6);
    }
    #[test]
    fn silhouette_metric_separates_round_from_faceted() {
        let size = 256usize;
        let draw = |inside: &dyn Fn(f64, f64) -> bool| {
            let mut image = vec![0u8; size * size * 4];
            for y in 0..size {
                for x in 0..size {
                    // 4x4 supersampled coverage.
                    let mut hits = 0;
                    for sy in 0..4 {
                        for sx in 0..4 {
                            let px = x as f64 + (sx as f64 + 0.5) / 4. - 128.;
                            let py = y as f64 + (sy as f64 + 0.5) / 4. - 128.;
                            hits += inside(px, py) as u32;
                        }
                    }
                    let k = hits as f64 / 16.;
                    let i = (y * size + x) * 4;
                    for (c, bg) in [(0, BACKGROUND[2]), (1, BACKGROUND[1]), (2, BACKGROUND[0])] {
                        image[i + c] = ((bg * 255.).round() * (1. - k) + 40. * k) as u8;
                    }
                    image[i + 3] = 255;
                }
            }
            image
        };
        let round = draw(&|x, y| x.hypot(y) <= 90.);
        let round = silhouette_deviation(&round, size, size, (128., 128.), 90.).unwrap();
        assert!(round.max_deviation < 0.2, "{round:?}");
        // A 12-gon inscribed in the same circle has a 3 px sagitta.
        let polygon = draw(&|x, y| {
            let sector = std::f64::consts::TAU / 12.;
            let angle = y.atan2(x).rem_euclid(sector) - sector / 2.;
            x.hypot(y) * angle.cos() <= 90. * (sector / 2.).cos()
        });
        let polygon = silhouette_deviation(&polygon, size, size, (128., 128.), 89.).unwrap();
        assert!(polygon.max_deviation > 1., "{polygon:?}");
    }
}
