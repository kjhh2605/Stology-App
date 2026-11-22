//! Fixed, interactive overview of the entire diagram in document coordinates.
use super::*;
use objc2::{rc::Weak, AnyThread};
use std::cell::Cell;

const PAD: f64 = 8.;

pub(super) struct MapState {
    surface: Weak<Surface>,
    thumbnail: RefCell<Option<Retained<NSImage>>>,
    grab_offset: Cell<Option<NSPoint>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyDiagramMinimap"]
    #[thread_kind = MainThreadOnly]
    #[ivars = MapState]
    pub(super) struct Minimap;
    unsafe impl NSObjectProtocol for Minimap {}
    impl Minimap {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method(viewportChanged:))]
        fn viewport_changed(&self, _: &NSNotification) { self.setNeedsDisplay(true); }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) { unsafe { self.draw_map(); } }

        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) {
            self.ivars().grab_offset.set(None);
            let Some(surface) = self.ivars().surface.load() else { return };
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let area = self.map_area();
            if !NSPointInRect(p, area) { return; }
            let point = self.document_point(p);
            let visible = surface.visibleRect();
            let offset = if NSPointInRect(point, visible) {
                NSPoint::new(point.x - visible.mid().x, point.y - visible.mid().y)
            } else { NSPoint::ZERO };
            self.ivars().grab_offset.set(Some(offset));
            self.navigate(&surface, point, offset);
        }

        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) {
            if let (Some(surface), Some(offset)) = (self.ivars().surface.load(), self.ivars().grab_offset.get()) {
                let p = self.convertPoint_fromView(event.locationInWindow(), None);
                self.navigate(&surface, self.document_point(p), offset);
            }
        }

        #[unsafe(method(mouseUp:))]
        fn up(&self, _: &NSEvent) { self.ivars().grab_offset.set(None); }
    }
);

impl Minimap {
    pub(super) unsafe fn new(surface: &Surface, scroll: &NSScrollView) -> Retained<Self> {
        let allocated = Self::alloc(mtm()).set_ivars(MapState {
            surface: Weak::new(surface),
            thumbnail: Default::default(),
            grab_offset: Cell::new(None),
        });
        let map: Retained<Self> = msg_send![super(allocated), initWithFrame: NSRect::ZERO];
        map.setAccessibilityElement(true);
        map.setAccessibilityRole(Some(NSAccessibilityGroupRole));
        map.setAccessibilityLabel(Some(&NSString::from_str("다이어그램 미니맵")));
        map.setAccessibilityHelp(Some(&NSString::from_str(
            "미니맵을 클릭하거나 현재 화면을 끌어 이동",
        )));
        let clip = scroll.contentView();
        clip.setPostsBoundsChangedNotifications(true);
        NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &map,
            sel!(viewportChanged:),
            Some(NSViewBoundsDidChangeNotification),
            Some(&clip),
        );
        map
    }

    pub(super) fn layout(&self, anchor: NSPoint, palette_bottom: f64) {
        let available_height = palette_bottom - anchor.y;
        let map_width = (anchor.x * 0.24)
            .clamp(105., 175.)
            .min((available_height - 2. * PAD) * WIDTH / HEIGHT);
        let hidden = map_width < 78. || anchor.x < 150.;
        self.setHidden(hidden);
        if hidden {
            return;
        }
        let outer = map_width + 2. * PAD;
        let frame = rect(
            anchor.x - outer,
            anchor.y,
            outer,
            map_width * HEIGHT / WIDTH + 2. * PAD,
        );
        if self.frame() != frame {
            self.setFrame(frame);
            self.document_changed();
        }
    }

    pub(super) fn document_changed(&self) {
        self.ivars().thumbnail.borrow_mut().take();
        self.setNeedsDisplay(true);
    }

    fn map_area(&self) -> NSRect {
        NSInsetRect(self.bounds(), PAD, PAD)
    }

    fn document_point(&self, p: NSPoint) -> NSPoint {
        let area = self.map_area();
        let scale = area.size.width / WIDTH;
        NSPoint::new((p.x - area.origin.x) / scale, (p.y - area.origin.y) / scale)
    }

    fn navigate(&self, surface: &Surface, point: NSPoint, offset: NSPoint) {
        let Some(scroll) = surface.enclosingScrollView() else {
            return;
        };
        let clip = scroll.contentView();
        let center = clip.convertPoint_fromView(
            NSPoint::new(point.x - offset.x, point.y - offset.y),
            Some(surface),
        );
        let mut bounds = clip.bounds();
        bounds.origin = NSPoint::new(
            center.x - bounds.size.width / 2.,
            center.y - bounds.size.height / 2.,
        );
        clip.scrollToPoint(clip.constrainBoundsRect(bounds).origin);
        scroll.reflectScrolledClipView(&clip);
    }

    unsafe fn draw_map(&self) {
        let Some(surface) = self.ivars().surface.load() else {
            return;
        };
        let area = self.map_area();
        let scale = area.size.width / WIDTH;
        if scale <= 0. {
            return;
        }
        let panel = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            NSInsetRect(self.bounds(), 0.5, 0.5),
            8.,
            8.,
        );
        color("#ffffff").setFill();
        panel.fill();
        color("#cecdd8").setStroke();
        panel.stroke();

        color("#f4f4f8").setFill();
        NSBezierPath::fillRect(area);
        let mut cached = self.ivars().thumbnail.borrow_mut();
        let thumbnail = cached.get_or_insert_with(|| {
            let image = NSImage::initWithSize(NSImage::alloc(), area.size);
            image.lockFocusFlipped(true);
            let transform = NSAffineTransform::transform();
            transform.scaleBy(scale);
            transform.concat();
            editor::diagram_paint::paint_with_line_width(
                &surface.ivars().model.borrow().document,
                0.85 / scale,
            );
            image.unlockFocus();
            image
        });
        thumbnail.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
            area,
            NSRect::ZERO,
            NSCompositingOperation::SourceOver,
            1.,
            true,
            None,
        );
        let visible = NSIntersectionRect(surface.visibleRect(), surface.bounds());
        if !NSIsEmptyRect(visible) {
            let frame = rect(
                area.origin.x + visible.origin.x * scale,
                area.origin.y + visible.origin.y * scale,
                visible.size.width * scale,
                visible.size.height * scale,
            );
            let outline = NSBezierPath::bezierPathWithRect(NSInsetRect(frame, 0.8, 0.8));
            let accent = color("#6554db");
            accent.colorWithAlphaComponent(0.12).setFill();
            outline.fill();
            outline.setLineWidth(1.6);
            accent.setStroke();
            outline.stroke();
        }
    }
}
