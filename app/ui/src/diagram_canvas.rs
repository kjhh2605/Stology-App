//! Native note diagram editor. A pointer gesture contributes one undo step.
use super::*;
use editor::diagram_paint::{color, element_bounds, paint, region};
use objc2::DefinedClass;
use stology_core::diagram_svg::{Diagram, Element, Kind, Point, HEIGHT, WIDTH};

#[path = "diagram_minimap.rs"]
mod minimap;
use minimap::Minimap;

#[path = "diagram_palette.rs"]
mod diagram_palette;
use diagram_palette::Palette;

#[path = "diagram_arrows.rs"]
mod diagram_arrows;
use diagram_arrows::{Direction, ARROW_TOOL};
use stology_core::diagram_svg::Side;

const AREA_SELECT: usize = 18;
const TOOLS: [(usize, &str, &str); 11] = [
    (0, "선택 (V)", "cursorarrow"),
    (AREA_SELECT, "영역 선택 (M)", "rectangle.dashed"),
    (1, "사각형 (R)", "rectangle"),
    (2, "타원 (O)", "oval"),
    (3, "마름모 (D)", "diamond"),
    (4, "화살표 (A)", "arrow.up.right"),
    (5, "펜 (P)", "pencil.tip"),
    (6, "글자 (T)", "textformat"),
    (7, "실행 취소 (⌘Z)", "arrow.uturn.backward"),
    (8, "다시 실행 (⇧⌘Z)", "arrow.uturn.forward"),
    (9, "삭제 (⌫)", "trash"),
];
const KINDS: [Kind; 6] = [
    Kind::Rect,
    Kind::Ellipse,
    Kind::Diamond,
    Kind::Arrow,
    Kind::Pen,
    Kind::Text,
];
#[derive(Clone, Copy)]
enum Motion {
    Create,
    Move { min: Point, max: Point },
    Resize,
    Endpoint(bool),
}
struct Gesture {
    motion: Motion,
    at: Point,
    before: Diagram,
}
struct Model {
    document: Diagram,
    tool: usize,
    selected: Vec<usize>,
    marquee: Option<(Point, Point)>,
    drag: Option<Gesture>,
    undo: Vec<Diagram>,
    redo: Vec<Diagram>,
    // The kind of arrow drawn next.
    direction: Direction,
    // A drawing tool chosen by double-click stays active after each drawing.
    locked: bool,
    // The shape, and its pinned side, that the moving arrow end would attach to.
    snap: Option<(usize, Option<Side>)>,
}
impl Model {
    fn remember(&mut self, old: Diagram) {
        if old == self.document {
            return;
        }
        if self.undo.len() >= 64 {
            self.undo.remove(0);
        }
        self.undo.push(old);
        self.redo.clear();
    }
    fn bounds(&self, i: usize) -> (Point, Point) {
        let e = &self.document.0[i];
        if e.kind == Kind::Text {
            let width = e
                .text
                .chars()
                .map(|c| if c.is_ascii() { 0.62 } else { 1. })
                .sum::<f64>()
                * e.font_size;
            return (
                [e.a[0], e.a[1] - e.font_size * 0.65],
                [e.a[0] + width.max(14.), e.a[1] + e.font_size * 0.65],
            );
        }
        e.corners(&self.document.0)
    }
    fn hit(&self, p: Point, shapes: bool) -> Option<usize> {
        self.document.0.iter().enumerate().rev().find_map(|(i, e)| {
            if shapes && !e.kind.shape() {
                return None;
            }
            let (a, b) = self.bounds(i);
            let in_box = (0..2).all(|k| p[k] >= a[k] - 4. && p[k] <= b[k] + 4.);
            if !in_box {
                return None;
            }
            let hit = match e.kind {
                Kind::Arrow => e
                    .route(&self.document.0)
                    .windows(2)
                    .any(|s| distance(p, s[0], s[1]) <= 7.),
                Kind::Pen => e
                    .points
                    .windows(2)
                    .any(|pair| distance(p, pair[0], pair[1]) <= 7.),
                Kind::Ellipse | Kind::Diamond => {
                    let c = e.center();
                    let u = (p[0] - c[0]).abs() / ((b[0] - a[0]) / 2. + 3.);
                    let v = (p[1] - c[1]).abs() / ((b[1] - a[1]) / 2. + 3.);
                    if e.kind == Kind::Ellipse {
                        u * u + v * v <= 1.
                    } else {
                        u + v <= 1.
                    }
                }
                _ => true,
            };
            hit.then_some(i)
        })
    }
    fn start(&mut self, p: Point) {
        let selecting = self.tool == 0 || self.tool == AREA_SELECT;
        let hit = selecting.then(|| self.hit(p, false)).flatten();
        let on_selection = hit.is_some_and(|i| self.selected.contains(&i));
        if self.tool == AREA_SELECT && !on_selection {
            self.selected.clear();
            self.marquee = Some((p, p));
            return;
        }
        let before = self.document.clone();
        let motion = if selecting {
            let handle = self
                .single_selection()
                .filter(|_| self.tool == 0)
                .and_then(|i| {
                    let e = &self.document.0[i];
                    if e.kind == Kind::Arrow {
                        let (a, b) = e.ends(&self.document.0);
                        [a, b]
                            .iter()
                            .position(|&q| near(p, q))
                            .map(|n| Motion::Endpoint(n == 0))
                    } else {
                        near(p, self.bounds(i).1).then_some(Motion::Resize)
                    }
                });
            if let Some(handle) = handle {
                handle
            } else {
                if !on_selection {
                    self.selected = hit.into_iter().collect();
                }
                let mut min = [0_f64; 2];
                let mut max = [WIDTH, HEIGHT];
                for &i in &self.selected {
                    let (a, b) = self.document.0[i].corners(&self.document.0);
                    for k in 0..2 {
                        min[k] = min[k].max(p[k] - a[k]);
                        max[k] = max[k].min(p[k] + [WIDTH, HEIGHT][k] - b[k]);
                    }
                }
                Motion::Move { min, max }
            }
        } else {
            if self.document.0.len() >= 2000 {
                return;
            }
            let kind = KINDS[self.tool - 1];
            let id = self
                .document
                .0
                .iter()
                .map(|e| e.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1);
            let mut e = Element::new(id, kind, p);
            if kind == Kind::Arrow {
                self.direction.apply(&mut e);
                if let Some((i, side)) = self.attach(p) {
                    let shape = &self.document.0[i];
                    (e.from, e.from_side) = (Some(shape.id), side);
                    if let Some(side) = side {
                        e.a = shape.anchor(side);
                    }
                }
            }
            if kind == Kind::Pen {
                e.points.push(p)
            }
            self.selected = vec![self.document.0.len()];
            self.document.0.push(e);
            Motion::Create
        };
        if !self.selected.is_empty() {
            self.drag = Some(Gesture {
                motion,
                at: p,
                before,
            });
        }
    }
    // `constrain` holds a free arrow end to 45° steps around the other end.
    fn update(&mut self, mut p: Point, constrain: bool) {
        if let Some((_, end)) = &mut self.marquee {
            *end = p;
            return;
        }
        let Some(g) = &self.drag else { return };
        if let Motion::Move { min, max } = g.motion {
            let dx = p[0].clamp(min[0], max[0]) - g.at[0];
            let dy = p[1].clamp(min[1], max[1]) - g.at[1];
            let shift = |q: Point| [(q[0] + dx).clamp(0., WIDTH), (q[1] + dy).clamp(0., HEIGHT)];
            for &i in &self.selected {
                let original = &g.before.0[i];
                let e = &mut self.document.0[i];
                // An attached arrow end stays with its shape.
                if original.from.is_none() {
                    e.a = shift(original.a);
                }
                if original.to.is_none() {
                    e.b = shift(original.b);
                }
                for (q, old) in e.points.iter_mut().zip(&original.points) {
                    *q = shift(*old);
                }
            }
            return;
        }
        let Some(i) = self.single_selection() else {
            return;
        };
        // Some(true) while the start of an arrow moves, Some(false) for its end.
        let moving = match g.motion {
            Motion::Create if self.document.0[i].kind == Kind::Arrow => Some(false),
            Motion::Endpoint(start) => Some(start),
            _ => None,
        };
        let mut link = None;
        if let Some(start) = moving {
            let e = &self.document.0[i];
            let other = if start { e.to } else { e.from };
            link = self
                .attach(p)
                .filter(|&(j, _)| Some(self.document.0[j].id) != other);
            match link {
                Some((j, Some(side))) => p = self.document.0[j].anchor(side),
                None if constrain => p = self.constrained(i, start, p),
                _ => (),
            }
            self.snap = link;
        }
        let (target, side) = link.map_or((None, None), |(j, side)| (Some(self.document.0[j].id), side));
        let y = g.at[1];
        let e = &mut self.document.0[i];
        match g.motion {
            Motion::Create if e.kind == Kind::Pen => {
                if e.points.len() < 32000 && e.points.last() != Some(&p) {
                    e.points.push(p);
                }
                e.b = p;
            }
            Motion::Create => {
                e.b = p;
                (e.to, e.to_side) = (target, side);
            }
            Motion::Endpoint(start) => {
                if start {
                    e.a = p;
                    (e.from, e.from_side) = (target, side);
                } else {
                    e.b = p;
                    (e.to, e.to_side) = (target, side);
                }
            }
            Motion::Move { .. } => unreachable!(),
            Motion::Resize => {
                let original = &g.before.0[i];
                let (a, b) = original.corners(&g.before.0);
                if e.kind == Kind::Text {
                    e.font_size = (original.font_size + (p[1] - y)).clamp(8., 72.);
                } else {
                    let end = [
                        p[0].max(a[0] + 1.).min(WIDTH),
                        p[1].max(a[1] + 1.).min(HEIGHT),
                    ];
                    e.a = a;
                    e.b = end;
                    if e.kind == Kind::Pen {
                        for (q, old) in e.points.iter_mut().zip(&original.points) {
                            for k in 0..2 {
                                q[k] = a[k]
                                    + (old[k] - a[k]) * (end[k] - a[k]) / (b[k] - a[k]).max(1.);
                            }
                        }
                    }
                }
            }
        }
    }
    fn end(&mut self) {
        self.snap = None;
        if let Some((start, end)) = self.marquee.take() {
            let area = region(start, end);
            if area.size.width > 0. && area.size.height > 0. {
                self.selected = self
                    .document
                    .0
                    .iter()
                    .enumerate()
                    .filter_map(|(i, e)| {
                        let bounds = unsafe { element_bounds(e, &self.document) };
                        let visible = NSIntersectionRect(bounds, region([0., 0.], [WIDTH, HEIGHT]));
                        NSContainsRect(area, visible).then_some(i)
                    })
                    .collect();
            }
            return;
        }
        let Some(g) = self.drag.take() else {
            return;
        };
        if matches!(g.motion, Motion::Create) {
            let i = self.selected[0];
            let e = &self.document.0[i];
            let small = match e.kind {
                Kind::Text => false,
                Kind::Pen => e.points.len() < 2,
                Kind::Arrow => distance(e.a, e.b, e.b) < 5. || self.loops(i),
                _ => (e.a[0] - e.b[0]).abs() < 5. || (e.a[1] - e.b[1]).abs() < 5.,
            };
            if small {
                self.document = g.before;
                self.selected.clear();
                return;
            }
            if !self.locked {
                self.tool = 0;
            }
        } else if matches!(g.motion, Motion::Endpoint(_)) && self.loops(self.selected[0]) {
            self.document = g.before;
            return;
        }
        self.remember(g.before);
    }
    fn single_selection(&self) -> Option<usize> {
        match self.selected.as_slice() {
            [i] => Some(*i),
            _ => None,
        }
    }
    fn command(&mut self, tag: usize) {
        self.marquee = None;
        self.end();
        if tag < 7 || tag == AREA_SELECT {
            if tag == AREA_SELECT && self.tool != AREA_SELECT {
                self.selected.clear();
            }
            self.tool = tag;
            self.locked = false;
            return;
        }
        if matches!(tag, 7 | 8) {
            let (src, dest) = if tag == 7 {
                (&mut self.undo, &mut self.redo)
            } else {
                (&mut self.redo, &mut self.undo)
            };
            if let Some(previous) = src.pop() {
                dest.push(std::mem::replace(&mut self.document, previous));
                self.selected.clear();
            }
            return;
        }
        if self.selected.is_empty() {
            return;
        }
        let old = self.document.clone();
        if tag == 9 {
            let ids: std::collections::HashSet<_> = self
                .selected
                .iter()
                .map(|&i| self.document.0[i].id)
                .collect();
            self.document.0.retain(|e| {
                ![Some(e.id), e.from, e.to]
                    .into_iter()
                    .flatten()
                    .any(|id| ids.contains(&id))
            });
            self.selected.clear();
        }
        self.remember(old);
    }
}
fn near(p: Point, q: Point) -> bool {
    (p[0] - q[0]).hypot(p[1] - q[1]) < 8.
}
fn distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let t =
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy).max(0.0001)).clamp(0., 1.);
    (a[0] + dx * t - p[0]).hypot(a[1] + dy * t - p[1])
}
struct SurfaceState {
    minimap: std::cell::OnceCell<Retained<Minimap>>,
    model: RefCell<Model>,
    pan_anchor: std::cell::Cell<Option<NSPoint>>,
    field: RefCell<Option<(usize, Retained<NSTextField>)>>,
    buttons: RefCell<Vec<Retained<NSButton>>>,
    palette: std::cell::OnceCell<Palette>,
    // A dot on the tool button that stays active after each drawing.
    lock: std::cell::OnceCell<Retained<NSView>>,
}
define_class!(
    #[unsafe(super(NSView,NSResponder))]
    #[name="StologyDiagramSurface"]
    #[thread_kind=MainThreadOnly]
    #[ivars=SurfaceState]
    struct Surface;
    unsafe impl NSObjectProtocol for Surface{}
    unsafe impl NSControlTextEditingDelegate for Surface{
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn text_command(&self,_:&NSControl,t:&NSTextView,cmd:Sel)->bool{
            let marked:bool=msg_send![t,hasMarkedText]; if marked{false} else
            if cmd == sel!(insertLineBreak:) || (cmd == sel!(insertNewline:) &&
                NSApplication::sharedApplication(mtm()).currentEvent().is_some_and(|e| e.modifierFlags().contains(NSEventModifierFlags::Shift))) {
                let _: () = msg_send![t, insertText:&*NSString::from_str("\n"), replacementRange:NSTextInputClient::selectedRange(t)];
                true
            } else if cmd==sel!(insertNewline:) || cmd==sel!(cancelOperation:){
                self.finish_text(cmd==sel!(insertNewline:));self.focus();true
            }else{false}
        }
        #[unsafe(method(controlTextDidChange:))]
        fn change_text(&self,_:&NSNotification){unsafe{
            if let Some((_, field)) = self.ivars().field.borrow().as_ref() {
                if let Some(editor) = field.currentEditor() {
                    let mut frame = field.frame();
                    let rows = editor.string().to_string().split('\n').count();
                    frame.size.height = (rows as f64 * field.font().unwrap().pointSize() * 1.25 + 8.).max(26.);
                    field.setFrame(frame);
                }
            }
        }}
        #[unsafe(method(controlTextDidEndEditing:))]
        fn end_text(&self,_:&NSNotification){unsafe{self.finish_text(true);}}
    }
    unsafe impl NSTextFieldDelegate for Surface{}
    impl Surface{
        #[unsafe(method(isFlipped))] fn flipped(&self)->bool{true}
        #[unsafe(method(acceptsFirstResponder))] fn accepts(&self)->bool{true}
        #[unsafe(method(drawRect:))] fn draw(&self,dirty:NSRect){unsafe{
            NSColor::whiteColor().setFill();NSBezierPath::fillRect(dirty);
            let m=self.ivars().model.borrow();paint(&m.document);
            for &i in &m.selected {
                let(a,b)=m.bounds(i);
                let path=NSBezierPath::bezierPathWithRect(NSInsetRect(region(a,b), -1., -1.));color("#756ee1").setStroke();let dash=[3.,3.];path.setLineDash_count_phase(dash.as_ptr(),2,0.);path.stroke();
                if m.selected.len() != 1 || m.tool == AREA_SELECT { continue; }
                let e=&m.document.0[i];let handles=if e.kind==Kind::Arrow{let(a,b)=e.ends(&m.document.0);vec![a,b]}else{vec![b]};
                for [x,y] in handles{color("#756ee1").setFill();NSBezierPath::fillRect(rect(x-3.5,y-3.5,7.,7.));}
            }
            if let Some((i, side)) = m.snap {
                diagram_arrows::highlight(&m.document.0[i], side);
            }
            if let Some((a,b)) = m.marquee {
                let path = NSBezierPath::bezierPathWithRect(region(a,b));
                let accent = color("#756ee1");
                accent.colorWithAlphaComponent(0.12).setFill();
                path.fill();
                accent.setStroke();
                let dash = [4., 3.];
                path.setLineDash_count_phase(dash.as_ptr(), 2, 0.);
                path.stroke();
            }
        }}
        #[unsafe(method(mouseDown:))]fn down(&self,event:&NSEvent){unsafe{
            self.palette().close();
            self.ivars().pan_anchor.set(None);
            self.finish_text(true);self.focus();let p=self.point(event);
            let mut m=self.ivars().model.borrow_mut();
            if event.clickCount()==2 && m.tool==0 && m.hit(p,false).is_some(){m.selected=m.hit(p,false).into_iter().collect();drop(m);self.edit_text();self.refresh();return}
            m.start(p);let text=m.tool==6 && m.drag.is_some();
            if m.tool == 0 && m.selected.is_empty() {
                if let Some(scroll) = self.enclosingScrollView() {
                    let clip = scroll.contentView();
                    self.ivars().pan_anchor.set(Some(clip.convertPoint_fromView(event.locationInWindow(), None)));
                }
            }
            if text{m.end();} drop(m);if text{self.edit_text()}self.refresh();
        }}
        #[unsafe(method(mouseDragged:))]fn dragged(&self,event:&NSEvent){if let Some(anchor) = self.ivars().pan_anchor.get() {
                self.pan_to_pointer(anchor, event);
            } else {
                let p=self.point(event);
                let constrain = event.modifierFlags().contains(NSEventModifierFlags::Shift);
                self.ivars().model.borrow_mut().update(p, constrain);self.redraw_document();
            }}
        // With the arrow tool, show where the pointer would attach before the drag starts.
        #[unsafe(method(mouseMoved:))]fn moved(&self,event:&NSEvent){
            let p=self.point(event);
            let mut m=self.ivars().model.borrow_mut();
            let snap = if m.tool == ARROW_TOOL && m.drag.is_none() { m.attach(p) } else { None };
            if snap != m.snap { m.snap = snap; drop(m); self.setNeedsDisplay(true); }
        }
        #[unsafe(method(mouseExited:))]fn exited(&self,_:&NSEvent){
            let cleared = self.ivars().model.borrow_mut().snap.take().is_some();
            if cleared { self.setNeedsDisplay(true); }
        }
        #[unsafe(method(mouseUp:))]fn up(&self,event:&NSEvent){
            self.ivars().pan_anchor.set(None);
            let mut m = self.ivars().model.borrow_mut();
            if let Some((_,end)) = &mut m.marquee { *end = self.point(event); }
            m.end();
            drop(m);
            self.refresh();
        }
        #[unsafe(method(openColors:))] fn open_colors(&self,b:&NSButton){unsafe{self.finish_text(true);}self.palette().open(b,&self.ivars().model.borrow());}
        // Right-clicking an arrow selects it and offers its line, route and direction.
        #[unsafe(method(menuForEvent:))] fn menu(&self,event:&NSEvent)->*mut NSMenu{unsafe{
            self.palette().close();self.finish_text(true);self.focus();
            self.arrow_menu(self.point(event)).map_or(std::ptr::null_mut(),Retained::autorelease_return)
        }}
        #[unsafe(method(arrowEdit:))] fn arrow_edit(&self,item:&NSMenuItem){self.edit_arrow(item.tag());}
        #[unsafe(method(arrowKind:))] fn arrow_kind(&self,item:&NSMenuItem){self.choose_arrow_kind(item.tag());}
        #[unsafe(method(arrowKeep:))] fn arrow_keep(&self,_:&NSMenuItem){self.keep_arrow_tool();}
        #[unsafe(method(pickColor:))] fn pick(&self,b:&NSButton){self.pick_color(b);}
        #[unsafe(method(hoverColor:))] fn hover_color(&self,_:&NSButton){self.palette().hover();}
        #[unsafe(method(tool:))]fn tool(&self,button:&NSButton){unsafe{
            self.palette().close();self.finish_text(true);
            let tag = button.tag() as usize;
            if tag == ARROW_TOOL {
                // The arrow tool opens its kinds; the A key draws the kind chosen last.
                let below = NSPoint::new(0., button.bounds().size.height + 3.);
                self.arrow_tool_menu().popUpMenuPositioningItem_atLocation_inView(None, below, Some(button));
                self.refresh();
                self.focus();
                return;
            }
            self.run(tag);
            // Only a double-click on this very button counts, not a stale event.
            let twice = NSApplication::sharedApplication(mtm()).currentEvent().is_some_and(|e| {
                e.r#type() == NSEventType::LeftMouseUp && e.clickCount() >= 2
                    && NSPointInRect(button.convertPoint_fromView(e.locationInWindow(), None), button.bounds())
            });
            if twice && (1..7).contains(&tag) {
                self.ivars().model.borrow_mut().locked = true;
                self.refresh();
            }
            self.focus();
        }}
        #[unsafe(method(undo:))]
        fn undo(&self,_:Option<&AnyObject>){self.run(7);}
        #[unsafe(method(redo:))]
        fn redo(&self,_:Option<&AnyObject>){self.run(8);}
        #[unsafe(method(performKeyEquivalent:))]unsafe fn equivalent(&self,event:&NSEvent)->bool{
            if self.zoom_shortcut(event) {true}
            else if self.ivars().field.borrow().is_none() && event.modifierFlags().contains(NSEventModifierFlags::Command) && event.keyCode()==6{
                self.run(if event.modifierFlags().contains(NSEventModifierFlags::Shift){8}else{7});true
            }else{msg_send![super(self),performKeyEquivalent:event]}
        }
        #[unsafe(method(keyDown:))]unsafe fn key(&self,event:&NSEvent){
            if event.modifierFlags().intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option) {
                let _:()=msg_send![super(self),keyDown:event];
            } else {match event.keyCode(){51|117=>self.run(9),36|76=>self.edit_text(),46=>self.run(AREA_SELECT),53=>{self.run(0);self.ivars().model.borrow_mut().selected.clear();self.refresh();},
                code=>if let Some(i)=[9,15,31,2,0,35,17].iter().position(|&k|k==code){self.run(i);}else{let _:()=msg_send![super(self),keyDown:event];}
            }}
        }
    }
);
impl Surface {
    fn pan_to_pointer(&self, anchor: NSPoint, event: &NSEvent) {
        let Some(scroll) = self.enclosingScrollView() else {
            return;
        };
        let clip = scroll.contentView();
        let pointer = clip.convertPoint_fromView(event.locationInWindow(), None);
        let mut bounds = clip.bounds();
        bounds.origin.x += anchor.x - pointer.x;
        bounds.origin.y += anchor.y - pointer.y;
        clip.scrollToPoint(clip.constrainBoundsRect(bounds).origin);
        scroll.reflectScrolledClipView(&clip);
    }
    fn zoom_shortcut(&self, event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command)
            || flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
        {
            return false;
        }
        let Some(scroll) = self.enclosingScrollView() else {
            return false;
        };
        let key = event
            .charactersIgnoringModifiers()
            .map(|s| s.to_string())
            .unwrap_or_default();
        let factor = match key.as_str() {
            "+" | "=" => 1.25,
            "-" => 1. / 1.25,
            _ => return false,
        };
        let scale = (scroll.magnification() * factor)
            .clamp(scroll.minMagnification(), scroll.maxMagnification());
        let visible = self.visibleRect();
        scroll.setMagnification_centeredAtPoint(scale, visible.mid());
        true
    }
    fn point(&self, event: &NSEvent) -> Point {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        [p.x.clamp(0., WIDTH), p.y.clamp(0., HEIGHT)]
    }
    fn focus(&self) {
        if let Some(w) = self.window() {
            w.makeFirstResponder(Some(self));
        }
    }
    fn redraw_document(&self) {
        self.setNeedsDisplay(true);
        if let Some(map) = self.ivars().minimap.get() {
            map.document_changed();
        }
    }
    fn refresh(&self) {
        self.redraw_document();
        let m = self.ivars().model.borrow();
        let has_selection = !m.selected.is_empty();
        self.palette().refresh(&m);
        let lock = self.lock_dot();
        lock.removeFromSuperview();
        for b in self.ivars().buttons.borrow().iter() {
            let tag = b.tag() as usize;
            let held = m.locked && tag == m.tool;
            if held {
                b.addSubview(lock);
            }
            if tag == ARROW_TOOL {
                if let Some(image) = m.direction.image() {
                    b.setImage(Some(&image));
                }
            }
            if let Some((_, title, _)) = TOOLS.iter().find(|tool| tool.0 == tag) {
                let name = if held { format!("{title} · 고정") } else { title.to_string() };
                b.setAccessibilityLabel(Some(&NSString::from_str(&name)));
            }
            b.setState((tag == m.tool) as _);
            b.setEnabled(match tag {
                7 => !m.undo.is_empty(),
                8 => !m.redo.is_empty(),
                9 => has_selection,
                _ => true,
            });
        }
    }
    fn run(&self, tag: usize) {
        self.ivars().model.borrow_mut().command(tag);
        self.refresh();
    }
    fn lock_dot(&self) -> &NSView {
        self.ivars().lock.get_or_init(|| unsafe {
            let dot = view(rect(24., 3., 5., 5.));
            dot.setWantsLayer(true);
            let layer: Retained<AnyObject> = msg_send![&dot, layer];
            let _: () = msg_send![&layer, setBackgroundColor: &*color("#756ee1").CGColor()];
            let _: () = msg_send![&layer, setCornerRadius: 2.5f64];
            dot
        })
    }
    unsafe fn edit_text(&self) {
        let m = self.ivars().model.borrow();
        let Some(i) = m.single_selection() else {
            return;
        };
        let e = &m.document.0[i];
        if e.kind == Kind::Pen {
            return;
        }
        let (a, b) = e.corners(&m.document.0);
        let [cx, cy] = e.label_point(&m.document.0);
        let width = (b[0] - a[0]).max(140.).min(450.);
        let x = if e.kind == Kind::Text {
            a[0]
        } else {
            cx - width / 2.
        };
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm()),
            rect(
                x.clamp(0., WIDTH - width),
                (cy - 13.).max(0.),
                width,
                (e.label_lines().count() as f64 * e.font_size * 1.25 + 8.).max(26.),
            ),
        );
        field.setMaximumNumberOfLines(0);
        field.setFont(Some(&font::regular(e.font_size)));
        field.setTextColor(Some(&color(e.label_color())));
        field.setStringValue(&NSString::from_str(&e.text));
        field.setAccessibilityLabel(Some(&NSString::from_str("다이어그램 글자")));
        field.setDelegate(Some(ProtocolObject::from_ref(self)));
        drop(m);
        self.addSubview(&field);
        *self.ivars().field.borrow_mut() = Some((i, field.clone()));
        field.selectText(None);
    }
    unsafe fn finish_text(&self, commit: bool) {
        let taken = self.ivars().field.borrow_mut().take();
        if let Some((i, field)) = taken {
            let mut m = self.ivars().model.borrow_mut();
            // A standalone label starts as a temporary empty element. Keep its
            // insertion and its first committed text in the same undo step.
            let creating = m.document.0[i].kind == Kind::Text && m.document.0[i].text.is_empty();
            let old = if creating {
                m.undo.pop().unwrap_or_else(|| m.document.clone())
            } else {
                m.document.clone()
            };
            if commit {
                m.document.0[i].text = field.stringValue().to_string();
            }
            if m.document.0[i].kind == Kind::Text && m.document.0[i].text.is_empty() {
                m.document.0.remove(i);
                m.selected.clear();
            }
            m.remember(old);
            drop(m);
            field.removeFromSuperview();
            self.refresh();
        }
    }
}
pub struct Canvas {
    pub root: Retained<NSView>,
    surface: Retained<Surface>,
    scroll: Retained<NSScrollView>,
    original: Diagram,
    original_title: String,
    pub title: String,
    pub selection: NSRange,
    pub insert_on_caret_line: bool,
    pub existing: Option<(String, String)>,
}
impl Canvas {
    pub unsafe fn new(
        original: Diagram,
        title: String,
        selection: NSRange,
        existing: Option<(String, String)>,
    ) -> Self {
        let root = view(NSRect::ZERO);
        let allocated = Surface::alloc(mtm()).set_ivars(SurfaceState {
            minimap: Default::default(),
            pan_anchor: Default::default(),
            model: RefCell::new(Model {
                document: original.clone(),
                selected: vec![],
                marquee: None,
                tool: 0,
                drag: None,
                undo: vec![],
                redo: vec![],
                direction: Direction::last(),
                locked: false,
                snap: None,
            }),
            buttons: Default::default(),
            palette: Default::default(),
            lock: Default::default(),
            field: Default::default(),
        });
        let surface: Retained<Surface> =
            msg_send![super(allocated),initWithFrame:rect(0.,0.,WIDTH,HEIGHT)];
        surface.setAccessibilityElement(true);
        surface.setAccessibilityRole(Some(NSAccessibilityGroupRole));
        surface.setAccessibilityLabel(Some(&NSString::from_str("다이어그램 캔버스")));
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., 0., 400., 300.));
        scroll.setAllowsMagnification(true);
        scroll.setMinMagnification(0.25);
        scroll.setMaxMagnification(4.);
        scroll.setDocumentView(Some(&surface));
        root.addSubview(&scroll);
        let minimap = Minimap::new(&surface, &scroll);
        root.addSubview(&minimap);
        assert!(surface.ivars().minimap.set(minimap).is_ok());
        assert!(surface.ivars().palette.set(Palette::new(&surface, &root)).is_ok());
        let hover = NSTrackingArea::initWithRect_options_owner_userInfo(
            <NSTrackingArea as objc2::AnyThread>::alloc(),
            NSRect::ZERO,
            NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInKeyWindow
                | NSTrackingAreaOptions::InVisibleRect,
            Some(&surface),
            None,
        );
        surface.addTrackingArea(&hover);
        for (tag, title, symbol) in TOOLS {
            let b = pdf_annotations::tooltip::button(&surface, sel!(tool:), &title);
            b.setTag(tag as _);
            if !(7..=9).contains(&tag) {
                b.setButtonType(NSButtonType::PushOnPushOff);
            }
            b.setAccessibilityLabel(Some(&NSString::from_str(&title)));
            if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                Some(&NSString::from_str(&title)),
            ) {
                b.setImage(Some(&img));
                b.setImagePosition(NSCellImagePosition::ImageOnly);
            }
            root.addSubview(&b);
            surface.ivars().buttons.borrow_mut().push(b);
        }
        surface.refresh();
        Self {
            root,
            surface,
            scroll,
            original,
            original_title: title.clone(),
            title,
            selection,
            insert_on_caret_line: false,
            existing,
        }
    }
    pub fn layout(&self, frame: NSRect, divider: NSRect) {
        self.root.setFrame(frame);
        let palette_width = 54.;
        let canvas_width = (frame.size.width - palette_width).max(1.);
        let tool_left = divider.origin.x;
        let columns = (((canvas_width - tool_left - 8.) / 34.).floor() as usize).clamp(1, TOOLS.len());
        let top = frame.size.height - 4.;
        let tools_bottom = top - TOOLS.len().div_ceil(columns) as f64 * 31. - 6.;
        let footer_height = 48.;
        self.scroll
            .setFrame(rect(0., footer_height, canvas_width, (tools_bottom - footer_height).max(1.)));
        let buttons = self.surface.ivars().buttons.borrow();
        for (i, b) in buttons[..TOOLS.len()].iter().enumerate() {
            b.setFrame(rect(
                tool_left + (i % columns) as f64 * 34.,
                top - 31. - (i / columns) as f64 * 31.,
                32.,
                28.,
            ));
        }
        let y = self.surface.palette().layout(canvas_width, tools_bottom, palette_width);
        let right = divider.max().x;
        let anchor = NSPoint::new(right, frame.size.width - right);
        self.surface.ivars().minimap.get().unwrap().layout(anchor, y);
    }
    pub fn focus(&self) {
        self.surface.focus();
    }
    pub unsafe fn document(&self) -> Diagram {
        self.surface.finish_text(true);
        self.surface.ivars().model.borrow().document.clone()
    }
    pub unsafe fn changed(&self) -> bool {
        self.document() != self.original || self.title != self.original_title
    }
}
