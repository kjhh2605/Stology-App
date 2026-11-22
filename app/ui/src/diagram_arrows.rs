//! Arrow behavior beyond a plain line: attaching ends to shapes, holding an
//! angle, the three kinds the arrow tool draws, and the arrow context menu.
use super::*;
use std::cell::Cell;
use stology_core::diagram_svg::{Head, Side};

pub(super) const ARROW_TOOL: usize = 4;
const ACCENT: &str = "#756ee1";
// How far outside a shape an arrow end still attaches, and how near the
// middle of a side it must be to stay pinned to that side.
const REACH: f64 = 8.;
const PIN: f64 = 8.;

thread_local! { static LAST: Cell<Direction> = const { Cell::new(Direction::One) }; }

/// Which ends of an arrow carry a head. The arrow tool draws one of these kinds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Direction {
    One,
    Both,
    Neither,
}
impl Direction {
    pub(super) const ALL: [Self; 3] = [Self::One, Self::Both, Self::Neither];
    pub(super) fn title(self) -> &'static str {
        match self {
            Self::One => "단방향 화살표",
            Self::Both => "양방향 화살표",
            Self::Neither => "화살촉 없는 선",
        }
    }
    fn symbol(self) -> &'static str {
        match self {
            Self::One => "arrow.up.right",
            Self::Both => "arrow.up.left.and.arrow.down.right",
            Self::Neither => "line.diagonal",
        }
    }
    pub(super) fn image(self) -> Option<Retained<NSImage>> {
        NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(self.symbol()),
            Some(&NSString::from_str(self.title())),
        )
    }
    /// The kind last chosen from the arrow tool since the app started.
    pub(super) fn last() -> Self {
        LAST.get()
    }
    fn of(e: &Element) -> Option<Self> {
        match (e.start_head != Head::None, e.end_head != Head::None) {
            (false, true) => Some(Self::One),
            (true, true) => Some(Self::Both),
            (false, false) => Some(Self::Neither),
            (true, false) => None,
        }
    }
    pub(super) fn apply(self, e: &mut Element) {
        // An arrow that already has a head keeps the shape of that head.
        let head = [e.end_head, e.start_head]
            .into_iter()
            .find(|head| *head != Head::None)
            .unwrap_or(Head::Open);
        e.end_head = if self == Self::Neither { Head::None } else { head };
        e.start_head = if self == Self::Both { head } else { Head::None };
    }
}

fn within(e: &Element, p: Point, slack: f64) -> bool {
    let c = e.center();
    let u = (p[0] - c[0]).abs() / ((e.b[0] - c[0]).abs() + slack);
    let v = (p[1] - c[1]).abs() / ((e.b[1] - c[1]).abs() + slack);
    match e.kind {
        Kind::Ellipse => u * u + v * v <= 1.,
        Kind::Diamond => u + v <= 1.,
        _ => u <= 1. && v <= 1.,
    }
}
impl Model {
    /// The shape an arrow end at `p` attaches to, and the side it pins to.
    pub(super) fn attach(&self, p: Point) -> Option<(usize, Option<Side>)> {
        self.document.0.iter().enumerate().rev().find_map(|(i, e)| {
            if !e.kind.shape() {
                return None;
            }
            let side = Side::ALL.into_iter().find(|&side| {
                let q = e.anchor(side);
                (p[0] - q[0]).hypot(p[1] - q[1]) <= PIN
            });
            (side.is_some() || within(e, p, REACH)).then_some((i, side))
        })
    }
    /// An arrow whose free end lies in the shape its other end is attached to.
    pub(super) fn loops(&self, i: usize) -> bool {
        let e = &self.document.0[i];
        let (id, free) = match (e.from, e.to) {
            (Some(id), None) => (id, e.b),
            (None, Some(id)) => (id, e.a),
            _ => return false,
        };
        self.document
            .0
            .iter()
            .any(|shape| shape.id == id && within(shape, free, REACH))
    }
    /// `p` moved onto the nearest 45° line through the arrow's other end.
    pub(super) fn constrained(&self, i: usize, start: bool, p: Point) -> Point {
        let e = &self.document.0[i];
        let (id, side, free) = if start {
            (e.to, e.to_side, e.b)
        } else {
            (e.from, e.from_side, e.a)
        };
        let shape = self.document.0.iter().find(|shape| Some(shape.id) == id);
        let origin = match (shape, side) {
            (Some(shape), Some(side)) => shape.anchor(side),
            (Some(shape), None) => shape.center(),
            _ => free,
        };
        let (dx, dy) = (p[0] - origin[0], p[1] - origin[1]);
        let step = std::f64::consts::FRAC_PI_4;
        let (sin, cos) = ((dy.atan2(dx) / step).round() * step).sin_cos();
        let length = dx * cos + dy * sin;
        [
            (origin[0] + length * cos).clamp(0., WIDTH),
            (origin[1] + length * sin).clamp(0., HEIGHT),
        ]
    }
    /// Picks the kind of arrow to draw, also for later canvases and the A key.
    fn choose_arrow(&mut self, direction: Direction) {
        self.command(ARROW_TOOL);
        self.direction = direction;
        LAST.set(direction);
    }
    /// Changes every selected arrow in one undo step.
    fn edit_arrows(&mut self, change: impl Fn(&mut Element)) {
        let old = self.document.clone();
        for &i in &self.selected {
            let e = &mut self.document.0[i];
            if e.kind == Kind::Arrow {
                change(e);
            }
        }
        self.remember(old);
    }
}

/// Marks the shape an arrow end will attach to and its four pinning points.
pub(super) unsafe fn highlight(e: &Element, pinned: Option<Side>) {
    let accent = color(ACCENT);
    let path = editor::diagram_paint::outline(e);
    path.setLineWidth(2.5);
    accent.colorWithAlphaComponent(0.45).setStroke();
    path.stroke();
    for side in Side::ALL {
        let [x, y] = e.anchor(side);
        let on = pinned == Some(side);
        let r = if on { 5. } else { 3.5 };
        let dot = NSBezierPath::bezierPathWithOvalInRect(rect(x - r, y - r, 2. * r, 2. * r));
        if on {
            accent.setFill();
        } else {
            NSColor::whiteColor().setFill();
        }
        dot.fill();
        dot.setLineWidth(1.5);
        accent.setStroke();
        dot.stroke();
    }
}

// Context menu tags: the kinds of `Direction::ALL` come first.
const SOLID: isize = 10;
const DASHED: isize = 11;
const STRAIGHT: isize = 12;
const ELBOW: isize = 13;

impl Surface {
    unsafe fn menu_item(&self, menu: &NSMenu, title: &str, action: Sel, tag: isize) -> Retained<NSMenuItem> {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm()),
            &NSString::from_str(title),
            Some(action),
            &NSString::new(),
        );
        item.setTarget(Some(self));
        item.setTag(tag);
        menu.addItem(&item);
        item
    }
    /// The three kinds of arrow, shown under the arrow tool button.
    pub(super) unsafe fn arrow_tool_menu(&self) -> Retained<NSMenu> {
        let m = self.ivars().model.borrow();
        let menu = NSMenu::new(mtm());
        menu.setAutoenablesItems(false);
        for (tag, direction) in Direction::ALL.into_iter().enumerate() {
            let item = self.menu_item(&menu, direction.title(), sel!(arrowKind:), tag as _);
            item.setImage(direction.image().as_deref());
            item.setState((direction == m.direction) as _);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm()));
        let keep = self.menu_item(&menu, "계속 그리기", sel!(arrowKeep:), 0);
        keep.setState((m.locked && m.tool == ARROW_TOOL) as _);
        menu
    }
    pub(super) fn choose_arrow_kind(&self, tag: isize) {
        if let Some(direction) = Direction::ALL.get(tag as usize) {
            self.ivars().model.borrow_mut().choose_arrow(*direction);
        }
        self.refresh();
    }
    pub(super) fn keep_arrow_tool(&self) {
        let mut m = self.ivars().model.borrow_mut();
        let keep = !(m.locked && m.tool == ARROW_TOOL);
        m.command(ARROW_TOOL);
        m.locked = keep;
        drop(m);
        self.refresh();
    }
    /// The context menu of the arrow at `p`, which becomes the selection
    /// unless it is already part of it. Other elements have no menu.
    pub(super) unsafe fn arrow_menu(&self, p: Point) -> Option<Retained<NSMenu>> {
        let mut m = self.ivars().model.borrow_mut();
        let i = m.hit(p, false)?;
        let arrow = m.document.0[i].clone();
        if arrow.kind != Kind::Arrow {
            return None;
        }
        if !m.selected.contains(&i) {
            m.selected = vec![i];
        }
        drop(m);
        self.refresh();
        let menu = NSMenu::new(mtm());
        menu.setAllowsContextMenuPlugIns(false);
        menu.setAutoenablesItems(false);
        let (line, line_tag) = if arrow.dashed {
            ("실선으로 전환", SOLID)
        } else {
            ("점선으로 전환", DASHED)
        };
        let (route, route_tag) = if arrow.elbow {
            ("직선으로 변환", STRAIGHT)
        } else {
            ("꺾인 선으로 변환", ELBOW)
        };
        self.menu_item(&menu, line, sel!(arrowEdit:), line_tag);
        self.menu_item(&menu, route, sel!(arrowEdit:), route_tag);
        menu.addItem(&NSMenuItem::separatorItem(mtm()));
        for (tag, direction) in Direction::ALL.into_iter().enumerate() {
            let item = self.menu_item(&menu, direction.title(), sel!(arrowEdit:), tag as _);
            item.setState((Direction::of(&arrow) == Some(direction)) as _);
        }
        Some(menu)
    }
    pub(super) fn edit_arrow(&self, tag: isize) {
        self.ivars().model.borrow_mut().edit_arrows(|e| match tag {
            SOLID | DASHED => e.dashed = tag == DASHED,
            STRAIGHT | ELBOW => e.elbow = tag == ELBOW,
            _ => {
                if let Some(direction) = Direction::ALL.get(tag as usize) {
                    direction.apply(e);
                }
            }
        });
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two boxes far enough apart that an arrow between them is unambiguous.
    fn model() -> Model {
        let boxes = [([40., 40.], [140., 100.]), ([300., 200.], [400., 260.])];
        let document = boxes
            .iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let mut e = Element::new(i as u32 + 1, Kind::Rect, *a);
                e.b = *b;
                e
            })
            .collect();
        Model {
            document: Diagram(document),
            tool: 0,
            selected: vec![],
            marquee: None,
            drag: None,
            undo: vec![],
            redo: vec![],
            direction: Direction::One,
            locked: false,
            snap: None,
        }
    }
    fn drag(m: &mut Model, from: Point, to: Point) {
        m.start(from);
        m.update(to, false);
        m.end();
    }
    fn close(p: Point, q: Point) -> bool {
        (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-6
    }

    #[test]
    fn ends_attach_inside_near_and_pinned_to_a_side() {
        let mut m = model();
        m.command(ARROW_TOOL);
        m.start([90., 70.]);
        m.update([350., 230.], false);
        assert_eq!(m.snap, Some((1, None)));
        m.end();
        let arrow = &m.document.0[2];
        assert_eq!((arrow.from, arrow.to), (Some(1), Some(2)));
        assert_eq!((arrow.from_side, arrow.to_side, m.snap), (None, None, None));

        // Starting and ending by the middle of a side pins both ends there.
        m.command(ARROW_TOOL);
        m.start([350., 201.]);
        m.update([92., 104.], false);
        assert_eq!(m.snap, Some((0, Some(Side::Bottom))));
        m.end();
        let arrow = &m.document.0[3];
        assert_eq!((arrow.from_side, arrow.to_side), (Some(Side::Top), Some(Side::Bottom)));
        assert_eq!(arrow.ends(&m.document.0), ([350., 200.], [90., 100.]));

        // Just outside the outline still attaches, without pinning.
        m.command(ARROW_TOOL);
        drag(&mut m, [500., 100.], [146., 50.]);
        let arrow = &m.document.0[4];
        assert_eq!((arrow.from, arrow.to, arrow.to_side), (None, Some(1), None));
        m.command(ARROW_TOOL);
        drag(&mut m, [500., 100.], [160., 50.]);
        assert_eq!(m.document.0[5].to, None);
        assert_eq!(m.document.validate().ok(), Some(()));
    }

    #[test]
    fn an_arrow_inside_one_shape_is_dropped() {
        let mut m = model();
        m.command(ARROW_TOOL);
        drag(&mut m, [50., 50.], [130., 90.]);
        assert!(m.document.0.len() == 2 && m.undo.is_empty());

        drag(&mut m, [90., 70.], [220., 70.]);
        let drawn = m.document.clone();
        assert_eq!((drawn.0[2].from, drawn.0[2].to, m.tool), (Some(1), None, 0));
        // Dropping the free end back into the attached shape restores the arrow.
        drag(&mut m, [220., 70.], [100., 70.]);
        assert!(m.document == drawn && m.undo.len() == 1);
        // Dropping it on the other shape attaches it.
        drag(&mut m, [220., 70.], [350., 230.]);
        assert!(m.document.0[2].to == Some(2) && m.undo.len() == 2);
    }

    #[test]
    fn attached_ends_follow_their_shapes_and_not_the_pointer() {
        let mut m = model();
        m.command(ARROW_TOOL);
        drag(&mut m, [90., 70.], [350., 230.]);
        let drawn = m.document.clone();
        let (a, b) = drawn.0[2].ends(&drawn.0);
        let middle = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
        drag(&mut m, middle, [middle[0] + 40., middle[1] + 40.]);
        assert!(m.selected == [2] && m.document == drawn && m.undo.len() == 1);

        drag(&mut m, [60., 60.], [80., 60.]);
        let (moved, _) = m.document.0[2].ends(&m.document.0);
        assert!(moved != a && m.document.0[2].from == Some(1));

        // A free end moves with the body while the attached end stays.
        let mut m = model();
        m.command(ARROW_TOOL);
        drag(&mut m, [90., 70.], [220., 70.]);
        drag(&mut m, [180., 70.], [180., 120.]);
        let arrow = &m.document.0[2];
        assert!(close(arrow.b, [220., 120.]) && close(arrow.a, [90., 70.]));
    }

    #[test]
    fn shift_holds_a_free_end_to_45_degree_steps() {
        let mut m = model();
        m.command(ARROW_TOOL);
        m.start([500., 300.]);
        m.update([600., 310.], true);
        assert!(close(m.document.0[2].b, [600., 300.]));
        m.update([590., 400.], true);
        assert!(close(m.document.0[2].b, [595., 395.]));
        m.update([600., 310.], false);
        assert!(close(m.document.0[2].b, [600., 310.]));
        m.end();

        // The angle is measured from the middle of an attached shape.
        m.command(ARROW_TOOL);
        m.start([70., 60.]);
        m.update([300., 80.], true);
        m.end();
        let arrow = &m.document.0[3];
        assert!(close(arrow.b, [300., 70.]));
        assert_eq!(arrow.ends(&m.document.0), ([140., 70.], [300., 70.]));
        // Attaching wins over the held angle.
        m.command(ARROW_TOOL);
        m.start([500., 300.]);
        m.update([390., 252.], true);
        m.end();
        assert_eq!(m.document.0[4].to, Some(2));
    }

    #[test]
    fn a_locked_tool_keeps_drawing_until_another_is_chosen() {
        let mut m = model();
        m.command(ARROW_TOOL);
        drag(&mut m, [500., 100.], [600., 100.]);
        assert_eq!(m.tool, 0);
        m.command(ARROW_TOOL);
        m.locked = true;
        drag(&mut m, [500., 120.], [600., 120.]);
        drag(&mut m, [500., 140.], [600., 140.]);
        assert!(m.tool == ARROW_TOOL && m.document.0.len() == 5);
        m.command(0);
        assert!(!m.locked);
    }

    #[test]
    fn the_chosen_kind_is_drawn_next_and_remembered() {
        let mut m = model();
        m.choose_arrow(Direction::Both);
        assert!(m.tool == ARROW_TOOL && m.undo.is_empty() && Direction::last() == Direction::Both);
        drag(&mut m, [500., 100.], [600., 100.]);
        let arrow = &m.document.0[2];
        assert_eq!((arrow.start_head, arrow.end_head), (Head::Open, Head::Open));
        // The A key keeps the kind chosen last.
        m.command(ARROW_TOOL);
        drag(&mut m, [500., 140.], [600., 140.]);
        assert_eq!(Direction::of(&m.document.0[3]), Some(Direction::Both));
        m.choose_arrow(Direction::Neither);
        drag(&mut m, [500., 180.], [600., 180.]);
        let line = &m.document.0[4];
        assert_eq!((line.start_head, line.end_head), (Head::None, Head::None));
        assert!(!line.dashed && !line.elbow);
        m.choose_arrow(Direction::One);
    }

    #[test]
    fn edits_reach_selected_arrows_only_in_one_undo_step() {
        let mut m = model();
        m.command(ARROW_TOOL);
        drag(&mut m, [90., 70.], [350., 230.]);
        m.command(ARROW_TOOL);
        drag(&mut m, [500., 100.], [600., 100.]);
        m.selected = vec![0, 2, 3];
        let before = m.document.clone();
        m.edit_arrows(|e| (e.dashed, e.elbow) = (true, true));
        assert!(m.document.0[2].dashed && m.document.0[3].elbow && m.undo.len() == 3);
        assert_eq!(m.document.0[0], before.0[0]);
        m.command(7);
        assert_eq!(m.document, before);
        // A filled head survives a change of direction, and returns as it was.
        m.document.0[3].end_head = Head::Filled;
        m.selected = vec![3];
        m.edit_arrows(|e| Direction::Both.apply(e));
        let arrow = &m.document.0[3];
        assert_eq!((arrow.start_head, arrow.end_head), (Head::Filled, Head::Filled));
        m.edit_arrows(|e| Direction::Neither.apply(e));
        assert_eq!(Direction::of(&m.document.0[3]), Some(Direction::Neither));
        m.edit_arrows(|e| Direction::One.apply(e));
        assert_eq!(m.document.0[3].end_head, Head::Open);
    }

}
