//! Offscreen hardware validation; opens no window and does not operate user data.
//! cargo run --release --manifest-path local-app/ui/Cargo.toml --example graph3d-smoke
#[path = "../src/font.rs"]
mod font;
#[path = "../src/graph.rs"]
mod graph;
use objc2::{msg_send, MainThreadOnly};
use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
use objc2_foundation::{NSPoint, NSRect, NSSize};
fn main() {
    unsafe {
        let mtm = objc2_foundation::MainThreadMarker::new().expect("macOS main thread");
        let _ = objc2_app_kit::NSApplication::sharedApplication(mtm);
        let result = graph::metal_smoke().expect("actual Metal 3D depth and clipping validation");
        println!("{result:?}");
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 600.));
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        // The window is never ordered on screen and cannot steal user focus.
        let map = graph::create(
            frame,
            serde_json::json!({"concepts":[],"assertions":[],"relations":[]}),
        );
        window.setContentView(Some(&map));
        assert!(
            graph::focus_input(),
            "graph overlay must accept shortcuts without canvas click"
        );
        let responder = window.firstResponder().expect("graph first responder");
        let overlay = map.subviews().lastObject().expect("interactive overlay");
        let actual_overlay: bool = msg_send![&*responder,isEqual:&*overlay];
        assert!(
            actual_overlay,
            "plain wrapper/NSWindow must not receive graph shortcuts"
        );
        window.setContentView(None);
        graph::dispose();
        assert!(!graph::focus_input());
        println!("Graph first responder: actual overlay; disposed focus: false");
    }
}
