//! Offscreen visual-quality measurement and scene captures for the 3D knowledge
//! map. Opens no window and reads no user data.
//! cargo run --release --manifest-path local-app/ui/Cargo.toml --example graph-quality -- <output-dir>
#[path = "../src/font.rs"]
mod font;
#[path = "../src/graph.rs"]
mod graph;
#[path = "support/graphs.rs"]
mod graphs;
use graphs::{biology, dense};
use objc2_foundation::{NSPoint, NSRect, NSSize};

const WIDTH: f64 = 780.;
const HEIGHT: f64 = 620.;
const SCALE: f64 = 2.;

fn crop(bgra: &[u8], width: usize, x: usize, y: usize, w: usize, h: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        out.extend_from_slice(&bgra[(row * width + x) * 4..(row * width + x + w) * 4]);
    }
    out
}
unsafe fn save(out: &str, name: &str) {
    let (w, h) = ((WIDTH * SCALE) as usize, (HEIGHT * SCALE) as usize);
    let pixels = graph::capture(WIDTH, HEIGHT, SCALE).expect("capture");
    graph::write_png(&format!("{out}/{name}.png"), &pixels, w, h).expect("png");
    println!(
        "scene {name}: camera distance {:.1}",
        graph::camera_distance()
    );
}
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&out).expect("output directory");
    unsafe {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("macOS main thread");
        let _ = objc2_app_kit::NSApplication::sharedApplication(mtm);

        // 1. Sphere silhouette at three sizes.
        for (size, radius) in [(256usize, 32.0f64), (1024, 256.), (2304, 1024.)] {
            let pixels = graph::probe_sphere(size, radius).expect("sphere probe");
            let c = size as f64 / 2.;
            let s = graph::silhouette_deviation(&pixels, size, size, (c, c), radius)
                .expect("silhouette");
            println!(
                "silhouette radius_px={radius} mean={:.3} max_deviation={:.3} rms={:.3}",
                s.mean_radius, s.max_deviation, s.rms
            );
            // Upper-right rim (45°), where facets and stair-steps are most visible.
            let k = (c + radius * std::f64::consts::FRAC_1_SQRT_2) as usize;
            let side = 96.min(size / 2);
            let (x, y) = (k - side / 2, size - k - side / 2);
            graph::write_png(
                &format!("{out}/rim-{}.png", radius as usize),
                &crop(&pixels, size, x, y, side, side),
                side,
                side,
            )
            .expect("png");
        }

        // 2. Relation width on screen at three zoom levels.
        let mut widths = Vec::new();
        for distance in [300., 900., 2700.] {
            let pixels = graph::probe_relation(512, distance, 2.).expect("relation probe");
            let width = graph::stroke_width(&pixels, 512, 512, 256, 236, 276);
            println!("relation distance={distance} width_px={width:.3}");
            widths.push(width);
        }
        let spread = widths.iter().copied().fold(f64::MIN, f64::max)
            - widths.iter().copied().fold(f64::MAX, f64::min);
        println!("relation width spread_px={spread:.3}");

        // 3. Scenes, through the same composition as the screen.
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(WIDTH, HEIGHT));
        let _map = graph::create(frame, biology());
        graph::settle_now();
        save(&out, "01-overview");
        graph::hover_for_capture("chloroplast");
        save(&out, "02-hover");
        graph::hover_for_capture("");
        graph::search("소");
        graph::settle_now();
        save(&out, "03-search");
        graph::search("");
        graph::select("chloroplast");
        graph::settle_now();
        save(&out, "04-focus");
        graph::filter("site");
        graph::settle_now();
        save(&out, "05-focus-relation");
        graph::filter("");
        // As close as the wheel allows: the case that used to show facets.
        graph::camera_for_capture(0.62, 0.30, Some(1.));
        save(&out, "06-closest");
        graph::select("");
        graph::hover_relation_for_capture("a1");
        save(&out, "07-relation-selected");
        graph::fit();
        graph::settle_now();
        // Orbit to the far side and zoom in: context concepts pass near the eye.
        graph::camera_for_capture(2.4, -0.55, Some(1.));
        save(&out, "08-orbit-near");

        graph::update(dense());
        graph::settle_now();
        save(&out, "09-dense-overview");
        graph::select("n000");
        graph::settle_now();
        save(&out, "10-dense-focus");
        graph::dispose();
    }
}
