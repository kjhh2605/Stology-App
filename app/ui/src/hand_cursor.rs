//! Hit-testable cursor surface that forwards clicks to its native control.
use super::*;

define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyHandCursorRegion"]
    #[thread_kind = MainThreadOnly]
    struct HandCursorRegion;
    impl HandCursorRegion {
        // AppKit excludes a hit-test-transparent child from cursor routing.
        // Keep this view as the mouse target and let the native control track clicks.
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            if let Some(control) = self.superview() { control.mouseDown(event); }
        } }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, event: Option<&NSEvent>) -> bool { unsafe {
            self.superview().is_some_and(|control| control.acceptsFirstMouse(event))
        } }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            theme::track_bounds(self, NSTrackingAreaOptions::CursorUpdate | NSTrackingAreaOptions::ActiveInKeyWindow);
        } }

        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, _event: &NSEvent) { unsafe {
            if self.superview().is_some_and(|view| {
                let control: &NSControl = &*Retained::as_ptr(&view).cast();
                control.isEnabled()
            }) {
                NSCursor::pointingHandCursor().set();
            }
        } }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursors(&self) { unsafe {
            if self.superview().is_some_and(|view| {
                // install() only attaches this view directly to an NSControl.
                let control: &NSControl = &*Retained::as_ptr(&view).cast();
                control.isEnabled()
            }) {
                self.addCursorRect_cursor(self.bounds(), &NSCursor::pointingHandCursor());
            }
        } }
    }
);

/// Track the actual clickable bounds, including subsequent AppKit alert layout.
pub unsafe fn install(control: &NSControl) {
    let region: Retained<HandCursorRegion> = msg_send![HandCursorRegion::alloc(mtm()), initWithFrame: control.bounds()];
    region.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    control.addSubview(&region);
}
