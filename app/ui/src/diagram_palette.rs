//! Color controls shared by the three diagram styling targets.
use super::*;
use objc2::AnyThread;
use std::cell::Cell;

const COLORS: [&str; 32] = [
    "#FFFFFF", "#FBE4E8", "#FBE8DB", "#FFF1C2", "#DDF3E4", "#DCF2F1", "#E0EAFF", "#E7E5FF",
    "#C8C7CF", "#EFA5B4", "#F0B185", "#F2D57C", "#9ED4AF", "#8DCDC8", "#A6BFF1", "#BDAAE9",
    "#75747F", "#D6455D", "#CA7138", "#B39429", "#1F8A5B", "#258680", "#4479C4", "#8462BD",
    "#1E1E2A", "#882C47", "#894A2A", "#786023", "#285F45", "#205B5E", "#2D4D80", "#4F46E5",
];
const FAMILIES: [&str; 8] = [
    "무채색",
    "빨강",
    "주황",
    "노랑",
    "초록",
    "청록",
    "파랑",
    "보라",
];
const TONES: [&str; 4] = ["옅은", "연한", "선명한", "짙은"];
thread_local! { static RECENTS: RefCell<Vec<String>> = RefCell::default(); }

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Stroke,
    Fill,
    Text,
}
impl Target {
    const ALL: [Self; 3] = [Self::Stroke, Self::Fill, Self::Text];
    fn title(self) -> &'static str {
        match self {
            Self::Stroke => "선 색",
            Self::Fill => "채우기",
            Self::Text => "텍스트 색",
        }
    }
    fn supports(self, e: &Element) -> bool {
        match self {
            Self::Stroke => e.kind != Kind::Text,
            Self::Fill => e.kind.shape(),
            Self::Text => e.kind != Kind::Pen,
        }
    }
    fn value(self, e: &Element) -> &str {
        match self {
            Self::Stroke => &e.ink,
            Self::Fill => &e.fill,
            Self::Text => e.label_color(),
        }
    }
    fn set(self, e: &mut Element, value: &str) {
        if self.value(e).eq_ignore_ascii_case(value) {
            return;
        }
        match self {
            Self::Stroke => {
                if e.text_color.is_none() {
                    e.text_color = Some(e.ink.clone());
                }
                e.ink = value.into();
            }
            Self::Fill => e.fill = value.into(),
            Self::Text => e.text_color = Some(value.into()),
        }
    }
}
impl Model {
    // None means inapplicable, while the empty string indicates mixed colors.
    fn selected_color(&self, target: Target) -> Option<&str> {
        let mut values = self
            .selected
            .iter()
            .map(|&i| &self.document.0[i])
            .filter(|e| target.supports(e))
            .map(|e| target.value(e));
        let first = values.next()?;
        Some(if values.all(|v| v.eq_ignore_ascii_case(first)) {
            first
        } else {
            ""
        })
    }
    fn apply_color(&mut self, target: Target, value: &str) {
        let old = self.document.clone();
        for &i in &self.selected {
            let e = &mut self.document.0[i];
            if target.supports(e) {
                target.set(e, value);
            }
        }
        self.remember(old);
    }
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}
// The app's body ink. Lower alphas give secondary text and hairlines.
fn ink(alpha: f64) -> Retained<NSColor> {
    rgba(0.13, 0.14, 0.24, alpha)
}
fn describe(value: &str) -> String {
    match value {
        "" => "여러 색".into(),
        "none" => "없음".into(),
        _ => match COLORS.iter().position(|c| c.eq_ignore_ascii_case(value)) {
            Some(i) => format!("{} {} · {}", TONES[i / 8], FAMILIES[i % 8], COLORS[i]),
            None => value.to_uppercase(),
        },
    }
}
fn circle(center: NSPoint, radius: f64) -> Retained<NSBezierPath> {
    NSBezierPath::bezierPathWithOvalInRect(rect(
        center.x - radius,
        center.y - radius,
        radius * 2.,
        radius * 2.,
    ))
}
fn fill_color(value: &str) -> Retained<NSColor> {
    match value {
        "" => rgba(0.93, 0.93, 0.95, 1.),
        "none" => NSColor::whiteColor(),
        _ => color(value),
    }
}
// A dash marks mixed colors and a red slash marks no fill.
fn mark(center: NSPoint, radius: f64, value: &str, flipped: bool) {
    let line = NSBezierPath::bezierPath();
    match value {
        "" => {
            ink(0.5).setStroke();
            line.moveToPoint(NSPoint::new(center.x - 3., center.y));
            line.lineToPoint(NSPoint::new(center.x + 3., center.y));
        }
        "none" => {
            let rise = if flipped { -radius } else { radius };
            color("#D6455D").setStroke();
            line.moveToPoint(NSPoint::new(center.x - radius, center.y - rise));
            line.lineToPoint(NSPoint::new(center.x + radius, center.y + rise));
        }
        _ => return,
    }
    line.setLineWidth(1.6);
    line.setLineCapStyle(NSLineCapStyle::Round);
    NSGraphicsContext::saveGraphicsState_class();
    circle(center, radius).addClip();
    line.stroke();
    NSGraphicsContext::restoreGraphicsState_class();
}
fn dot(center: NSPoint, radius: f64, value: &str, flipped: bool) {
    fill_color(value).setFill();
    circle(center, radius).fill();
    mark(center, radius, value, flipped);
    ink(0.16).setStroke();
    circle(center, radius - 0.5).stroke();
}
// Each target has its own glyph: a ring, a disc and a letter above a color bar.
fn chip(target: Target, value: &str) -> Retained<NSImage> {
    let value = value.to_owned();
    let block = block2::RcBlock::new(move |_: NSRect| {
        let center = NSPoint::new(13., 13.);
        match target {
            Target::Fill => dot(center, 10., &value, false),
            Target::Stroke => {
                let ring = circle(center, 7.75);
                ring.setLineWidth(4.5);
                fill_color(&value).setStroke();
                ring.stroke();
                mark(center, 5.5, &value, false);
                ink(0.16).setStroke();
                circle(center, 9.5).stroke();
                circle(center, 6.).stroke();
            }
            Target::Text => {
                let attrs = NSDictionary::from_slices(
                    unsafe { &[NSFontAttributeName, NSForegroundColorAttributeName] },
                    &[&*font::weighted(14., 0.4) as &AnyObject, &*ink(0.92)],
                );
                let letter = NSString::from_str("A");
                let size = unsafe { letter.sizeWithAttributes(Some(&attrs)) };
                let origin = NSPoint::new(13. - size.width / 2., 15.5 - size.height / 2.);
                unsafe { letter.drawAtPoint_withAttributes(origin, Some(&attrs)) };
                let bar = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(4.5, 1.5, 17., 5.),
                    2.5,
                    2.5,
                );
                fill_color(&value).setFill();
                bar.fill();
                ink(0.16).setStroke();
                bar.stroke();
            }
        }
        objc2::runtime::Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(26., 26.), false, &block)
}
unsafe fn rail_style(b: &NSButton, title: &str, active: bool) {
    b.setWantsLayer(true);
    let layer: Retained<AnyObject> = msg_send![b, layer];
    let wash = rgba(0.42, 0.38, 0.98, if active { 0.11 } else { 0. });
    let _: () = msg_send![&layer, setBackgroundColor: &*wash.CGColor()];
    let _: () = msg_send![&layer, setCornerRadius: 10.0f64];
    let text = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(title),
    );
    let range = NSRange::new(0, text.length());
    let (tint, weight) = if active {
        (rgba(0.26, 0.22, 0.78, 1.), 0.25)
    } else {
        (ink(0.72), 0.)
    };
    let centered = NSMutableParagraphStyle::new();
    centered.setAlignment(NSTextAlignment::Center);
    text.addAttribute_value_range(NSForegroundColorAttributeName, &tint, range);
    text.addAttribute_value_range(NSFontAttributeName, &font::weighted(10., weight), range);
    text.addAttribute_value_range(NSParagraphStyleAttributeName, &centered, range);
    b.setAttributedTitle(&text);
}
fn label(root: &NSView, value: &str, frame: NSRect, size: f64, weight: f64, alpha: f64) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(value), mtm());
    l.setFont(Some(&font::weighted(size, weight)));
    l.setTextColor(Some(&ink(alpha)));
    l.setFrame(frame);
    root.addSubview(&l);
    l
}

struct SwatchState {
    value: RefCell<String>,
    selected: Cell<bool>,
    hover: Cell<bool>,
}
define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologyDiagramSwatch"]
    #[thread_kind = MainThreadOnly]
    #[ivars = SwatchState]
    struct Swatch;
    impl Swatch {
        #[unsafe(method(wantsUpdateLayer))]
        fn updates_layer(&self) -> bool { false }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            for area in self.trackingAreas().iter() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect, Some(self), None);
            self.addTrackingArea(&area);
        } }

        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: &NSEvent) { self.set_hover(true); }

        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: &NSEvent) { self.set_hover(false); }

        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { self.ivars().hover.set(false); }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            let state = self.ivars();
            let center = self.bounds().mid();
            let selected = state.selected.get();
            let radius = if selected {
                8.
            } else if self.isHighlighted() {
                9.
            } else if state.hover.get() {
                11.5
            } else {
                10.
            };
            dot(center, radius, &state.value.borrow(), true);
            if selected {
                let ring = circle(center, 11.);
                ring.setLineWidth(2.);
                color("#4F46E5").setStroke();
                ring.stroke();
            }
        }
    }
);
impl Swatch {
    unsafe fn new(surface: &Surface, tag: usize, frame: NSRect) -> Retained<Self> {
        let allocated = Self::alloc(mtm()).set_ivars(SwatchState {
            value: Default::default(),
            selected: Cell::new(false),
            hover: Cell::new(false),
        });
        let b: Retained<Self> = msg_send![super(allocated), initWithFrame: frame];
        b.setTitle(&NSString::new());
        b.setBordered(false);
        b.setTarget(Some(surface));
        b.setAction(Some(sel!(pickColor:)));
        b.setTag(tag as _);
        b
    }
    // The header names the color under the pointer, so no delayed tool tip is used.
    fn set_hover(&self, hover: bool) {
        self.ivars().hover.set(hover);
        self.setNeedsDisplay();
        if let Some(target) = self.target() {
            let _: () = unsafe { msg_send![&*target, hoverColor: self] };
        }
    }
}

const PANEL_WIDTH: f64 = 264.;
const PANEL_HEIGHT: f64 = 222.;
const TIP: f64 = 9.;
const SHADOW: f64 = 32.;
const DIVIDER_Y: f64 = 62.;

// The body and its pointer are one outline, so the border never crosses the pointer.
fn panel_shape(target: Target, inset: f64) -> Retained<NSBezierPath> {
    let (x0, y0) = (inset + 0.5, inset + 0.5);
    let (x1, y1) = (inset + PANEL_WIDTH - 0.5, inset + PANEL_HEIGHT - 0.5);
    let y = inset + PANEL_HEIGHT - (target as usize + 1) as f64 * 48. + 23.;
    let radius = 14.;
    let at = NSPoint::new;
    let path = NSBezierPath::bezierPath();
    path.moveToPoint(at(x0 + radius, y0));
    path.appendBezierPathWithArcFromPoint_toPoint_radius(at(x1, y0), at(x1, y0 + radius), radius);
    path.lineToPoint(at(x1, y - TIP));
    path.appendBezierPathWithArcFromPoint_toPoint_radius(at(x1 + TIP, y), at(x1, y + TIP), 2.5);
    path.lineToPoint(at(x1, y + TIP));
    path.appendBezierPathWithArcFromPoint_toPoint_radius(at(x1, y1), at(x1 - radius, y1), radius);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(at(x0, y1), at(x0, y1 - radius), radius);
    path.appendBezierPathWithArcFromPoint_toPoint_radius(at(x0, y0), at(x0 + radius, y0), radius);
    path.closePath();
    path
}
// The blur behind the panel is limited to the same outline as the tint.
fn panel_mask(target: Target) -> Retained<NSImage> {
    let draw = block2::RcBlock::new(move |_: NSRect| {
        NSColor::blackColor().setFill();
        panel_shape(target, 0.).fill();
        objc2::runtime::Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(PANEL_WIDTH + TIP, PANEL_HEIGHT),
        false,
        &draw,
    )
}
fn panel_background(target: Target) -> Retained<NSImage> {
    let size = NSSize::new(PANEL_WIDTH + TIP + 2. * SHADOW, PANEL_HEIGHT + 2. * SHADOW);
    let draw = block2::RcBlock::new(move |_: NSRect| {
        let shape = panel_shape(target, SHADOW);
        // Keep the shadow outside the outline so the glass itself stays clear.
        NSGraphicsContext::saveGraphicsState_class();
        let outside = NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::ZERO, size));
        outside.appendBezierPath(&shape);
        outside.setWindingRule(NSWindingRule::EvenOdd);
        outside.addClip();
        for (alpha, blur, drop) in [(0.17, 22., 9.), (0.10, 3., 1.)] {
            let shadow = NSShadow::new();
            shadow.setShadowColor(Some(&rgba(0.15, 0.18, 0.36, alpha)));
            shadow.setShadowBlurRadius(blur);
            shadow.setShadowOffset(NSSize::new(0., -drop));
            shadow.set();
            NSColor::whiteColor().setFill();
            shape.fill();
        }
        NSGraphicsContext::restoreGraphicsState_class();
        NSColor::colorWithWhite_alpha(1., 0.58).setFill();
        shape.fill();
        rgba(0.70, 0.64, 0.95, 0.05).setFill();
        shape.fill();
        rgba(0.36, 0.37, 0.60, 0.26).setStroke();
        shape.stroke();
        rgba(0.36, 0.36, 0.84, 0.14).setFill();
        NSBezierPath::fillRect(rect(SHADOW + 16., SHADOW + DIVIDER_Y, PANEL_WIDTH - 32., 1.));
        objc2::runtime::Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(size, false, &draw)
}

pub(super) struct Palette {
    panel: Retained<NSView>,
    glass: Retained<NSVisualEffectView>,
    backdrop: Retained<NSImageView>,
    shapes: [(Retained<NSImage>, Retained<NSImage>); 3],
    target: Cell<Target>,
    rail: Vec<Retained<NSButton>>,
    swatches: Vec<Retained<Swatch>>,
    title: Retained<NSTextField>,
    caption: Retained<NSTextField>,
    current: RefCell<String>,
    empty_recent: Retained<NSTextField>,
}
impl Palette {
    pub(super) unsafe fn new(surface: &Surface, root: &NSView) -> Self {
        let panel = view(rect(0., 0., PANEL_WIDTH + TIP, PANEL_HEIGHT));
        panel.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
        let glass = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm()), panel.bounds());
        glass.setMaterial(NSVisualEffectMaterial::Popover);
        glass.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
        glass.setState(NSVisualEffectState::Active);
        panel.addSubview(&glass);
        let backdrop = NSImageView::initWithFrame(
            NSImageView::alloc(mtm()),
            NSInsetRect(panel.bounds(), -SHADOW, -SHADOW),
        );
        panel.addSubview(&backdrop);
        panel.setHidden(true);
        root.addSubview(&panel);
        let mut rail = Vec::new();
        for (i, target) in Target::ALL.iter().enumerate() {
            let b = NSButton::buttonWithTitle_target_action(
                &NSString::from_str(target.title()),
                Some(surface),
                Some(sel!(openColors:)),
                mtm(),
            );
            b.setTag((20 + i) as _);
            b.setBordered(false);
            b.setImagePosition(NSCellImagePosition::ImageAbove);
            rail_style(&b, target.title(), false);
            root.addSubview(&b);
            rail.push(b);
        }
        let title = label(&panel, "", rect(16., 193., 90., 18.), 12.5, 0.25, 1.);
        let caption = label(&panel, "", rect(98., 194., 150., 16.), 11., 0., 0.62);
        caption.setAlignment(NSTextAlignment::Right);
        caption.setLineBreakMode(NSLineBreakMode::ByTruncatingHead);
        let mut swatches = Vec::new();
        for i in 0..40 {
            let y = if i < 32 {
                160. - (i / 8) as f64 * 30.
            } else {
                10.
            };
            let b = Swatch::new(surface, i, rect(14. + (i % 8) as f64 * 30., y, 26., 26.));
            panel.addSubview(&b);
            swatches.push(b);
        }
        label(&panel, "최근 색상", rect(16., 39., 120., 15.), 11., 0., 0.62);
        let empty_recent = label(
            &panel,
            "선택한 색이 여기에 표시됩니다",
            rect(16., 15., 232., 15.),
            11.,
            0.,
            0.42,
        );
        Self {
            panel,
            target: Cell::new(Target::Stroke),
            rail,
            swatches,
            title,
            caption,
            current: Default::default(),
            empty_recent,
            shapes: Target::ALL.map(|t| (panel_mask(t), panel_background(t))),
            glass,
            backdrop,
        }
    }
    pub(super) fn layout(&self, x: f64, top: f64, width: f64) -> f64 {
        for (i, b) in self.rail.iter().enumerate() {
            b.setFrame(rect(x + 3., top - (i + 1) as f64 * 48., width - 6., 46.));
        }
        self.panel
            .setFrameOrigin(NSPoint::new(x - PANEL_WIDTH - TIP - 2., top - PANEL_HEIGHT));
        top - 3. * 48. - 8.
    }
    pub(super) fn refresh(&self, m: &Model) {
        for (i, t) in Target::ALL.iter().enumerate() {
            let value = m.selected_color(*t);
            let b = &self.rail[i];
            b.setHidden(value.is_none());

            if let Some(v) = value {
                b.setImage(Some(&chip(*t, v)));
                let name = format!("{} · {}", t.title(), describe(v));
                b.setAccessibilityLabel(Some(&NSString::from_str(&name)));
                b.setToolTip(Some(&NSString::from_str(&name)));
            }
        }
        if !self.panel.isHidden() {
            if m.selected_color(self.target.get()).is_some() {
                self.sync(m);
            } else {
                self.close();
            }
        }
    }
    pub(super) fn open(&self, button: &NSButton, m: &Model) {
        let target = Target::ALL[(button.tag() - 20) as usize];
        if m.selected_color(target).is_none() {
            return;
        }
        let same = self.target.get() == target;
        if !self.panel.isHidden() && same {
            self.close();
            return;
        }
        self.target.set(target);
        self.sync(m);
        self.panel.setHidden(false);
        self.mark_rail();
    }
    pub(super) fn close(&self) {
        if self.panel.isHidden() {
            return;
        }
        self.panel.setHidden(true);
        self.mark_rail();
    }
    // The header shows the color under the pointer, or else the current one.
    pub(super) fn hover(&self) {
        let hovered = self
            .swatches
            .iter()
            .find(|b| !b.isHidden() && b.ivars().hover.get())
            .map(|b| describe(&b.ivars().value.borrow()));
        let text = hovered.unwrap_or_else(|| self.current.borrow().clone());
        self.caption.setStringValue(&NSString::from_str(&text));
    }
    fn mark_rail(&self) {
        let open = !self.panel.isHidden();
        for (b, t) in self.rail.iter().zip(Target::ALL) {
            unsafe { rail_style(b, t.title(), open && self.target.get() == t) };
        }
    }
    fn sync(&self, m: &Model) {
        let target = self.target.get();
        let current = m.selected_color(target).unwrap_or("");
        let (mask, background) = &self.shapes[target as usize];
        self.glass.setMaskImage(Some(mask));
        self.backdrop.setImage(Some(background));
        self.title.setStringValue(&NSString::from_str(target.title()));
        *self.current.borrow_mut() = describe(current);
        RECENTS.with_borrow(|recent| {
            self.empty_recent.setHidden(!recent.is_empty());
            for (i, b) in self.swatches.iter().enumerate() {
                let (v, name) = if i == 0 && target == Target::Fill {
                    ("none", "채우기 없음".into())
                } else if i < COLORS.len() {
                    (COLORS[i], format!("{} {}", TONES[i / 8], FAMILIES[i % 8]))
                } else if let Some(v) = recent.get(i - COLORS.len()) {
                    (v.as_str(), "최근 색상".into())
                } else {
                    b.setHidden(true);
                    continue;
                };
                b.setHidden(false);
                let selected = current.eq_ignore_ascii_case(v);
                *b.ivars().value.borrow_mut() = v.into();
                b.ivars().selected.set(selected);
                b.setState(selected as _);
                b.setNeedsDisplay();
                let description = NSString::from_str(&if v == "none" {
                    name
                } else {
                    format!("{name} {v}")
                });
                b.setAccessibilityLabel(Some(&description));
            }
        });
        self.hover();
    }
}
impl Surface {
    pub(super) fn palette(&self) -> &Palette {
        self.ivars().palette.get().unwrap()
    }
    pub(super) fn pick_color(&self, b: &NSButton) {
        let i = b.tag() as usize;
        let value = match i {
            0 if self.palette().target.get() == Target::Fill => "none".into(),
            0..32 => COLORS[i].into(),
            32..40 => match RECENTS.with_borrow(|r| r.get(i - 32).cloned()) {
                Some(v) => v,
                None => return,
            },
            _ => return,
        };
        self.apply_palette_color(&value);
    }
    fn apply_palette_color(&self, value: &str) {
        let target = self.palette().target.get();
        let mut m = self.ivars().model.borrow_mut();
        if m.selected_color(target).is_none() {
            return;
        }
        m.apply_color(target, value);
        drop(m);
        if value != "none" {
            RECENTS.with_borrow_mut(|r| {
                r.retain(|v| !v.eq_ignore_ascii_case(value));
                r.insert(0, value.into());
                r.truncate(8);
            });
        }
        self.refresh();
    }
}
