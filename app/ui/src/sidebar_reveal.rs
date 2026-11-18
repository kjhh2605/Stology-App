//! Full-height, normally invisible edge controls for opening and closing sidebars.
use super::*;
use objc2::DefinedClass;

#[derive(Default)]
pub struct RevealState {
    hover: Cell<bool>,
    side: Cell<usize>,
    open: Cell<bool>,
    /// Hover cue slide: 0 = hidden past the outer edge, 1 = fully in.
    slide: Cell<f64>,
    slide_from: Cell<f64>,
    slide_started: Cell<Option<Instant>>,
    slide_timer: RefCell<Option<Retained<NSTimer>>>,
}

/// Duration of the hover cue sliding in from (or back to) the outer edge.
pub const SLIDE_SECONDS: f64 = 0.16;

/// Ease-out cubic so the cue decelerates as it settles at the edge.
pub fn slide_ease(t: f64) -> f64 { let t = t.clamp(0., 1.); 1. - (1. - t).powi(3) }

/// Horizontal offset of the cue; it hides past the outer (window-side) edge of its side.
pub fn slide_offset(left: bool, width: f64, progress: f64) -> f64 {
    let hidden = (1. - slide_ease(progress)) * width;
    if left { -hidden } else { hidden }
}

define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologySidebarRevealButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = RevealState]
    pub struct RevealButton;
    impl RevealButton {
        #[unsafe(method(setSidebarOpen:))]
        fn set_open(&self, open: bool) { unsafe {
            self.ivars().open.set(open);
            // A mode change snaps the cue; only hover changes slide.
            self.stop_slide(if self.ivars().hover.get() { 1. } else { 0. });
            let title = NSString::from_str(&format!("{} 사이드바 {}",
                if self.ivars().side.get() == 0 { "왼쪽" } else { "오른쪽" },
                if open { "접기" } else { "펼치기" }));
            let _: () = msg_send![self, setAccessibilityLabel:&*title];
            self.setNeedsDisplay();
        } }
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> { unsafe {
            let local = self.convertPoint_fromView(point, self.superview().as_deref());
            let hit = self.hit_rect();
            if local.x < hit.origin.x || local.x >= hit.origin.x + hit.size.width { None }
            else { msg_send![super(self), hitTest:point] }
        } }

        #[unsafe(method(resetCursorRects))]
        fn reset_cursors(&self) {
            self.addCursorRect_cursor(self.hit_rect(), &NSCursor::pointingHandCursor());
        }
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            for area in self.trackingAreas().iter() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.hit_rect(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways, Some(self), None);
            self.addTrackingArea(&area);
        } }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _event: &NSEvent) { self.set_hover(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { self.set_hover(false); }
        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { self.ivars().hover.set(false); self.stop_slide(0.); }
        #[unsafe(method(revealSlideTick:))]
        fn slide_tick(&self, _timer: &NSTimer) {
            let Some(started) = self.ivars().slide_started.get() else { return; };
            let t = (started.elapsed().as_secs_f64() / SLIDE_SECONDS).min(1.);
            let from = self.ivars().slide_from.get();
            let to = if self.ivars().hover.get() { 1. } else { 0. };
            if t >= 1. { self.stop_slide(to); return; }
            self.ivars().slide.set(from + (to - from) * t);
            self.setNeedsDisplay();
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) { unsafe {
            if self.ivars().slide.get() <= 0. { return; }
            let b = self.bounds();
            let left = self.ivars().side.get() == 0;
            // Both 접기 and 펼치기 cues slide in from the outer edge on hover.
            let dx = slide_offset(left, b.size.width, self.ivars().slide.get());
            let edge = NSColor::colorWithSRGBRed_green_blue_alpha(0.63, 0.59, 0.88, 0.30);
            let clear = NSColor::colorWithSRGBRed_green_blue_alpha(0.63, 0.59, 0.88, 0.);
            if let Some(gradient) = NSGradient::initWithStartingColor_endingColor(
                NSGradient::alloc(), &edge, &clear) {
                gradient.drawInRect_angle(rect(b.origin.x + dx, b.origin.y, b.size.width, b.size.height), if left { 0. } else { 180. });
            }
            let ink = NSColor::colorWithSRGBRed_green_blue_alpha(0.39, 0.34, 0.68, 1.);
            ink.setStroke();
            // Keep the label inside the drawing surface, independently of the narrow hit strip.
            let x = if left { 20. } else { b.size.width - 20. } + dx;
            let y = b.size.height / 2. + if self.isFlipped() { -8. } else { 8. };
            let direction = if left != self.ivars().open.get() { 1. } else { -1. };
            let arrow = NSBezierPath::bezierPath();
            arrow.setLineWidth(1.8);
            arrow.moveToPoint(NSPoint::new(x - direction * 6., y));
            arrow.lineToPoint(NSPoint::new(x + direction * 6., y));
            arrow.moveToPoint(NSPoint::new(x, y + 6.));
            arrow.lineToPoint(NSPoint::new(x + direction * 6., y));
            arrow.lineToPoint(NSPoint::new(x, y - 6.));
            arrow.stroke();
            let label = NSMutableAttributedString::initWithString(
                NSMutableAttributedString::alloc(), &NSString::from_str(if self.ivars().open.get() { "접기" } else { "펼치기" }));
            let range = NSRange::new(0, label.length());
            label.addAttribute_value_range(NSForegroundColorAttributeName, &ink, range);
            label.addAttribute_value_range(NSFontAttributeName,
                &NSFont::systemFontOfSize_weight(10., NSFontWeightMedium), range);
            let size: NSSize = msg_send![&label, size];
            let label_y = if self.isFlipped() { y + 14. } else { y - 26. };
            let _: () = msg_send![&label, drawAtPoint:NSPoint::new(x - size.width / 2., label_y)];
        } }
    }
);

impl RevealButton {
    fn set_hover(&self, hover: bool) {
        self.ivars().hover.set(hover);
        // Restart from the current position so a quick exit/re-enter reverses smoothly.
        self.ivars().slide_from.set(self.ivars().slide.get());
        self.ivars().slide_started.set(Some(Instant::now()));
        if self.ivars().slide_timer.borrow().is_none() { unsafe {
            let timer: Retained<NSTimer> = msg_send![NSTimer::class(),
                timerWithTimeInterval:(1. / 60.) target:self
                selector:sel!(revealSlideTick:) userInfo:std::ptr::null::<AnyObject>() repeats:true];
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            *self.ivars().slide_timer.borrow_mut() = Some(timer);
        } }
        self.setNeedsDisplay();
    }
    /// Ends any slide and releases the timer (which retains this view).
    fn stop_slide(&self, progress: f64) {
        if let Some(timer) = self.ivars().slide_timer.borrow_mut().take() { timer.invalidate(); }
        self.ivars().slide_started.set(None);
        self.ivars().slide.set(progress);
        self.setNeedsDisplay();
    }
    fn hit_rect(&self) -> NSRect {
        let b = self.bounds();
        let width = if self.ivars().open.get() { 14. } else { sidebar_layout::HANDLE_WIDTH };
        rect(if self.ivars().side.get() == 0 { 0. } else { b.size.width - width },
            0., width, b.size.height)
    }
}

pub unsafe fn set_open(button: &NSButton, open: bool) {
    let _: () = msg_send![button, setSidebarOpen:open];
}

pub unsafe fn new(side: usize) -> Retained<NSButton> {
    let allocated = RevealButton::alloc(mtm()).set_ivars(RevealState::default());
    let button: Retained<RevealButton> = msg_send![super(allocated), initWithFrame:rect(0., 0., sidebar_layout::GLOW_WIDTH, 100.)];
    button.ivars().side.set(side);
    button.setTitle(&NSString::from_str(""));
    button.setBordered(false);
    button.setRefusesFirstResponder(true);
    button.setFocusRingType(NSFocusRingType::None);
    button.setButtonType(NSButtonType::MomentaryPushIn);
    set_open(&button, true);
    button.into_super()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cue_slides_in_from_its_own_outer_edge() {
        assert_eq!(slide_offset(true, 56., 0.), -56.);
        assert_eq!(slide_offset(false, 56., 0.), 56.);
        assert_eq!(slide_offset(true, 56., 1.), 0.);
        assert_eq!(slide_offset(false, 56., 1.), 0.);
        // Ease-out: more than half the distance is covered at the midpoint.
        assert!(slide_offset(true, 56., 0.5).abs() < 28.);
        assert_eq!(slide_ease(2.), 1.);
        assert_eq!(slide_ease(-1.), 0.);
    }
}
