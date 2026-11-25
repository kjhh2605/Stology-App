//! Driving native controls in the smoke examples: toolbar buttons by tag, and
//! synthetic mouse and key events sent straight to a view or the first responder.
use objc2::{class, msg_send, rc::Retained};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSString};

pub unsafe fn button(root: &NSView, tag: isize) -> Retained<NSButton> {
    let view = root
        .subviews()
        .iter()
        .find(|view| {
            let yes: bool = msg_send![&**view,isKindOfClass:class!(NSButton)];
            yes && {
                let value: isize = msg_send![&**view, tag];
                value == tag
            }
        })
        .expect("tool button");
    Retained::retain(&*view as *const NSView as *mut NSButton).unwrap()
}
pub unsafe fn choose(root: &NSView, tag: isize) {
    button(root, tag).performClick(None);
}
pub unsafe fn mouse(overlay: &NSView, window: &NSWindow, kind: NSEventType, location: NSPoint) {
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, location, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
    let _: () = match kind {
        NSEventType::LeftMouseDown => msg_send![overlay,mouseDown:&*event],
        NSEventType::LeftMouseDragged => msg_send![overlay,mouseDragged:&*event],
        _ => msg_send![overlay,mouseUp:&*event],
    };
}
pub unsafe fn key(window: &NSWindow, text: &str, keycode: u16) {
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown,NSPoint::new(0.,0.),NSEventModifierFlags::empty(),0.,window.windowNumber(),None,
        &NSString::from_str(text),&NSString::from_str(text),false,keycode).unwrap();
    let responder = window.firstResponder().expect("first responder");
    let _: () = msg_send![&*responder,keyDown:&*event];
}
