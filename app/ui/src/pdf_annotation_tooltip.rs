//! One non-activating hover bubble for PDF tool buttons. Native tool tips stay
//! disabled so only this immediate, short explanation is displayed.
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, Sel},
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

const TAIL: f64 = 6.0;
thread_local! {
    static SHARED: RefCell<Weak<Bubble>> = RefCell::new(Weak::new());
}

define_class!(
    #[unsafe(super(NSPanel, NSWindow, NSResponder))]
    #[name = "StologyPDFToolBubblePanel"]
    #[thread_kind = MainThreadOnly]
    struct BubblePanel;
    impl BubblePanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool { false }
        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool { false }
    }
);

struct BubbleViewState {
    tail_x: Cell<f64>,
}
define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyPDFToolBubbleView"]
    #[thread_kind = MainThreadOnly]
    #[ivars = BubbleViewState]
    struct BubbleView;
    impl BubbleView {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _rect: NSRect) {
            let bounds = self.bounds();
            let body = NSRect::new(NSPoint::new(0.5, TAIL + 0.5),
                NSSize::new(bounds.size.width - 1.0, bounds.size.height - TAIL - 1.0));
            let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(body, 7.0, 7.0);
            NSColor::controlBackgroundColor().setFill();
            path.fill();
            NSColor::separatorColor().setStroke();
            path.setLineWidth(0.5);
            path.stroke();
            let x = self.ivars().tail_x.get();
            let tail = NSBezierPath::bezierPath();
            tail.moveToPoint(NSPoint::new(x - TAIL, TAIL + 1.0));
            tail.lineToPoint(NSPoint::new(x, 0.5));
            tail.lineToPoint(NSPoint::new(x + TAIL, TAIL + 1.0));
            NSColor::controlBackgroundColor().setFill();
            tail.fill();
            tail.setLineWidth(0.5);
            tail.stroke();
        }
    }
);

struct Bubble {
    panel: Retained<BubblePanel>,
    view: Retained<BubbleView>,
    label: Retained<NSTextField>,
    owner: Cell<usize>,
}
impl Bubble {
    unsafe fn new() -> Rc<Self> {
        let mtm = MainThreadMarker::new().unwrap();
        let panel: Retained<BubblePanel> = msg_send![super(BubblePanel::alloc(mtm).set_ivars(())),
            initWithContentRect:NSRect::ZERO,
            styleMask:NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing:NSBackingStoreType::Buffered, defer:false];
        panel.setReleasedWhenClosed(false);
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setIgnoresMouseEvents(true);
        panel.setHidesOnDeactivate(true);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::Transient | NSWindowCollectionBehavior::IgnoresCycle,
        );
        let allocated = BubbleView::alloc(mtm).set_ivars(BubbleViewState {
            tail_x: Cell::new(20.0),
        });
        let view: Retained<BubbleView> = msg_send![super(allocated), initWithFrame:NSRect::ZERO];
        let label = NSTextField::labelWithString(&NSString::new(), mtm);
        label.setFont(Some(&crate::font::regular(11.0)));
        label.setTextColor(Some(&NSColor::labelColor()));
        label.setAccessibilityLabel(Some(&NSString::from_str("PDF 도구 설명")));
        view.addSubview(&label);
        panel.setContentView(Some(&view));
        Rc::new(Self {
            panel,
            view,
            label,
            owner: Cell::new(0),
        })
    }
    unsafe fn show(&self, button: &ToolButton) {
        let Some(parent) = button.window() else {
            return;
        };
        if button.isHiddenOrHasHiddenAncestor() || !parent.isVisible() {
            return;
        }
        self.hide_all();
        self.label
            .setStringValue(&NSString::from_str(&button.ivars().text));
        self.label.sizeToFit();
        let label_size = self.label.frame().size;
        let size = NSSize::new(
            label_size.width.ceil() + 20.0,
            label_size.height.ceil() + 12.0 + TAIL,
        );
        let anchor = parent.convertRectToScreen(button.convertRect_toView(button.bounds(), None));
        let screen = parent
            .screen()
            .map(|s| s.visibleFrame())
            .unwrap_or(parent.frame());
        let (frame, tail_x) = bubble_geometry(anchor, size, screen);
        self.panel.setFrame_display(frame, false);
        self.view.setFrame(NSRect::new(NSPoint::ZERO, size));
        self.view.ivars().tail_x.set(tail_x);
        self.label
            .setFrame(NSRect::new(NSPoint::new(10.0, TAIL + 6.0), label_size));
        self.view.setNeedsDisplay(true);
        self.owner.set(button as *const ToolButton as usize);
        // Child ordering follows the document window without activating the
        // app, becoming key, or taking mouse/keyboard events from the editor.
        parent.addChildWindow_ordered(&self.panel, NSWindowOrderingMode::Above);
        self.panel.orderFront(None);
    }
    fn hide(&self, owner: usize) {
        if self.owner.get() == owner {
            self.hide_all();
        }
    }
    fn hide_all(&self) {
        self.owner.set(0);
        self.panel.orderOut(None);
        if let Some(parent) = self.panel.parentWindow() {
            parent.removeChildWindow(&self.panel);
        }
    }
}
impl Drop for Bubble {
    fn drop(&mut self) {
        self.hide_all();
    }
}

fn bubble_geometry(anchor: NSRect, size: NSSize, screen: NSRect) -> (NSRect, f64) {
    let center = anchor.origin.x + anchor.size.width / 2.0;
    let left = screen.origin.x + 4.0;
    let right = (screen.origin.x + screen.size.width - size.width - 4.0).max(left);
    let x = (center - size.width / 2.0).clamp(left, right);
    let y = anchor.origin.y + anchor.size.height + 4.0;
    let tail_x = (center - x).clamp(12.0, (size.width - 12.0).max(12.0));
    (NSRect::new(NSPoint::new(x, y), size), tail_x)
}

struct ToolButtonState {
    bubble: Rc<Bubble>,
    text: String,
    tracking: RefCell<Option<Retained<NSTrackingArea>>>,
}
define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologyPDFHoverToolButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ToolButtonState]
    struct ToolButton;
    impl ToolButton {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            if let Some(area) = self.ivars().tracking.borrow_mut().take() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(self), None);
            self.addTrackingArea(&area);
            *self.ivars().tracking.borrow_mut() = Some(area);
        } }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _event: &NSEvent) { unsafe { self.ivars().bubble.show(self); } }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { self.hide_bubble(); }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) { unsafe {
            self.hide_bubble();
            let _: () = msg_send![super(self), mouseDown:event];
        } }
        #[unsafe(method(performClick:))]
        fn click(&self, sender: Option<&AnyObject>) { unsafe {
            self.hide_bubble();
            let _: () = msg_send![super(self), performClick:sender];
        } }
        #[unsafe(method(viewWillMoveToWindow:))]
        fn moving_window(&self, window: Option<&NSWindow>) { unsafe {
            self.hide_bubble();
            let _: () = msg_send![super(self), viewWillMoveToWindow:window];
        } }
        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { unsafe {
            self.hide_bubble();
            let _: () = msg_send![super(self), viewDidHide];
        } }
    }
);
impl ToolButton {
    fn hide_bubble(&self) {
        self.ivars().bubble.hide(self as *const Self as usize);
    }
}

/// The caller configures the returned button's icon, tag, accessible name/help,
/// and frame exactly like a normal NSButton. It never receives a native tooltip.
pub unsafe fn button(target: &AnyObject, action: Sel, short_help: &str) -> Retained<NSButton> {
    let bubble = SHARED.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(bubble) = slot.upgrade() {
            return bubble;
        }
        let bubble = Bubble::new();
        *slot = Rc::downgrade(&bubble);
        bubble
    });
    let allocated =
        ToolButton::alloc(MainThreadMarker::new().unwrap()).set_ivars(ToolButtonState {
            bubble,
            text: short_help.to_owned(),
            tracking: RefCell::new(None),
        });
    let button: Retained<ToolButton> = msg_send![super(allocated), initWithFrame:NSRect::ZERO];
    button.setTitle(&NSString::new());
    button.setTarget(Some(target));
    button.setAction(Some(action));
    button.setRefusesFirstResponder(true);
    Retained::into_super(button)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bubble_sits_above_button_and_points_to_it() {
        let (frame, tail) = bubble_geometry(
            NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(32.0, 28.0)),
            NSSize::new(90.0, 33.0),
            NSRect::new(NSPoint::ZERO, NSSize::new(1000.0, 800.0)),
        );
        assert_eq!(frame.origin, NSPoint::new(71.0, 132.0));
        assert_eq!(tail, 45.0);
    }
    #[test]
    fn bubble_stays_on_screen_at_either_horizontal_edge() {
        let screen = NSRect::new(NSPoint::new(-1000.0, 0.0), NSSize::new(1000.0, 800.0));
        for x in [-1000.0, -32.0] {
            let (frame, tail) = bubble_geometry(
                NSRect::new(NSPoint::new(x, 100.0), NSSize::new(32.0, 28.0)),
                NSSize::new(160.0, 33.0),
                screen,
            );
            assert!(frame.origin.x >= -996.0 && frame.origin.x + frame.size.width <= -4.0);
            assert!((12.0..=148.0).contains(&tail));
        }
    }
}
