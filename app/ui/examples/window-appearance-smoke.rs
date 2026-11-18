//! Headless AppKit assertions: no windows are shown and no focus is acquired.
#[path = "../src/window_appearance.rs"]
mod window_appearance;

use objc2::{msg_send, ClassType, MainThreadMarker, MainThreadOnly};
use objc2::rc::Retained;
use objc2::runtime::AnyClass;
use objc2_app_kit::*;
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};

fn main() { unsafe {
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(600., 400.));
    let ordinary = NSWindow::initWithContentRect_styleMask_backing_defer(
        NSWindow::alloc(mtm), frame, NSWindowStyleMask::Titled,
        NSBackingStoreType::Buffered, false);
    let fixed = NSWindow::initWithContentRect_styleMask_backing_defer(
        window_appearance::alloc(mtm), frame, NSWindowStyleMask::Titled,
        NSBackingStoreType::Buffered, false);
    ordinary.setReleasedWhenClosed(false);
    fixed.setReleasedWhenClosed(false);
    assert!(!app.isActive());
    assert!(!ordinary.isKeyWindow() && !fixed.isKeyWindow());
    assert!(!ordinary.isMainWindow() && !fixed.isMainWindow());
    assert!(app.keyWindow().is_none());
    assert_eq!(ordinary.canBecomeKeyWindow(), fixed.canBecomeKeyWindow());
    let regular_active: bool = msg_send![&ordinary, _hasActiveAppearance];
    let fixed_active: bool = msg_send![&fixed, _hasActiveAppearance];
    assert!(!regular_active);
    assert!(fixed_active);
    assert!(fixed.isKindOfClass(NSWindow::class()));
    let glass = AnyClass::get(c"NSGlassEffectView").expect("macOS 26 glass required for this smoke");
    let allocated: objc2::rc::Allocated<NSView> = msg_send![glass, alloc];
    let glass: Retained<NSView> = msg_send![allocated, initWithFrame:frame];
    fixed.contentView().unwrap().addSubview(&glass);
    fixed.contentView().unwrap().layoutSubtreeIfNeeded();
    assert!(glass.window().is_some());
    assert!(!fixed.isKeyWindow());
    println!("PASS inactive app; both windows non-key/non-main; ordinary appearance inactive; glass window appearance active; real NSGlassEffectView attached; input eligibility unchanged.");
} }
