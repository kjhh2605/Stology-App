//! On-screen self-test of the 3D knowledge map: the real display link, the real
//! pointer and scroll handlers and the real drawable, in a small window of its
//! own. It never takes keyboard focus and reads no user data. Runs ~15 seconds.
//! cargo run --release --manifest-path local-app/ui/Cargo.toml --example graph-live -- <output-dir>
#[path = "../src/font.rs"]
mod font;
#[path = "../src/graph.rs"]
mod graph;
#[path = "support/graphs.rs"]
mod graphs;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{msg_send, Encoding, MainThreadOnly, RefEncode};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEvent,
    NSEventModifierFlags, NSEventType, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSDate, NSPoint, NSRect, NSSize};
use serde_json::Value;
use std::time::Instant;

#[repr(C)]
struct CGEvent([u8; 0]);
unsafe impl RefEncode for CGEvent {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateScrollWheelEvent(
        source: *const std::ffi::c_void,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        ...
    ) -> *mut CGEvent;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const std::ffi::c_void);
}

const WIDTH: f64 = 720.;
const HEIGHT: f64 = 540.;

struct Report {
    failures: usize,
}
impl Report {
    fn check(&mut self, name: &str, pass: bool, detail: String) {
        println!("{} {name}: {detail}", if pass { "PASS" } else { "FAIL" });
        self.failures += !pass as usize;
    }
}
/// Run the application's event loop for a while: window-server events (which
/// carry occlusion changes), the display cycle and the display link all run.
fn pump(seconds: f64) {
    let mtm = objc2_foundation::MainThreadMarker::new().expect("macOS main thread");
    let app = NSApplication::sharedApplication(mtm);
    let deadline = NSDate::dateWithTimeIntervalSinceNow(seconds);
    loop {
        let event = unsafe {
            app.nextEventMatchingMask_untilDate_inMode_dequeue(
                objc2_app_kit::NSEventMask::Any,
                Some(&deadline),
                objc2_foundation::NSDefaultRunLoopMode,
                true,
            )
        };
        match event {
            Some(event) => app.sendEvent(&event),
            None => break,
        }
    }
}
fn state(node: &str) -> Value {
    graph::debug_snapshot(node)
}
fn number(value: &Value, key: &str) -> f64 {
    value[key].as_f64().unwrap_or(f64::NAN)
}
/// Pump until the display link stops; returns seconds waited.
fn wait_idle(limit: f64) -> f64 {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < limit {
        pump(0.02);
        if state("")["running"] == false {
            break;
        }
    }
    start.elapsed().as_secs_f64()
}
unsafe fn mouse(view: &NSView, window: &NSWindow, kind: NSEventType, x: f64, y: f64) {
    let location = view.convertPoint_toView(NSPoint::new(x, y), None);
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind,
        location,
        NSEventModifierFlags::empty(),
        0.,
        window.windowNumber(),
        None,
        0,
        1,
        1.,
    )
    .expect("mouse event");
    let _: () = match kind {
        NSEventType::MouseMoved => msg_send![view, mouseMoved: &*event],
        NSEventType::LeftMouseDown => msg_send![view, mouseDown: &*event],
        NSEventType::LeftMouseDragged => msg_send![view, mouseDragged: &*event],
        _ => msg_send![view, mouseUp: &*event],
    };
}
/// A real scroll-wheel event. `pixels`: a trackpad-style precise delta;
/// otherwise wheel notches. Positive is "up", away from the user.
unsafe fn scroll(view: &NSView, amount: i32, pixels: bool) -> (bool, f64) {
    let raw =
        CGEventCreateScrollWheelEvent(std::ptr::null(), if pixels { 0 } else { 1 }, 1, amount);
    let event: Retained<NSEvent> = msg_send![objc2::class!(NSEvent), eventWithCGEvent: raw];
    CFRelease(raw.cast());
    let precise = event.hasPreciseScrollingDeltas();
    let delta = if precise {
        event.scrollingDeltaY()
    } else {
        event.deltaY()
    };
    let _: () = msg_send![view, scrollWheel: &*event];
    (precise, delta)
}
fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&out).expect("output directory");
    let mut report = Report { failures: 0 };
    unsafe {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("macOS main thread");
        let app = NSApplication::sharedApplication(mtm);
        // No Dock icon, no menu bar, no focus: the window just floats in a corner.
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        graph::allow_background_motion(true);
        let frame = NSRect::new(NSPoint::new(40., 60.), NSSize::new(WIDTH, HEIGHT));
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setTitle(&objc2_foundation::NSString::from_str(
            "Stology Local · 지도 자체 검증",
        ));
        window.setLevel(3); // floating, so it cannot be covered and count as occluded
        let map = graph::create(
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(WIDTH, HEIGHT)),
            graphs::biology(),
        );
        window.setContentView(Some(&map));
        window.orderFrontRegardless();
        let overlay: Retained<NSView> = map.subviews().lastObject().expect("overlay");
        let scale = window.backingScaleFactor();
        println!("window {WIDTH}x{HEIGHT} pt, backing scale {scale}");
        // The window server reports the window visible a moment after ordering it.
        let shown = Instant::now();
        while !window
            .occlusionState()
            .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
            && shown.elapsed().as_secs_f64() < 3.
        {
            pump(0.01);
        }
        println!(
            "INFO visible after {:.2}s, reduce motion {}",
            shown.elapsed().as_secs_f64(),
            objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
        );

        // 1. Entry animation runs on the display link and then stops by itself.
        pump(0.12);
        let early = state("chloroplast");
        report.check(
            "entry animation is running",
            early["running"] == true
                && number(&early, "ticks") >= 3.
                && number(&early, "scale") < 15.,
            format!(
                "ticks={} scale={:.2}",
                early["ticks"],
                number(&early, "scale")
            ),
        );
        let entry_started = Instant::now();
        let entry = 0.12 + wait_idle(3.);
        let settled = state("chloroplast");
        report.check(
            "entry animation finishes and the loop stops",
            settled["running"] == false
                && (number(&settled, "scale") - 15.).abs() < 1e-9
                && entry < 1.6,
            format!(
                "{entry:.2}s, ticks={} label alpha={:.2}",
                settled["ticks"],
                number(&settled, "label")
            ),
        );
        let rate = (number(&settled, "ticks") - number(&early, "ticks"))
            / entry_started.elapsed().as_secs_f64();
        println!("INFO display link rate ≈ {rate:.0} Hz");

        // 2. Nothing is drawn while nothing moves.
        pump(0.8);
        let idle = state("chloroplast");
        report.check(
            "no ticks or presented frames while idle (0.8 s)",
            idle["ticks"] == settled["ticks"] && idle["presented"] == settled["presented"],
            format!(
                "ticks {}→{}, presented {}→{}",
                settled["ticks"], idle["ticks"], settled["presented"], idle["presented"]
            ),
        );
        report.check(
            "frames reach the real drawable",
            number(&idle, "presented") > 10.,
            format!("presented={}", idle["presented"]),
        );

        // 3. Hover from a real mouseMoved: eased highlight, halo, then stop.
        let atp = state("atp");
        let (x, y) = (number(&atp, "x"), number(&atp, "y"));
        let other_before = number(&state("square"), "color");
        mouse(&overlay, &window, NSEventType::MouseMoved, x + 3., y - 2.);
        let hovered = state("atp");
        pump(0.05);
        let mid = state("atp");
        wait_idle(2.);
        let done = state("atp");
        report.check(
            "hover picks the concept under the pointer",
            hovered["hovered"] == "atp",
            format!("hovered={}", hovered["hovered"]),
        );
        report.check(
            "hover highlight eases in instead of jumping",
            number(&mid, "scale") > 15.
                && number(&mid, "scale") < 15.6 - 1e-6
                && (number(&done, "scale") - 15.6).abs() < 1e-9
                && number(&mid, "hover_halo") > 0.
                && number(&mid, "hover_halo") < 1.
                && number(&done, "hover_halo") == 1.,
            format!(
                "scale {:.3}→{:.3}, halo {:.2}→{:.2}",
                number(&mid, "scale"),
                number(&done, "scale"),
                number(&mid, "hover_halo"),
                number(&done, "hover_halo")
            ),
        );
        report.check(
            "unrelated concepts dim under hover",
            number(&state("square"), "color") > other_before + 0.05,
            format!(
                "linear red {:.3}→{:.3}",
                other_before,
                number(&state("square"), "color")
            ),
        );
        mouse(&overlay, &window, NSEventType::MouseMoved, 6., HEIGHT - 60.);
        wait_idle(2.);
        report.check(
            "leaving the concept restores it",
            state("atp")["hovered"].is_null()
                && (number(&state("atp"), "scale") - 15.).abs() < 1e-9,
            format!(
                "hovered={} scale={:.2}",
                state("atp")["hovered"],
                number(&state("atp"), "scale")
            ),
        );

        // 4. Scroll: wheel up zooms in, damped; a precise delta uses the web factor.
        let before = number(&state(""), "goal_distance");
        // Zoom in first so that neither limit hides the factor.
        graph::zoom(1.5);
        wait_idle(2.);
        let base = number(&state(""), "goal_distance");
        let (precise, delta) = scroll(&overlay, 20, true);
        let after = number(&state(""), "goal_distance");
        let immediate = number(&state(""), "distance");
        report.check(
            "precise scroll up zooms in by exp(0.0016·px)",
            precise && delta > 0. && ((base / after) - (delta * 0.0016).exp()).abs() < 1e-6,
            format!("precise={precise} delta={delta} distance {base:.1}→{after:.1} (overview {before:.1})"),
        );
        report.check(
            "zoom is damped, not instantaneous",
            (immediate - base).abs() < 1e-6,
            format!("distance right after the event {immediate:.1}, goal {after:.1}"),
        );
        wait_idle(2.);
        report.check(
            "zoom arrives and the loop stops",
            (number(&state(""), "distance") - after).abs() < 0.05 && state("")["running"] == false,
            format!("distance {:.2}", number(&state(""), "distance")),
        );
        let (notch_precise, notch) = scroll(&overlay, -1, false);
        let out_distance = number(&state(""), "goal_distance");
        report.check(
            "one wheel notch down zooms out by exp(0.0016·40)",
            !notch_precise
                && notch < 0.
                && ((out_distance / after) - (-notch * 40. * 0.0016).exp()).abs() < 1e-6,
            format!("precise={notch_precise} delta={notch} distance {after:.1}→{out_distance:.1}"),
        );
        for _ in 0..80 {
            scroll(&overlay, 10, false);
        }
        wait_idle(3.);
        let closest = state("chloroplast");
        report.check(
            "zoom stops 9.6 node radii away",
            (number(&closest, "distance") - 144.).abs() < 0.05,
            format!("distance {:.2}", number(&closest, "distance")),
        );
        graph::fit();
        wait_idle(3.);

        // 5. Drag a concept: it follows the pointer exactly, then eases home.
        let node = state("square");
        let (x, y) = (number(&node, "x"), number(&node, "y"));
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseDown,
            x + 2.,
            y + 1.,
        );
        for i in 1..=5 {
            mouse(
                &overlay,
                &window,
                NSEventType::LeftMouseDragged,
                x + 2. + 10. * i as f64,
                y + 1. + 4. * i as f64,
            );
            pump(0.02);
        }
        let held = state("square");
        report.check(
            "dragged concept stays under the pointer",
            (number(&held, "x") - x - 50.).abs() < 0.01
                && (number(&held, "y") - y - 20.).abs() < 0.01,
            format!(
                "moved ({:.2}, {:.2}) px",
                number(&held, "x") - x,
                number(&held, "y") - y
            ),
        );
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseUp,
            x + 52.,
            y + 21.,
        );
        pump(0.05);
        let returning = state("square");
        wait_idle(3.);
        let home = state("square");
        report.check(
            "released concept eases back and nothing is selected",
            number(&returning, "home") > 1.
                && number(&home, "home") < 1e-9
                && home["selected"].is_null(),
            format!(
                "offset {:.1} → {:.3}",
                number(&returning, "home"),
                number(&home, "home")
            ),
        );

        // 6. Orbit: dragging empty space turns the camera with damping.
        let yaw = number(&state(""), "yaw");
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseDown,
            30.,
            HEIGHT - 80.,
        );
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseDragged,
            130.,
            HEIGHT - 80.,
        );
        let turning = number(&state(""), "yaw");
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseUp,
            130.,
            HEIGHT - 80.,
        );
        wait_idle(3.);
        let turned = number(&state(""), "yaw");
        report.check(
            "orbit is damped and a drag is not a tap",
            (turning - yaw).abs() < 1e-9 && (turned - (yaw - 100. * 0.0042)).abs() < 1e-3,
            format!("yaw {yaw:.3} → {turned:.3}"),
        );

        // 7. Taps: empty space keeps the orbited camera; a concept focuses in 720 ms.
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseDown,
            30.,
            HEIGHT - 80.,
        );
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseUp,
            30.,
            HEIGHT - 80.,
        );
        wait_idle(1.);
        report.check(
            "empty tap with nothing selected changes nothing",
            (number(&state(""), "yaw") - turned).abs() < 1e-9,
            format!("yaw {:.3}", number(&state(""), "yaw")),
        );
        pump(0.35); // not a double tap
        let target = state("chloroplast");
        let (x, y) = (number(&target, "x"), number(&target, "y"));
        let overview_distance = number(&target, "distance");
        mouse(&overlay, &window, NSEventType::LeftMouseDown, x, y);
        mouse(&overlay, &window, NSEventType::LeftMouseUp, x, y);
        let started = Instant::now();
        let mut samples = Vec::new();
        let mut ran = true;
        while started.elapsed().as_secs_f64() < 0.60 {
            pump(0.05);
            let sample = state("chloroplast");
            ran &= sample["running"] == true;
            samples.push(number(&sample, "distance"));
        }
        let focus_time = started.elapsed().as_secs_f64() + wait_idle(3.);
        let focused = state("chloroplast");
        report.check(
            "tap selects and the 720 ms focus move is gradual",
            focused["selected"] == "chloroplast"
                && ran
                && samples.windows(2).all(|w| w[1] <= w[0] + 1e-9)
                && samples[3] < overview_distance - 1.
                && samples[3] > number(&focused, "distance") + 1.
                && (number(&focused, "x") - WIDTH / 2.).abs() < 0.5
                && focus_time < 1.6,
            format!(
                "distance {overview_distance:.0} → {:.0} → {:.0}, {focus_time:.2}s, halo {:.2}",
                samples[3],
                number(&focused, "distance"),
                number(&focused, "halo")
            ),
        );
        report.check(
            "selected concept sits at the centre of the free area",
            (number(&focused, "y") - (HEIGHT / 2. - 9.)).abs() < 0.5,
            format!(
                "screen ({:.1}, {:.1})",
                number(&focused, "x"),
                number(&focused, "y")
            ),
        );
        std::process::Command::new("screencapture")
            .args([
                "-x",
                "-o",
                &format!("-l{}", window.windowNumber()),
                &format!("{out}/live-focus.png"),
            ])
            .status()
            .ok();

        // 8. A chosen relation floats its neighbours; clearing it stops the loop.
        graph::filter("site");
        pump(1.0);
        let floating = state("photosynthesis");
        report.check(
            "chosen relation keeps its neighbour floating",
            floating["running"] == true && number(&floating, "floating") > 0.05,
            format!("offset {:.2} world units", number(&floating, "floating")),
        );
        graph::filter("");
        wait_idle(3.);
        let calm = state("photosynthesis");
        report.check(
            "clearing the relation stops the motion",
            calm["running"] == false && number(&calm, "floating") == 0.,
            format!("running={} offset={}", calm["running"], calm["floating"]),
        );

        // 9. Empty tap deselects and returns to the default overview.
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseDown,
            12.,
            HEIGHT - 70.,
        );
        mouse(
            &overlay,
            &window,
            NSEventType::LeftMouseUp,
            12.,
            HEIGHT - 70.,
        );
        wait_idle(3.);
        let reset = state("");
        report.check(
            "empty tap deselects and frames everything again",
            reset["selected"].is_null() && (number(&reset, "yaw") - 0.62).abs() < 1e-6,
            format!(
                "selected={} yaw={:.2} distance={:.0}",
                reset["selected"],
                number(&reset, "yaw"),
                number(&reset, "distance")
            ),
        );

        // 10. Frame cost with 300 concepts during a focus move.
        graph::update(graphs::dense());
        wait_idle(3.);
        let a = state("");
        graph::select("n000");
        let started = Instant::now();
        pump(0.7);
        let seconds = started.elapsed().as_secs_f64();
        let b = state("");
        let frames = number(&b, "draws") - number(&a, "draws");
        let ticks = number(&b, "ticks") - number(&a, "ticks");
        let cost = (number(&b, "work") - number(&a, "work")) / frames.max(1.) * 1000.;
        report.check(
            "300 concepts: every tick is drawn and a frame costs under 4 ms of CPU",
            frames >= ticks - 2. && ticks / seconds > rate * 0.9 && cost < 4.,
            format!(
                "{:.0} ticks/s, {:.0} frames/s, {cost:.2} ms per frame",
                ticks / seconds,
                frames / seconds
            ),
        );
        wait_idle(3.);
        std::process::Command::new("screencapture")
            .args([
                "-x",
                "-o",
                &format!("-l{}", window.windowNumber()),
                &format!("{out}/live-dense.png"),
            ])
            .status()
            .ok();

        // 11. Closing releases everything and stops the loop.
        window.setContentView(None);
        graph::dispose();
        report.check(
            "dispose stops the loop",
            state("")["running"] == false && !graph::focus_input(),
            "disposed".into(),
        );
        window.close();
        let _: Option<Retained<AnyObject>> = None;
    }
    println!("{} failure(s)", report.failures);
    std::process::exit(report.failures.min(1) as i32);
}
