//! Native drag target in the existing 12pt gutter; never replaces the editor.
use super::*;
use objc2::DefinedClass;

#[derive(Default)]
pub struct DividerState {
    side: usize,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
    hover: Cell<bool>,
    drag: Cell<Option<(f64, f64, f64)>>,
    previous_focus: RefCell<Option<Retained<NSResponder>>>,
}
define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologySidebarResizeDivider"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DividerState]
    pub struct Divider;
    impl Divider {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursors(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::resizeLeftRightCursor());
        }
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            if let Some(area) = self.ivars().tracking.borrow_mut().take() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(self), None);
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
        } }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _event: &NSEvent) { self.ivars().hover.set(true); self.setNeedsDisplay(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { self.ivars().hover.set(false); self.setNeedsDisplay(true); }
        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { self.ivars().hover.set(false); self.setNeedsDisplay(true); }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            if self.ivars().hover.get() || self.ivars().drag.get().is_some() {
                NSColor::colorWithSRGBRed_green_blue_alpha(0.45, 0.46, 0.51, 0.55).setFill();
                let b = self.bounds();
                NSBezierPath::fillRect(rect((b.size.width - 2.) / 2., 12., 2., (b.size.height - 24.).max(0.)));
            }
        }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool { true }
        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) { self.cancel(); }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            if event.keyCode() == 53 { self.cancel(); }
        }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) { unsafe {
            let allowed = STATE.with(|state| {
                let Ok(state) = state.try_borrow() else { return false; };
                let Some(s) = state.as_ref() else { return false; };
                s.sidebar_open[self.ivars().side] && s.sidebar_motion.is_none()
                    && !s.left_resizer.is_dragging() && !s.right_resizer.is_dragging()
                    && !s.document_tree.as_ref().is_some_and(|t| t.renaming_workspace().is_some())
            });
            if !allowed { return; }
            let Some(window) = self.window() else { return; };
            let previous = window.firstResponder();
            if let Some(responder) = &previous {
                if responder.respondsToSelector(sel!(hasMarkedText)) {
                    let marked: bool = msg_send![responder, hasMarkedText];
                    if marked { return; }
                }
            }
            if !window.makeFirstResponder(Some(self)) { return; }
            *self.ivars().previous_focus.borrow_mut() = previous;
            STATE.with(|state| {
                let Ok(state) = state.try_borrow() else { return; };
                let Some(s) = state.as_ref() else { return; };
                self.ivars().drag.set(Some((event.locationInWindow().x,
                    if self.ivars().side == 0 { s.left_shell.frame().size.width } else { s.right_shell.frame().size.width },
                    if self.ivars().side == 0 { s.left_width } else { s.right_width })));
            });
            NSCursor::resizeLeftRightCursor().set();
            self.setNeedsDisplay(true);
        } }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) { self.resize(event); }
        #[unsafe(method(mouseUp:))]
        fn up(&self, event: &NSEvent) { unsafe {
            if self.ivars().drag.get().is_none() { return; }
            self.resize(event);
            self.ivars().drag.set(None);
            self.ivars().hover.set(false);
            self.setNeedsDisplay(true);
            let collapsed = STATE.with(|state| {
                let Ok(mut state) = state.try_borrow_mut() else { return false; };
                let Some(s) = state.as_mut() else { return false; };
                let side = self.ivars().side;
                let pending = if side == 0 {s.left_drag_pending} else {s.right_drag_pending};
                if pending { s.toggle_sidebar(side); }
                if s.sidebar_open[side] {
                    if side == 0 { s.left_drag_pending = false; s.left_drag_offset = 0.; }
                    else { s.right_drag_pending = false; s.right_drag_offset = 0.; }
                    s.layout_frames(s.root.bounds().size);
                    if side == 0 { s.refresh_left(); }
                    s.refresh_right();
                    if s.preview.is_none() && s.map.is_none() {
                        editor::style_with_base(&s.text, s.document.as_ref().and_then(document_base));
                    }
                }
                !s.sidebar_open[side]
            });
            self.restore_focus(collapsed);
        } }

    }
);
impl Divider {
    pub fn is_dragging(&self) -> bool { self.ivars().drag.get().is_some() }
    pub fn cancel(&self) { unsafe {
        let Some((_, _, original_width)) = self.ivars().drag.take() else { return; };
        self.ivars().hover.set(false);
        STATE.with(|state| {
            let Ok(mut state) = state.try_borrow_mut() else { return; };
            if let Some(s) = state.as_mut() {
                if self.ivars().side == 0 {
                    s.left_width = original_width; s.left_drag_offset = 0.; s.left_drag_pending = false;
                } else {
                    s.right_width = original_width; s.right_drag_offset = 0.; s.right_drag_pending = false;
                }
                s.layout_frames(s.root.bounds().size);
                s.refresh_right();
            }
        });
        self.setNeedsDisplay(true);
        self.restore_focus(false);
    } }
    unsafe fn restore_focus(&self, collapsed: bool) {
        let previous = self.ivars().previous_focus.borrow_mut().take();
        let Some(window) = self.window() else { return; };
        if collapsed {
            let text = STATE.with(|s| s.borrow().as_ref().map(|s| s.text.clone()));
            if let Some(text) = text { window.makeFirstResponder(Some(&text)); }
        } else if let Some(previous) = previous {
            window.makeFirstResponder(Some(&previous));
        }
    }
    pub unsafe fn new(side: usize) -> Retained<Self> {
        let allocated = Self::alloc(mtm()).set_ivars(DividerState {side, ..Default::default()});
        let divider: Retained<Self> = msg_send![super(allocated),
            initWithFrame:rect(0., 0., sidebar_layout::RESIZE_WIDTH, 100.)];
        let _: () = msg_send![&divider, setAccessibilityElement:true];
        let _: () = msg_send![&divider, setAccessibilityRole:&*NSString::from_str("AXSplitter")];
        let _: () = msg_send![&divider, setAccessibilityLabel:&*NSString::from_str(if side == 0 {"왼쪽 사이드바 너비 조절"} else {"오른쪽 사이드바 너비 조절"})];
        divider
    }
    fn resize(&self, event: &NSEvent) { unsafe {
        let Some((start, width, _)) = self.ivars().drag.get() else { return; };
        STATE.with(|state| {
            let Ok(mut state) = state.try_borrow_mut() else { return; };
            if let Some(s) = state.as_mut() {
                let left = self.ivars().side == 0;
                let requested = width + (event.locationInWindow().x - start) * if left {1.} else {-1.};
                if left {
                    let geometry = sidebar_layout::left_drag_geometry(s.root.bounds().size.width,
                        s.sidebar_openness, requested, s.right_width);
                    s.left_width = geometry.width;
                    s.left_drag_offset = geometry.offset;
                    s.left_drag_pending = geometry.hide;
                } else {
                    let geometry = sidebar_layout::drag_geometry(s.root.bounds().size.width,
                        s.sidebar_openness[0], requested);
                    s.right_width = sidebar_layout::right_width_with_left(s.root.bounds().size.width,
                        s.sidebar_openness[0], s.left_shell.frame().size.width, geometry.width);
                    s.right_drag_offset = geometry.offset;
                    s.right_drag_pending = geometry.hide;
                }
                s.layout_frames(s.root.bounds().size);
            }
        });
        NSCursor::resizeLeftRightCursor().set();
    } }
}

/// Flexible full-width rows follow the panel while small icons retain their size.
/// Configure existing views before resizing, preserving tree selection and IME.
pub unsafe fn prepare_content_width(parent: &NSView) {
    for child in parent.subviews().iter() {
        if child.frame().size.width < parent.bounds().size.width / 2. { continue; }
        child.setAutoresizingMask(child.autoresizingMask() | NSAutoresizingMaskOptions::ViewWidthSizable);
        if child.isKindOfClass(NSScrollView::class()) {
            let document: Option<Retained<NSView>> = msg_send![&child, documentView];
            if let Some(document) = document {
                document.setAutoresizingMask(document.autoresizingMask() | NSAutoresizingMaskOptions::ViewWidthSizable);
                if !document.isKindOfClass(NSTableView::class()) {
                    prepare_content_width(&document);
                }
            }
        } else if !child.isKindOfClass(NSControl::class()) {
            prepare_content_width(&child);
        }
    }
}

