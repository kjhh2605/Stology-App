//! Lavender hide preview: native blur underneath a separate paint view.
//! NSVisualEffectView owns its material rendering; don't override its drawRect.
use objc2::{define_class, msg_send, AnyThread, MainThreadMarker, MainThreadOnly, DefinedClass};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};

fn rect(x:f64,y:f64,w:f64,h:f64)->NSRect {NSRect::new(NSPoint::new(x,y),NSSize::new(w,h))}

define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologySidebarLavenderTint"]
    #[thread_kind = MainThreadOnly]
    #[ivars = bool]
    pub struct LavenderTint;
    impl LavenderTint {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) { unsafe {
            let b = self.bounds();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(b,22.,22.).addClip();
            let colors = NSArray::from_retained_slice(&[
                NSColor::colorWithSRGBRed_green_blue_alpha(233./255.,225./255.,1.,0.16),
                NSColor::colorWithSRGBRed_green_blue_alpha(206./255.,190./255.,247./255.,0.46),
                NSColor::colorWithSRGBRed_green_blue_alpha(170./255.,148./255.,226./255.,0.72),
            ]);
            let stops = [0.,0.4,1.];
            if let Some(gradient) = NSGradient::initWithColors_atLocations_colorSpace(
                NSGradient::alloc(), &colors, stops.as_ptr(), &NSColorSpace::sRGBColorSpace()) {
                gradient.drawInRect_angle(b,if *self.ivars() {180.} else {0.});
            }

        } }
    }
);


// The cue follows the remaining area and clips its right edge as that area closes.
define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologySidebarRemainingAreaArrow"]
    #[thread_kind = MainThreadOnly]
    #[ivars = bool]
    pub struct RemainingAreaArrow;
    impl RemainingAreaArrow {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            let b = self.bounds();
            // A plain right arrow, without a surrounding panel or button shape.
            NSBezierPath::bezierPathWithRect(b).addClip();
            // Keep the glyph full size and left-aligned once its 24pt box no longer fits.
            let left = *self.ivars();
            let x = if left { b.size.width - b.size.width.max(24.) / 2. }
                else { b.size.width.max(24.) / 2. };
            let direction = if left { -1. } else { 1. };
            let y = b.size.height / 2.;
            NSColor::colorWithSRGBRed_green_blue_alpha(103./255.,79./255.,157./255.,1.).setStroke();
            let arrow = NSBezierPath::bezierPath();
            arrow.setLineWidth(2.2);
            arrow.setLineCapStyle(NSLineCapStyle::Round);
            arrow.setLineJoinStyle(NSLineJoinStyle::Round);
            arrow.moveToPoint(NSPoint::new(x-10.*direction,y));
            arrow.lineToPoint(NSPoint::new(x+10.*direction,y));
            arrow.moveToPoint(NSPoint::new(x+3.*direction,y+7.));
            arrow.lineToPoint(NSPoint::new(x+10.*direction,y));
            arrow.lineToPoint(NSPoint::new(x+3.*direction,y-7.));
            arrow.stroke();
        }
    }
);

/// The area between the moving divider's right edge and the sidebar's right edge.
pub fn arrow_frame(panel: NSRect, right_edge: f64) -> NSRect {
    let left = panel.origin.x.min(right_edge);
    rect(left, panel.origin.y + (panel.size.height - 40.) / 2.,
        (right_edge - left).max(0.), 40.)
}
pub fn left_arrow_frame(panel: NSRect, left_edge: f64) -> NSRect {
    rect(left_edge, panel.origin.y + (panel.size.height - 40.) / 2.,
        (panel.origin.x + panel.size.width - left_edge).max(0.), 40.)
}
pub unsafe fn new_arrow(left: bool) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("hide arrow requires main thread");
    let allocated = RemainingAreaArrow::alloc(mtm).set_ivars(left);
    let arrow: Retained<RemainingAreaArrow> = msg_send![super(allocated), initWithFrame:rect(0.,0.,40.,40.)];
    let _: () = msg_send![&arrow, setAccessibilityElement:false];
    arrow.setWantsLayer(true);
    let layer: Retained<AnyObject> = msg_send![&arrow, layer];
    let _: () = msg_send![&layer, setMasksToBounds:true];
    arrow.setHidden(true);
    arrow.into_super()
}

pub unsafe fn new(left: bool) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("hide preview requires main thread");
    let frame=rect(0.,0.,220.,100.);
    let host=NSView::initWithFrame(NSView::alloc(mtm),frame);
    host.setWantsLayer(true);
    let layer:Retained<AnyObject>=msg_send![&host,layer];
    let _:()=msg_send![&layer,setCornerRadius:22.0f64];
    let _:()=msg_send![&layer,setMasksToBounds:true];
    let flexible=NSAutoresizingMaskOptions::ViewWidthSizable|NSAutoresizingMaskOptions::ViewHeightSizable;
    let blur=NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm),frame);
    blur.setMaterial(NSVisualEffectMaterial::Popover);
    blur.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    blur.setState(NSVisualEffectState::Active);
    // Keep the neutral system material subordinate to the lavender tint.
    blur.setAlphaValue(0.35);
    blur.setAutoresizingMask(flexible);
    host.addSubview(&blur);
    let allocated=LavenderTint::alloc(mtm).set_ivars(left);
    let ink:Retained<LavenderTint>=msg_send![super(allocated),initWithFrame:frame];
    ink.setAutoresizingMask(flexible);
    // This plain NSView paints above the material's private compositor layers.
    host.addSubview(&ink);
    let _:()=msg_send![&host,setAccessibilityElement:true];
    let _:()=msg_send![&host,setAccessibilityLabel:&*NSString::from_str(if left {"왼쪽 사이드바 숨김 대기 — 놓으면 접기, Esc로 취소"} else {"오른쪽 사이드바 숨김 대기 — 놓으면 접기, Esc로 취소"})];
    host.setHidden(true);
    host
}
