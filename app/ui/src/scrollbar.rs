//! Slim native scrollers: keep the gutter and drag target while centering the thumb.
use objc2::{define_class, msg_send, rc::Retained, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSObjectProtocol, NSRect};

const THUMB_WIDTH: f64 = 11. * 0.75;

define_class!(
    #[unsafe(super(NSScroller, NSControl, NSView, NSResponder))]
    #[name = "StologyThinScroller"]
    #[thread_kind = MainThreadOnly]
    struct ThinScroller;

    impl ThinScroller {
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool { false }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) { self.paint_thumb(); }

        #[unsafe(method(drawKnob))]
        fn draw_knob(&self) { self.paint_thumb(); }

        #[unsafe(method(drawKnobSlotInRect:highlight:))]
        fn draw_slot(&self, _rect: NSRect, _highlight: bool) {}
    }
);

impl ThinScroller {
    fn paint_thumb(&self) {
        paint_thumb(self);
    }
}

fn paint_thumb(scroller: &NSScroller) {
        // AppKit clears this view's transparent backing layer before drawing.
        if !scroller.isEnabled() || scroller.knobProportion() >= 1. { return; }
        let mut knob = scroller.rectForPart(NSScrollerPart::Knob);
        if knob.size.height <= 0. { return; }
        // Center within AppKit's knob rect so both margins match. Staying inside
        // that rect also keeps partial redraws from leaving a trailing edge.
        knob.origin.x += (knob.size.width - THUMB_WIDTH) / 2.;
        knob.size.width = THUMB_WIDTH;
        NSColor::secondaryLabelColor().setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            knob, THUMB_WIDTH / 2., THUMB_WIDTH / 2.).fill();
}

define_class!(
    #[unsafe(super(NSScroller, NSControl, NSView, NSResponder))]
    #[name = "StologyPdfTrackScroller"]
    #[thread_kind = MainThreadOnly]
    struct PdfTrackScroller;

    impl PdfTrackScroller {
        #[unsafe(method(isOpaque))]
        fn is_opaque(&self) -> bool { true }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) { self.paint(); }

        #[unsafe(method(drawKnob))]
        fn draw_knob(&self) { self.paint(); }

        #[unsafe(method(drawKnobSlotInRect:highlight:))]
        fn draw_slot(&self, _rect: NSRect, _highlight: bool) { self.paint(); }
    }
);

impl PdfTrackScroller {
    fn paint(&self) {
        let bounds = self.bounds();
        NSColor::whiteColor().setFill();
        NSBezierPath::fillRect(bounds);
        if !self.isEnabled() || self.knobProportion() >= 1. { return; }
        let mut track = bounds;
        track.origin.x += 3.;
        track.origin.y += 5.;
        track.size.width = (track.size.width - 6.).max(0.);
        track.size.height = (track.size.height - 10.).max(0.);
        NSColor::colorWithSRGBRed_green_blue_alpha(0.87, 0.875, 0.89, 1.).setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            track, track.size.width / 2., track.size.width / 2.).fill();
        paint_thumb(self);
    }
}

pub unsafe fn style(scroll: &NSScrollView) {
    let mtm = MainThreadMarker::new().expect("scroller requires main thread");
    // A reserved gutter keeps the margins stable while scrolling.
    scroll.setScrollerStyle(NSScrollerStyle::Legacy);
    let scroller: Retained<ThinScroller> =
        msg_send![ThinScroller::alloc(mtm), initWithFrame:scroll.bounds()];
    scroller.setWantsLayer(true);
    scroll.setVerticalScroller(Some(&scroller));
}

/// PDFKit owns its scroll view, so find it within the PDF view hierarchy.
pub unsafe fn style_descendants(view: &NSView) {
    for child in view.subviews().iter() {
        if child.isKindOfClass(NSScrollView::class()) {
            let scroll = &*((&*child as *const NSView).cast::<NSScrollView>());
            let mtm = MainThreadMarker::new().expect("scroller requires main thread");
            scroll.setScrollerStyle(NSScrollerStyle::Legacy);
            let scroller: Retained<PdfTrackScroller> =
                msg_send![PdfTrackScroller::alloc(mtm), initWithFrame:scroll.bounds()];
            scroller.setWantsLayer(true);
            scroll.setVerticalScroller(Some(&scroller));
        } else {
            style_descendants(&child);
        }
    }
}
