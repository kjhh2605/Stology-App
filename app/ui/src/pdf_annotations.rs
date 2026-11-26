//! Editable PDF annotations. The PDF stores both standard appearance and the
//! small editing model; unknown annotations are never replaced.
#[path = "pdf_annotation_tooltip.rs"]
pub(crate) mod tooltip;
use objc2::{
    class, define_class, msg_send,
    rc::{Allocated, Retained, Weak},
    runtime::AnyObject,
    sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSArray, NSDictionary, NSPoint, NSRect, NSSize, NSString, NSValue};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::{Duration, Instant},
};

const MODEL_KEY: &str = "/StologyPayload";
const HISTORY_BYTES: usize = 4 * 1024 * 1024;
const MAX_POINTS: usize = 100_000;
const SHAPE_DRAG_THRESHOLD: f64 = 3.0;
const MARQUEE_TOOL: isize = 10;
const NOTE_FONT_SIZE: f64 = 14.0;
const NOTE_PADDING: f64 = 2.0;
const NOTE_EDIT_WIDTH: f64 = 220.0;
const NOTE_MIN_WIDTH: f64 = 30.0;
const NOTE_HANDLE: f64 = 8.0;
fn number(v: &Value) -> f64 {
    v.as_f64().filter(|n| n.is_finite()).unwrap_or(0.0)
}
fn bounds(v: &Value) -> NSRect {
    NSRect::new(
        NSPoint::new(number(&v["bounds"][0]), number(&v["bounds"][1])),
        NSSize::new(
            number(&v["bounds"][2]).max(1.0),
            number(&v["bounds"][3]).max(1.0),
        ),
    )
}
fn rect_value(r: NSRect) -> Value {
    json!([r.origin.x, r.origin.y, r.size.width, r.size.height])
}
fn contains(r: NSRect, p: NSPoint) -> bool {
    p.x >= r.origin.x
        && p.y >= r.origin.y
        && p.x <= r.origin.x + r.size.width
        && p.y <= r.origin.y + r.size.height
}
fn contains_rect(outer: NSRect, inner: NSRect) -> bool {
    contains(outer, inner.origin)
        && contains(
            outer,
            NSPoint::new(
                inner.origin.x + inner.size.width,
                inner.origin.y + inner.size.height,
            ),
        )
}
fn command(before: Value, after: Value) -> Value {
    let v = if after.is_null() { &before } else { &after };
    json!({"id":v["id"],"page":v["page"],"before":before,"after":after})
}
fn command_leaves<'a>(c: &'a Value, out: &mut Vec<&'a Value>) {
    if let Some(commands) = c["commands"].as_array() {
        for child in commands {
            command_leaves(child, out);
        }
    } else {
        out.push(c);
    }
}
fn command_pages(c: &Value) -> Vec<usize> {
    let mut leaves = Vec::new();
    command_leaves(c, &mut leaves);
    let mut pages: Vec<_> = leaves
        .iter()
        .filter_map(|c| c["page"].as_u64().map(|p| p as usize))
        .collect();
    pages.sort_unstable();
    pages.dedup();
    pages
}
pub fn inverse(c: &Value) -> Value {
    if let Some(commands) = c["commands"].as_array() {
        return json!({"commands":commands.iter().rev().map(inverse).collect::<Vec<_>>()});
    }
    json!({"id":c["id"],"page":c["page"],"before":c["after"],"after":c["before"]})
}
unsafe fn uuid() -> String {
    let u: Retained<AnyObject> = msg_send![class!(NSUUID), UUID];
    let s: Retained<NSString> = msg_send![&*u, UUIDString];
    s.to_string()
}
unsafe fn page(document: &AnyObject, index: usize) -> Option<Retained<AnyObject>> {
    let n: usize = msg_send![document, pageCount];
    if index >= n {
        None
    } else {
        msg_send![document,pageAtIndex:index]
    }
}
unsafe fn model(annotation: &AnyObject) -> Option<Value> {
    let s: Option<Retained<AnyObject>> =
        msg_send![annotation,valueForAnnotationKey:&*NSString::from_str(MODEL_KEY)];
    let s = s?;
    let yes: bool = msg_send![&*s,isKindOfClass:NSString::class()];
    if !yes {
        return None;
    }
    let s = &*(&*s as *const AnyObject as *const NSString);
    let v: Value = serde_json::from_str(&s.to_string()).ok()?;
    let supported = matches!(
        v["type"].as_str(),
        Some("box" | "area" | "note" | "ink" | "highlight")
    );
    let valid_bounds = v["bounds"].as_array().is_some_and(|b| {
        b.len() == 4
            && b.iter().all(|n| n.as_f64().is_some_and(f64::is_finite))
            && number(&b[2]) > 0.0
            && number(&b[3]) > 0.0
    });
    let identifier: Option<Retained<AnyObject>> =
        msg_send![annotation,valueForAnnotationKey:&*NSString::from_str("/StologyUUID")];
    let identifier = identifier?;
    let is_string: bool = msg_send![&*identifier,isKindOfClass:NSString::class()];
    if !is_string {
        return None;
    }
    let identifier = &*(&*identifier as *const AnyObject as *const NSString);
    (v["schema"] == 1
        && supported
        && valid_bounds
        && v["page"].as_u64().is_some()
        && v["id"].as_str() == Some(identifier.to_string().as_str()))
    .then_some(v)
}
/// Only the requested page is decoded, keeping offscreen editing models cold.
pub unsafe fn read_page(document: &AnyObject, index: usize) -> Vec<Value> {
    let Some(p) = page(document, index) else {
        return vec![];
    };
    let a: Retained<NSArray<AnyObject>> = msg_send![&*p, annotations];
    a.iter().filter_map(|a| model(&a)).collect()
}

/// Works on either the UI PDFDocument or the worker's private PDFDocument.
/// All geometry in the JSON is absolute page space; PDFKit paths/quads are
/// translated to annotation-relative coordinates only at this boundary.
pub unsafe fn apply(document: &AnyObject, c: &Value) -> Result<(), String> {
    let mut leaves = Vec::new();
    command_leaves(c, &mut leaves);
    // Prepare every replacement before touching the document. A group either
    // reaches the UI/worker in full or leaves the document unchanged.
    let prepared: Result<Vec<_>, _> = leaves.into_iter().map(|c| prepare(document, c)).collect();
    for (page, id, replacement) in prepared? {
        let old: Retained<NSArray<AnyObject>> = msg_send![&*page, annotations];
        for annotation in old.iter() {
            if model(&annotation).is_some_and(|m| m["id"].as_str() == Some(&id)) {
                let _: () = msg_send![&*page,removeAnnotation:&*annotation];
            }
        }
        if let Some(annotation) = replacement {
            let _: () = msg_send![&*page,addAnnotation:&*annotation];
        }
    }
    Ok(())
}
type PreparedAnnotation = (Retained<AnyObject>, String, Option<Retained<AnyObject>>);
unsafe fn prepare(document: &AnyObject, c: &Value) -> Result<PreparedAnnotation, String> {
    let index = c["page"].as_u64().ok_or("필기 페이지가 없습니다.")? as usize;
    let p = page(document, index).ok_or("필기 페이지를 찾을 수 없습니다.")?;
    let id = c["id"].as_str().ok_or("필기 식별자가 없습니다.")?;
    let v = &c["after"];
    let replacement = if v.is_null() {
        None
    } else {
        if v["id"].as_str() != Some(id) || v["page"].as_u64() != Some(index as u64) {
            return Err("필기 식별 정보가 일치하지 않습니다.".into());
        }
        let subtype = match v["type"].as_str() {
            Some("box" | "area") => "Square",
            Some("note") => "FreeText",
            Some("ink") => "Ink",
            Some("highlight") => "Highlight",
            _ => return Err("지원하지 않는 필기입니다.".into()),
        };
        let rect = bounds(v);
        let a: Allocated<AnyObject> = msg_send![class!(PDFAnnotation), alloc];
        let a: Option<Retained<AnyObject>> = msg_send![a,initWithBounds:rect,forType:&*NSString::from_str(subtype),withProperties:std::ptr::null::<AnyObject>()];
        let a = a.ok_or("필기 객체를 만들 수 없습니다.")?;
        let _: () = msg_send![&*a,setShouldPrint:true];
        let kind = v["type"].as_str().unwrap_or("");
        let (r, g, b, alpha) = if kind == "highlight" || kind == "area" {
            // Standard Highlight uses PDF multiply blending. PDFKit does not
            // persist its NSColor alpha, so use its native opaque color both
            // before and after reload; Square's appearance carries its alpha.
            (1.0, 0.82, 0.15, if kind == "area" { 0.32 } else { 1.0 })
        } else {
            (0.13, 0.34, 0.88, 1.0)
        };
        let color: Retained<AnyObject> =
            msg_send![class!(NSColor),colorWithSRGBRed:r,green:g,blue:b,alpha:alpha];
        let _: () = msg_send![&*a,setColor:&*color];
        let border: Retained<AnyObject> = msg_send![class!(PDFBorder), new];
        let width = if kind == "area" || kind == "note" {
            0.0f64
        } else {
            1.8f64
        };
        let _: () = msg_send![&*border,setLineWidth:width];
        let _: () = msg_send![&*a,setBorder:&*border];
        if kind == "area" {
            let _: () = msg_send![&*a,setInteriorColor:&*color];
        }
        if kind == "note" {
            let _: () =
                msg_send![&*a,setContents:&*NSString::from_str(v["text"].as_str().unwrap_or(""))];
            let font = crate::font::regular(NOTE_FONT_SIZE);
            let _: () = msg_send![&*a,setFont:&*font];
            let _: () = msg_send![&*a,setFontColor:&*color];
            let clear: Retained<AnyObject> = msg_send![class!(NSColor), clearColor];
            let _: () = msg_send![&*a,setColor:&*clear];
        }
        if kind == "ink" {
            let path: Retained<AnyObject> = msg_send![class!(NSBezierPath), bezierPath];
            for (i, point) in v["points"]
                .as_array()
                .into_iter()
                .flatten()
                .take(MAX_POINTS)
                .enumerate()
            {
                let point = NSPoint::new(
                    number(&point[0]) - rect.origin.x,
                    number(&point[1]) - rect.origin.y,
                );
                if i == 0 {
                    let _: () = msg_send![&*path,moveToPoint:point];
                } else {
                    let _: () = msg_send![&*path,lineToPoint:point];
                }
            }
            let _: () = msg_send![&*a,addBezierPath:&*path];
        }
        if kind == "highlight" {
            let points: Vec<Retained<NSValue>> = v["quads"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|q| {
                    NSValue::valueWithPoint(NSPoint::new(
                        number(&q[0]) - rect.origin.x,
                        number(&q[1]) - rect.origin.y,
                    ))
                })
                .collect();
            if !points.is_empty() {
                let refs: Vec<&NSValue> = points.iter().map(|p| &**p).collect();
                let array = NSArray::from_slice(&refs);
                let _: () = msg_send![&*a,setQuadrilateralPoints:&*array];
            }
        }
        let mut saved = v.clone();
        saved["schema"] = json!(1);
        let ok: bool = msg_send![&*a,setValue:&*NSString::from_str(id),forAnnotationKey:&*NSString::from_str("/NM")];
        let ok3: bool = msg_send![&*a,setValue:&*NSString::from_str(id),forAnnotationKey:&*NSString::from_str("/StologyUUID")];
        let schema: Retained<AnyObject> = msg_send![class!(NSNumber),numberWithInteger:1isize];
        let _: bool = msg_send![&*a,setValue:&*schema,forAnnotationKey:&*NSString::from_str("/StologySchema")];
        let ok2: bool = msg_send![&*a,setValue:&*NSString::from_str(&saved.to_string()),forAnnotationKey:&*NSString::from_str(MODEL_KEY)];
        if !ok || !ok2 || !ok3 {
            return Err("PDF에 필기 식별자를 기록하지 못했습니다.".into());
        }
        Some(a)
    };
    Ok((p, id.to_owned(), replacement))
}
/// Validate the last command for every UUID, not historical intermediate edits.
pub unsafe fn validate_roundtrip(document: &AnyObject, commands: &[Value]) -> Result<(), String> {
    let mut latest = std::collections::HashMap::new();
    let mut leaves = Vec::new();
    for c in commands {
        command_leaves(c, &mut leaves);
    }
    for c in leaves {
        latest.insert(c["id"].as_str().unwrap_or(""), c);
    }
    for (id, c) in latest {
        let models = read_page(document, c["page"].as_u64().unwrap_or(0) as usize);
        let matches: Vec<_> = models
            .iter()
            .filter(|v| v["id"].as_str() == Some(id))
            .collect();
        if c["after"].is_null() {
            if !matches.is_empty() {
                return Err("삭제한 필기가 PDF에 남아 있습니다.".into());
            }
        } else {
            let mut expected = c["after"].clone();
            expected["schema"] = json!(1);
            if matches.len() != 1 || matches[0] != &expected {
                return Err("저장한 PDF의 필기 검증에 실패했습니다.".into());
            }
        }
    }
    Ok(())
}

struct Gesture {
    page: usize,
    start: NSPoint,
    start_in_window: NSPoint,
    shape_started: bool,
    before: Value,
    current: Value,
    resize: bool,
    last_commit: Instant,
    committed: Value,
}
struct Marquee {
    start: NSPoint,
    current: NSPoint,
}
impl Marquee {
    fn rect(&self) -> NSRect {
        NSRect::new(
            NSPoint::new(
                self.start.x.min(self.current.x),
                self.start.y.min(self.current.y),
            ),
            NSSize::new(
                (self.current.x - self.start.x).abs(),
                (self.current.y - self.start.y).abs(),
            ),
        )
    }
}
struct State {
    pdf: Weak<NSView>,
    on_edit: Rc<dyn Fn(Value) -> Result<(), String>>,
    tool: Cell<isize>,
    selected: RefCell<Vec<Value>>,
    marquee: RefCell<Option<Marquee>>,
    gesture: RefCell<Option<Gesture>>,
    undo: RefCell<Vec<Value>>,
    redo: RefCell<Vec<Value>>,
    memo: RefCell<Option<Retained<NoteEditor>>>,
    status: Retained<NSTextField>,
    tool_buttons: RefCell<Vec<Weak<NSButton>>>,
}
define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyPDFAnnotationOverlay"]
    #[thread_kind = MainThreadOnly]
    #[ivars = State]
    struct Overlay;
    impl Overlay {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { false }
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self) -> bool { true }
        #[unsafe(method(hitTest:))]
        fn hit(&self, point: NSPoint) -> *mut NSView { unsafe { self.hit_point(point) } }
        #[unsafe(method(tool:))]
        fn tool(&self, sender: &NSButton) { unsafe { self.choose_tool(sender.tag()); } }
        #[unsafe(method(scrollWheel:))]
        fn scroll(&self, event: &NSEvent) { unsafe { self.forward_scroll(event, false); } }
        #[unsafe(method(magnifyWithEvent:))]
        fn magnify(&self, event: &NSEvent) { unsafe { self.forward_scroll(event, true); } }
        #[unsafe(method(undo:))]
        fn undo_action(&self, _sender: Option<&AnyObject>) { unsafe { self.history(false); } }
        #[unsafe(method(redo:))]
        fn redo_action(&self, _sender: Option<&AnyObject>) { unsafe { self.history(true); } }
        #[unsafe(method(deleteBackward:))]
        fn delete_backward(&self, _sender: Option<&AnyObject>) { unsafe { self.keyboard_delete(); } }
        #[unsafe(method(deleteForward:))]
        fn delete_forward(&self, _sender: Option<&AnyObject>) { unsafe { self.keyboard_delete(); } }
        #[unsafe(method(delete:))]
        fn delete_action(&self, _sender: Option<&AnyObject>) { unsafe { self.keyboard_delete(); } }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: &NSEvent) { unsafe { self.begin(event); } }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) { unsafe { self.drag(event, false); } }
        #[unsafe(method(mouseUp:))]
        fn up(&self, event: &NSEvent) { unsafe { self.drag(event, true); } }
        #[unsafe(method(keyDown:))]
        fn key(&self, event: &NSEvent) { unsafe {
            if !self.handle_key(event) { let _: () = msg_send![super(self), keyDown:event]; }
        } }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _rect: NSRect) { unsafe { self.draw_selection(); } }
    }
);
// Measure with the same native font/word wrapping used by PDF FreeText. The
// two-point inset keeps glyphs clear of PDFKit's appearance edge without making
// the selection cover an otherwise empty editing box.
unsafe fn note_size(text: &str, width: f64) -> NSSize {
    let font = crate::font::regular(NOTE_FONT_SIZE);
    let attrs = NSDictionary::from_slices(&[NSFontAttributeName], &[&*font as &AnyObject]);
    let options =
        NSStringDrawingOptions::UsesLineFragmentOrigin | NSStringDrawingOptions::UsesFontLeading;
    let available = (width - NOTE_PADDING * 2.0).max(1.0);
    let measured = NSString::from_str(text.trim_end())
        .boundingRectWithSize_options_attributes_context(
            NSSize::new(available, 1_000_000.0),
            options,
            Some(&attrs),
            None,
        );
    let line = NSString::from_str("Ag").boundingRectWithSize_options_attributes_context(
        NSSize::new(available, 1_000_000.0),
        options,
        Some(&attrs),
        None,
    );
    NSSize::new(
        measured.size.width.ceil().max(1.0) + NOTE_PADDING * 2.0,
        measured.size.height.ceil().max(line.size.height.ceil()) + NOTE_PADDING * 2.0,
    )
}
/// An unsized note widens with its text up to the typing width, then wraps.
unsafe fn auto_note_size(text: &str) -> NSSize {
    let size = note_size(text, NOTE_EDIT_WIDTH);
    NSSize::new(size.width.max(NOTE_MIN_WIDTH), size.height)
}
/// Hide the stored memo while its in-place editor draws the same text.
unsafe fn show_note(document: &AnyObject, value: &Value, visible: bool) {
    let Some(p) = page(document, value["page"].as_u64().unwrap_or(0) as usize) else {
        return;
    };
    let all: Retained<NSArray<AnyObject>> = msg_send![&*p, annotations];
    for a in all.iter() {
        if model(&a).is_some_and(|m| m["id"] == value["id"]) {
            let _: () = msg_send![&*a, setShouldDisplay: visible];
        }
    }
}
fn note_rect_with_size(original: NSRect, size: NSSize) -> NSRect {
    NSRect::new(
        NSPoint::new(
            original.origin.x,
            original.origin.y + original.size.height - size.height,
        ),
        size,
    )
}
struct NoteState {
    overlay: Weak<Overlay>,
    original: Value,
    current: RefCell<Value>,
    // Page-space size chosen with the editor's resize handle.
    user_size: Cell<Option<NSSize>>,
    scale: f64,
}
define_class!(
 #[unsafe(super(NSTextView,NSText,NSView,NSResponder))]
 #[name="StologyPDFNoteEditor"]
 #[thread_kind=MainThreadOnly]
 #[ivars=NoteState]
 struct NoteEditor;
 impl NoteEditor {
  #[unsafe(method(didChangeText))] fn changed(&self){unsafe{let _:()=msg_send![super(self),didChangeText];self.grow_editor();self.commit();}}
  #[unsafe(method(mouseDown:))] fn down(&self,event:&NSEvent){unsafe{if !self.track_resize(event){let _:()=msg_send![super(self),mouseDown:event];}}}
 }
);
impl NoteEditor {
    fn handle_rect(&self) -> NSRect {
        let b = self.bounds();
        let y = if self.isFlipped() {
            b.origin.y + b.size.height - NOTE_HANDLE
        } else {
            b.origin.y
        };
        NSRect::new(
            NSPoint::new(b.origin.x + b.size.width - NOTE_HANDLE, y),
            NSSize::new(NOTE_HANDLE, NOTE_HANDLE),
        )
    }
    /// Drags the bottom-right handle while writing. The top-left corner stays
    /// put, the text rewraps to the new width, and the height never clips text.
    unsafe fn track_resize(&self, event: &NSEvent) -> bool {
        let local = self.convertPoint_fromView(event.locationInWindow(), None);
        let h = self.handle_rect();
        let hit = NSRect::new(
            NSPoint::new(h.origin.x - 4.0, h.origin.y - 4.0),
            NSSize::new(h.size.width + 8.0, h.size.height + 8.0),
        );
        let (Some(window), Some(parent)) = (self.window(), self.superview()) else {
            return false;
        };
        if !contains(hit, local) {
            return false;
        }
        let scale = self.ivars().scale;
        let start = self.frame();
        let top = start.origin.y + start.size.height;
        let p0 = parent.convertPoint_fromView(event.locationInWindow(), None);
        let (dx, dy) = (
            start.origin.x + start.size.width - p0.x,
            start.origin.y - p0.y,
        );
        loop {
            let Some(e) = window
                .nextEventMatchingMask(NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp)
            else {
                break;
            };
            let p = parent.convertPoint_fromView(e.locationInWindow(), None);
            let width = (p.x + dx - start.origin.x).max(NOTE_MIN_WIDTH * scale);
            let fit = note_size(&self.string().to_string(), width / scale).height * scale;
            let height = (top - (p.y + dy)).max(fit);
            self.setFrame(NSRect::new(
                NSPoint::new(start.origin.x, top - height),
                NSSize::new(width, height),
            ));
            self.ivars()
                .user_size
                .set(Some(NSSize::new(width / scale, height / scale)));
            self.setNeedsDisplay(true);
            parent.setNeedsDisplay(true);
            if e.r#type() == NSEventType::LeftMouseUp {
                break;
            }
        }
        true
    }
    /// Page-space memo size for the current text, and whether the user fixed it.
    unsafe fn fitted_size(&self, text: &str) -> (NSSize, bool) {
        let current = self.ivars().current.borrow();
        let sized = self
            .ivars()
            .user_size
            .get()
            .or_else(|| (current["sized"] == true).then(|| bounds(&current).size));
        match sized {
            // A user-sized note keeps its width and only grows taller for text.
            Some(user) => (
                NSSize::new(
                    user.width,
                    user.height.max(note_size(text, user.width).height),
                ),
                true,
            ),
            None => (auto_note_size(text), false),
        }
    }
    unsafe fn grow_editor(&self) {
        let scale = self.ivars().scale;
        let (size, _) = self.fitted_size(&self.string().to_string());
        self.setFrame(note_rect_with_size(
            self.frame(),
            NSSize::new(size.width * scale, size.height * scale),
        ));
        self.setNeedsDisplay(true);
        if let Some(parent) = self.superview() {
            parent.setNeedsDisplay(true);
        }
    }
    unsafe fn finish(&self) -> bool {
        if !self.commit() {
            return false;
        }
        let Some(overlay) = self.ivars().overlay.load() else {
            return false;
        };
        let before = self.ivars().current.borrow().clone();
        let mut after = before.clone();
        let (size, sized) = self.fitted_size(before["text"].as_str().unwrap_or(""));
        after["bounds"] = rect_value(note_rect_with_size(bounds(&before), size));
        if sized {
            after["sized"] = json!(true);
        }
        if after != before {
            if !overlay.record(command(before, after.clone()), false) {
                return false;
            }
            *overlay.ivars().selected.borrow_mut() = vec![after.clone()];
            *self.ivars().current.borrow_mut() = after;
        }
        true
    }
    unsafe fn commit(&self) -> bool {
        let marked: bool = msg_send![self, hasMarkedText];
        if marked {
            return false;
        }
        let Some(o) = self.ivars().overlay.load() else {
            return false;
        };
        let before = self.ivars().current.borrow().clone();
        let mut after = before.clone();
        after["text"] = json!(self.string().to_string());
        if before != after {
            if !o.record(command(before, after.clone()), false) {
                return false;
            }
            if let Some(doc) = o.ivars().pdf.load().and_then(|pdf| document(&pdf)) {
                show_note(&doc, &after, false);
            }
            *o.ivars().selected.borrow_mut() = vec![after.clone()];
            *self.ivars().current.borrow_mut() = after;
        }
        true
    }
}
unsafe fn document(pdf: &NSView) -> Option<Retained<AnyObject>> {
    msg_send![pdf, document]
}
/// PDFView maintains annotation-derived tooltips/accessibility/thumbnail state
/// independently of its drawing invalidation. Notify it using its own page.
unsafe fn annotations_changed(pdf: &NSView, document: &AnyObject, index: usize) {
    if let Some(page) = page(document, index) {
        let _: () = msg_send![pdf, annotationsChangedOnPage: &*page];
    }
    pdf.setNeedsDisplay(true);
}
impl Overlay {
    unsafe fn hit_point(&self, point: NSPoint) -> *mut NSView {
        if !contains(self.frame(), point) {
            return std::ptr::null_mut();
        }
        if let Some(memo) = self.ivars().memo.borrow().as_ref() {
            if contains(memo.frame(), point) {
                return &**memo as *const NoteEditor as *mut NSView;
            }
        }
        if self.ivars().tool.get() == 0 {
            return std::ptr::null_mut();
        }
        self as *const Self as *mut NSView
    }
    unsafe fn choose_tool(&self, tag: isize) {
        if !self.finish_memo() || !self.finish_gesture() {
            self.update_tool_buttons();
            return;
        }
        match tag {
            6 => self.highlight(),
            7 => self.history(false),
            8 => self.history(true),
            9 => self.delete_selected(),
            tag => {
                self.ivars().tool.set(tag);
                self.ivars().selected.borrow_mut().clear();
                self.setNeedsDisplay(true);
            }
        }
        self.update_tool_buttons();
        // Toolbar clicks must not strand Delete/Backspace focus on a button.
        if self.ivars().tool.get() != 0 {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
        }
    }
    fn update_tool_buttons(&self) {
        for button in self
            .ivars()
            .tool_buttons
            .borrow()
            .iter()
            .filter_map(Weak::load)
        {
            button.setState(if button.tag() == self.ivars().tool.get() {
                1
            } else {
                0
            });
        }
    }
    unsafe fn keyboard_delete(&self) {
        // The native text view owns text deletion and marked-text composition.
        // A responder-chain action must never turn that into annotation deletion.
        if self.ivars().memo.borrow().is_none() {
            self.delete_selected();
        }
    }
    unsafe fn forward_scroll(&self, event: &NSEvent, magnify: bool) {
        if !self.finish_memo() || !self.finish_gesture() {
            return;
        }
        if let Some(pdf) = self.ivars().pdf.load() {
            if magnify {
                let _: () = msg_send![&*pdf,magnifyWithEvent:event];
            } else {
                let _: () = msg_send![&*pdf,scrollWheel:event];
            }
        }
        self.setNeedsDisplay(true);
    }
    unsafe fn handle_key(&self, event: &NSEvent) -> bool {
        let text = event
            .charactersIgnoringModifiers()
            .map(|s| s.to_string())
            .unwrap_or_default();
        if event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command)
            && text == "z"
        {
            self.history(event.modifierFlags().contains(NSEventModifierFlags::Shift));
        } else if matches!(text.as_str(), "\u{8}" | "\u{7f}" | "\u{f728}")
            || matches!(event.keyCode(), 51 | 117)
        {
            self.keyboard_delete();
        } else if text == "\u{1b}" {
            if self.finish_memo() && self.finish_gesture() {
                self.ivars().selected.borrow_mut().clear();
                self.ivars().tool.set(0);
                self.update_tool_buttons();
                self.setNeedsDisplay(true);
            }
        } else {
            return false;
        }
        true
    }
    unsafe fn annotation_rect(
        &self,
        pdf: &NSView,
        doc: &AnyObject,
        value: &Value,
    ) -> Option<NSRect> {
        let page = page(doc, value["page"].as_u64()? as usize)?;
        let rect: NSRect = msg_send![pdf,convertRect:bounds(value),fromPage:&*page];
        Some(self.convertRect_fromView(rect, Some(pdf)))
    }
    unsafe fn draw_selection(&self) {
        let Some(pdf) = self.ivars().pdf.load() else {
            return;
        };
        let Some(doc) = document(&pdf) else {
            return;
        };
        let selected = self.ivars().selected.borrow();
        NSColor::systemBlueColor().setStroke();
        let memo = self.ivars().memo.borrow();
        // The transparent editor sits on its memo; outline the live editor
        // frame with the resize handle inside its bottom-right corner.
        if let Some(m) = memo.as_ref() {
            let f = m.frame();
            let path = NSBezierPath::bezierPathWithRect(f);
            path.setLineWidth(1.5);
            path.stroke();
            NSColor::systemBlueColor().setFill();
            NSBezierPath::bezierPathWithRect(NSRect::new(
                NSPoint::new(f.origin.x + f.size.width - NOTE_HANDLE, f.origin.y),
                NSSize::new(NOTE_HANDLE, NOTE_HANDLE),
            ))
            .fill();
        }
        let editing = memo
            .as_ref()
            .map(|m| m.ivars().current.borrow()["id"].clone());
        for value in selected.iter() {
            if editing.as_ref() == Some(&value["id"]) {
                continue;
            }
            let Some(rect) = self.annotation_rect(&pdf, &doc, value) else {
                continue;
            };
            let grouped = self.ivars().tool.get() == MARQUEE_TOOL || selected.len() > 1;
            let outline = if grouped {
                NSRect::new(
                    NSPoint::new(rect.origin.x - 3.0, rect.origin.y - 3.0),
                    NSSize::new(rect.size.width + 6.0, rect.size.height + 6.0),
                )
            } else {
                rect
            };
            let path = NSBezierPath::bezierPathWithRect(outline);
            if grouped {
                NSColor::colorWithSRGBRed_green_blue_alpha(0.13, 0.34, 0.88, 0.08).setFill();
                path.fill();
                let dash = [4.0f64, 3.0];
                path.setLineDash_count_phase(dash.as_ptr(), dash.len() as isize, 0.0);
            }
            path.setLineWidth(1.5);
            path.stroke();
            // Only the single-item selection tool offers a resize handle.
            if selected.len() == 1 && self.ivars().tool.get() == 1 {
                let Some(page) = page(&doc, value["page"].as_u64().unwrap_or(0) as usize) else {
                    continue;
                };
                let page_bounds = bounds(value);
                let corner = NSPoint::new(
                    page_bounds.origin.x + page_bounds.size.width,
                    page_bounds.origin.y,
                );
                let corner: NSPoint = msg_send![&*pdf,convertPoint:corner,fromPage:&*page];
                let corner = self.convertPoint_fromView(corner, Some(&pdf));
                let handle = NSBezierPath::bezierPathWithRect(NSRect::new(
                    NSPoint::new(corner.x - 4.0, corner.y - 4.0),
                    NSSize::new(8.0, 8.0),
                ));
                NSColor::systemBlueColor().setFill();
                handle.fill();
            }
        }
        if let Some(marquee) = self.ivars().marquee.borrow().as_ref() {
            let rect = marquee.rect();
            if rect.size.width.hypot(rect.size.height) >= SHAPE_DRAG_THRESHOLD {
                let path = NSBezierPath::bezierPathWithRect(rect);
                NSColor::colorWithSRGBRed_green_blue_alpha(0.13, 0.34, 0.88, 0.09).setFill();
                path.fill();
                NSColor::systemBlueColor().setStroke();
                path.setLineWidth(1.0);
                let dash = [4.0f64, 3.0];
                path.setLineDash_count_phase(dash.as_ptr(), dash.len() as isize, 0.0);
                path.stroke();
            }
        }
    }
    unsafe fn drag_marquee(&self, event: &NSEvent, finish: bool) {
        let Some(mut marquee) = self.ivars().marquee.borrow_mut().take() else {
            return;
        };
        marquee.current = self.convertPoint_fromView(event.locationInWindow(), None);
        let rect = marquee.rect();
        let mut selected = Vec::new();
        if rect.size.width.hypot(rect.size.height) >= SHAPE_DRAG_THRESHOLD {
            if let Some(pdf) = self.ivars().pdf.load() {
                if let Some(doc) = document(&pdf) {
                    // Limit model decoding to visible pages. View-space bounds
                    // handle zoom, crop origins, rotation, and page gaps alike.
                    let pages: Retained<NSArray<AnyObject>> = msg_send![&*pdf, visiblePages];
                    for page in pages.iter() {
                        let index: usize = msg_send![&*doc,indexForPage:&*page];
                        for value in read_page(&doc, index) {
                            if let Some(item) = self.annotation_rect(&pdf, &doc, &value) {
                                if contains_rect(rect, item) {
                                    selected.push(value);
                                }
                            }
                        }
                    }
                }
            }
        }
        *self.ivars().selected.borrow_mut() = selected;
        if !finish {
            *self.ivars().marquee.borrow_mut() = Some(marquee);
        }
        self.setNeedsDisplay(true);
    }
    unsafe fn event_point(&self, e: &NSEvent, index: Option<usize>) -> Option<(usize, NSPoint)> {
        let pdf = self.ivars().pdf.load()?;
        let local = pdf.convertPoint_fromView(e.locationInWindow(), None);
        let doc = document(&pdf)?;
        let p = if let Some(i) = index {
            page(&doc, i)?
        } else {
            let p: Option<Retained<AnyObject>> = msg_send![&*pdf,pageForPoint:local,nearest:false];
            p?
        };
        let index: usize = msg_send![&*doc,indexForPage:&*p];
        let point: NSPoint = msg_send![&*pdf,convertPoint:local,toPage:&*p];
        Some((index, point))
    }
    unsafe fn record(&self, c: Value, history: bool) -> bool {
        let Some(pdf) = self.ivars().pdf.load() else {
            return false;
        };
        let Some(doc) = document(&pdf) else {
            return false;
        };
        if let Err(e) = apply(&doc, &c) {
            self.ivars().status.setStringValue(&NSString::from_str(&e));
            return false;
        }
        if let Err(e) = (self.ivars().on_edit)(c.clone()) {
            let _ = apply(&doc, &inverse(&c));
            for index in command_pages(&c) {
                annotations_changed(&pdf, &doc, index);
            }
            self.setNeedsDisplay(true);
            self.ivars().status.setStringValue(&NSString::from_str(&e));
            return false;
        }
        for index in command_pages(&c) {
            annotations_changed(&pdf, &doc, index);
        }
        if history {
            self.push_history(c);
        }
        self.setNeedsDisplay(true);
        true
    }
    fn push_history(&self, c: Value) {
        let mut undo = self.ivars().undo.borrow_mut();
        undo.push(c);
        while undo.len() > 64
            || undo.iter().map(|v| v.to_string().len()).sum::<usize>() > HISTORY_BYTES
        {
            undo.remove(0);
        }
        self.ivars().redo.borrow_mut().clear();
    }
    unsafe fn finish_gesture(&self) -> bool {
        self.ivars().marquee.borrow_mut().take();
        let gesture = self.ivars().gesture.borrow_mut().take();
        if let Some(g) = gesture {
            if !g.shape_started {
                return true;
            }
            if g.current != g.committed
                && !self.record(command(g.committed.clone(), g.current.clone()), false)
            {
                *self.ivars().gesture.borrow_mut() = Some(g);
                return false;
            }
            if g.before != g.current {
                self.push_history(command(g.before, g.current));
            }
        }
        true
    }
    unsafe fn finish_memo(&self) -> bool {
        let m = self.ivars().memo.borrow().clone();
        let Some(m) = m else {
            return true;
        };
        if !m.finish() {
            return false;
        }
        let current = m.ivars().current.borrow().clone();
        let original = m.ivars().original.clone();
        if current["text"].as_str().unwrap_or("").trim().is_empty() {
            // A memo left without text leaves no annotation behind. A memo
            // created by this edit also drops its creation from history.
            if self.record(command(current, Value::Null), false) {
                self.ivars().selected.borrow_mut().clear();
                let mut undo = self.ivars().undo.borrow_mut();
                let created = undo
                    .last()
                    .is_some_and(|c| c["before"].is_null() && c["id"] == original["id"]);
                if created {
                    undo.pop();
                    drop(undo);
                } else {
                    drop(undo);
                    self.push_history(command(original, Value::Null));
                }
            }
        } else {
            if let Some(pdf) = self.ivars().pdf.load() {
                if let Some(doc) = document(&pdf) {
                    show_note(&doc, &current, true);
                    annotations_changed(&pdf, &doc, current["page"].as_u64().unwrap_or(0) as usize);
                }
            }
            if current != original {
                self.push_history(command(original, current));
            }
        }
        m.removeFromSuperview();
        self.ivars().memo.borrow_mut().take();
        true
    }
    unsafe fn edit_note(&self, v: Value) {
        let Some(pdf) = self.ivars().pdf.load() else {
            return;
        };
        let Some(doc) = document(&pdf) else {
            return;
        };
        let Some(p) = page(&doc, v["page"].as_u64().unwrap_or(0) as usize) else {
            return;
        };
        let r: NSRect = msg_send![&*pdf,convertRect:bounds(&v),fromPage:&*p];
        let r = self.convertRect_fromView(r, Some(&pdf));
        let scale: f64 = msg_send![&*pdf, scaleFactor];
        let scale = if scale.is_finite() {
            scale.max(0.01)
        } else {
            1.0
        };
        let allocated = NoteEditor::alloc(MainThreadMarker::new().unwrap()).set_ivars(NoteState {
            overlay: Weak::new(self),
            original: v.clone(),
            current: RefCell::new(v.clone()),
            user_size: Cell::new(None),
            scale,
        });
        let text: Retained<NoteEditor> = msg_send![super(allocated),initWithFrame:r];
        text.setRichText(false);
        // grow_editor owns the frame; native self-sizing would keep the bottom
        // edge fixed and shift the text away from the memo.
        text.setVerticallyResizable(false);
        text.setHorizontallyResizable(false);
        text.setMinSize(NSSize::new(0.0, 0.0));
        text.setFont(Some(&crate::font::regular(NOTE_FONT_SIZE * scale)));
        text.setTextContainerInset(NSSize::new(NOTE_PADDING * scale, NOTE_PADDING * scale));
        if let Some(container) = text.textContainer() {
            container.setLineFragmentPadding(0.0);
        }
        text.setString(&NSString::from_str(v["text"].as_str().unwrap_or("")));
        text.grow_editor();
        text.setDrawsBackground(false);
        // Same color as the stored FreeText so editing looks like the memo itself.
        text.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
            0.13, 0.34, 0.88, 1.0,
        )));
        show_note(&doc, &v, false);
        annotations_changed(&pdf, &doc, v["page"].as_u64().unwrap_or(0) as usize);
        text.setAutoresizingMask(NSAutoresizingMaskOptions::empty());
        self.addSubview(&text);
        if let Some(w) = self.window() {
            w.makeFirstResponder(Some(&text));
        }
        *self.ivars().memo.borrow_mut() = Some(text);
    }
    unsafe fn begin(&self, e: &NSEvent) {
        if !self.finish_memo() || !self.finish_gesture() {
            return;
        }
        if let Some(w) = self.window() {
            w.makeFirstResponder(Some(self));
        }
        if self.ivars().tool.get() == MARQUEE_TOOL {
            let start = self.convertPoint_fromView(e.locationInWindow(), None);
            self.ivars().selected.borrow_mut().clear();
            *self.ivars().marquee.borrow_mut() = Some(Marquee {
                start,
                current: start,
            });
            self.setNeedsDisplay(true);
            return;
        }
        let Some((index, p)) = self.event_point(e, None) else {
            return;
        };
        let tool = self.ivars().tool.get();
        let mut before = Value::Null;
        let mut current;
        let mut resize = false;
        if tool == 1 {
            let Some(pdf) = self.ivars().pdf.load() else {
                return;
            };
            let Some(doc) = document(&pdf) else {
                return;
            };
            let hit = read_page(&doc, index)
                .into_iter()
                .rev()
                .find(|v| contains(bounds(v), p));
            *self.ivars().selected.borrow_mut() = hit.clone().into_iter().collect();
            if let Some(v) = hit {
                if v["type"] == "note" && e.clickCount() > 1 {
                    self.edit_note(v);
                    return;
                }
                let r = bounds(&v);
                let scale: f64 = msg_send![&*pdf, scaleFactor];
                let tolerance = 12.0 / scale.max(0.01);
                resize = (p.x - (r.origin.x + r.size.width)).abs() < tolerance
                    && (p.y - r.origin.y).abs() < tolerance;
                before = v.clone();
                current = v;
            } else {
                self.setNeedsDisplay(true);
                return;
            }
        } else {
            let kind = match tool {
                2 => "area",
                3 => "box",
                4 => "note",
                5 => "ink",
                _ => return,
            };
            current = json!({"schema":1,"id":uuid(),"page":index,"type":kind,"bounds":[p.x,p.y,1.0,1.0],"text":"","points":[[p.x,p.y]]});
            if tool == 4 {
                let size = auto_note_size("");
                current["bounds"] = json!([p.x, p.y - size.height, size.width, size.height]);
                if self.record(command(Value::Null, current.clone()), true) {
                    *self.ivars().selected.borrow_mut() = vec![current.clone()];
                    self.edit_note(current);
                }
                return;
            }
        }
        *self.ivars().gesture.borrow_mut() = Some(Gesture {
            page: index,
            start: p,
            start_in_window: e.locationInWindow(),
            shape_started: !before.is_null()
                || !matches!(current["type"].as_str(), Some("box" | "area")),
            before: before.clone(),
            current,
            resize,
            last_commit: Instant::now(),
            committed: before,
        });
    }
    unsafe fn drag(&self, e: &NSEvent, finish: bool) {
        if self.ivars().marquee.borrow().is_some() {
            self.drag_marquee(e, finish);
            return;
        }
        let Some(mut g) = self.ivars().gesture.borrow_mut().take() else {
            return;
        };
        if !g.shape_started {
            // Window coordinates are logical screen points, so a click or tiny
            // pointer jitter is ignored consistently at every PDF zoom level.
            let pointer = e.locationInWindow();
            if (pointer.x - g.start_in_window.x).hypot(pointer.y - g.start_in_window.y)
                < SHAPE_DRAG_THRESHOLD
            {
                if !finish {
                    *self.ivars().gesture.borrow_mut() = Some(g);
                }
                return;
            }
            g.shape_started = true;
        }
        let Some((_, p)) = self.event_point(e, Some(g.page)) else {
            *self.ivars().gesture.borrow_mut() = Some(g);
            return;
        };
        if g.before.is_null() {
            if g.current["type"] == "ink" {
                let points = g.current["points"].as_array_mut().unwrap();
                if points.len() < MAX_POINTS {
                    points.push(json!([p.x, p.y]));
                }
                let old = bounds(&g.current);
                let minx = (old.origin.x + 2.0).min(g.start.x).min(p.x);
                let miny = (old.origin.y + 2.0).min(g.start.y).min(p.y);
                let maxx = (old.origin.x + old.size.width - 2.0)
                    .max(g.start.x)
                    .max(p.x);
                let maxy = (old.origin.y + old.size.height - 2.0)
                    .max(g.start.y)
                    .max(p.y);
                g.current["bounds"] =
                    json!([minx - 2.0, miny - 2.0, maxx - minx + 4.0, maxy - miny + 4.0]);
            } else {
                g.current["bounds"] = json!([
                    g.start.x.min(p.x),
                    g.start.y.min(p.y),
                    (p.x - g.start.x).abs().max(2.0),
                    (p.y - g.start.y).abs().max(2.0)
                ]);
            }
        } else {
            let r = bounds(&g.before);
            let mut target = r;
            let note = g.before["type"] == "note";
            if g.resize {
                let top = r.origin.y + r.size.height;
                target.origin.y = p.y.min(top - 2.0);
                target.size.width = (p.x - r.origin.x).max(if note { NOTE_MIN_WIDTH } else { 2.0 });
                target.size.height = top - target.origin.y;
                if note {
                    let text = g.before["text"].as_str().unwrap_or("");
                    let fit = note_size(text, target.size.width).height;
                    target.size.height = target.size.height.max(fit);
                    target.origin.y = top - target.size.height;
                }
            } else {
                target.origin.x += p.x - g.start.x;
                target.origin.y += p.y - g.start.y;
            }
            g.current = transform(&g.before, target);
            if note && g.resize {
                g.current["sized"] = json!(true);
            }
        }
        let Some(pdf) = self.ivars().pdf.load() else {
            return;
        };
        let Some(doc) = document(&pdf) else {
            return;
        };
        let _ = apply(&doc, &command(g.committed.clone(), g.current.clone()));
        annotations_changed(&pdf, &doc, g.page);
        *self.ivars().selected.borrow_mut() = vec![g.current.clone()];
        self.setNeedsDisplay(true);
        if finish || g.last_commit.elapsed() >= Duration::from_millis(100) {
            if g.current != g.committed {
                if !self.record(command(g.committed.clone(), g.current.clone()), false) {
                    g.current = g.committed.clone();
                    *self.ivars().selected.borrow_mut() = (!g.committed.is_null())
                        .then(|| g.committed.clone())
                        .into_iter()
                        .collect();
                    self.setNeedsDisplay(true);
                } else {
                    g.committed = g.current.clone();
                    g.last_commit = Instant::now();
                }
            }
        }
        if finish {
            if g.before != g.committed {
                self.push_history(command(g.before, g.committed));
            }
        } else {
            *self.ivars().gesture.borrow_mut() = Some(g);
        }
    }
    unsafe fn delete_selected(&self) {
        if !self.finish_memo() || !self.finish_gesture() {
            return;
        }
        let selected = std::mem::take(&mut *self.ivars().selected.borrow_mut());
        if selected.is_empty() {
            return;
        }
        let mut commands: Vec<_> = selected
            .iter()
            .cloned()
            .map(|v| command(v, Value::Null))
            .collect();
        let action = if commands.len() == 1 {
            commands.pop().unwrap()
        } else {
            json!({"commands":commands})
        };
        if !self.record(action, true) {
            *self.ivars().selected.borrow_mut() = selected;
        }
    }
    unsafe fn history(&self, redo: bool) {
        if !self.finish_memo() || !self.finish_gesture() {
            return;
        }
        let c = if redo {
            self.ivars().redo.borrow_mut().pop()
        } else {
            self.ivars().undo.borrow_mut().pop()
        };
        if let Some(c) = c {
            let action = if redo { c.clone() } else { inverse(&c) };
            if self.record(action, false) {
                if redo {
                    self.ivars().undo.borrow_mut().push(c);
                } else {
                    self.ivars().redo.borrow_mut().push(c);
                }
                self.ivars().selected.borrow_mut().clear();
            } else if redo {
                self.ivars().redo.borrow_mut().push(c);
            } else {
                self.ivars().undo.borrow_mut().push(c);
            }
        }
    }
    unsafe fn highlight(&self) {
        let Some(pdf) = self.ivars().pdf.load() else {
            return;
        };
        let Some(doc) = document(&pdf) else {
            return;
        };
        let selection: Option<Retained<AnyObject>> = msg_send![&*pdf, currentSelection];
        let Some(selection) = selection else {
            self.ivars().status.setStringValue(&NSString::from_str(
                "읽기 도구로 텍스트를 선택한 뒤 강조를 누르세요.",
            ));
            return;
        };
        let lines: Retained<NSArray<AnyObject>> = msg_send![&*selection, selectionsByLine];
        for line in lines.iter() {
            let pages: Retained<NSArray<AnyObject>> = msg_send![&*line, pages];
            for p in pages.iter() {
                let index: usize = msg_send![&*doc,indexForPage:&*p];
                let r: NSRect = msg_send![&*line,boundsForPage:&*p];
                if r.size.width <= 0.0 || r.size.height <= 0.0 {
                    continue;
                }
                let x = r.origin.x;
                let y = r.origin.y;
                let w = r.size.width;
                let h = r.size.height;
                let v = json!({"schema":1,"id":uuid(),"page":index,"type":"highlight","bounds":rect_value(r),"quads":[[x,y+h],[x+w,y+h],[x,y],[x+w,y]]});
                self.record(command(Value::Null, v), true);
            }
        }
    }
}
/// Preserve pen/quad geometry when moving or resizing, without converting to
/// screen coordinates. This also works for rotated and non-zero crop boxes.
fn transform(v: &Value, target: NSRect) -> Value {
    let old = bounds(v);
    let sx = target.size.width / old.size.width;
    let sy = target.size.height / old.size.height;
    let mut out = v.clone();
    out["bounds"] = rect_value(target);
    for key in ["points", "quads"] {
        if let Some(points) = out.get_mut(key).and_then(Value::as_array_mut) {
            for p in points {
                let x = number(&p[0]);
                let y = number(&p[1]);
                *p = json!([
                    target.origin.x + (x - old.origin.x) * sx,
                    target.origin.y + (y - old.origin.y) * sy
                ]);
            }
        }
    }
    out
}

pub unsafe fn create(
    pdf: &NSView,
    frame: NSRect,
    on_edit: Rc<dyn Fn(Value) -> Result<(), String>>,
) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().unwrap();
    let root = NSView::initWithFrame(NSView::alloc(mtm), frame);
    root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let height = (frame.size.height - 64.0).max(1.0);
    let content = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(frame.size.width, height),
    );
    pdf.setFrame(content);
    pdf.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    root.addSubview(pdf);
    let status = NSTextField::labelWithString(&NSString::from_str("PDF에 저장됨"), mtm);
    status.setFrame(NSRect::new(
        NSPoint::new(10.0, height + 2.0),
        NSSize::new((frame.size.width - 20.0).max(10.0), 20.0),
    ));
    status.setFont(Some(&crate::font::regular(11.0)));
    status.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
    );
    root.addSubview(&status);
    let allocated = Overlay::alloc(mtm).set_ivars(State {
        pdf: Weak::new(pdf),
        on_edit,
        tool: Cell::new(0),
        selected: RefCell::new(vec![]),
        marquee: RefCell::new(None),
        gesture: RefCell::new(None),
        undo: RefCell::new(vec![]),
        redo: RefCell::new(vec![]),
        memo: RefCell::new(None),
        status,
        tool_buttons: RefCell::new(vec![]),
    });
    let overlay: Retained<Overlay> = msg_send![super(allocated),initWithFrame:content];
    overlay.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    overlay.setAccessibilityLabel(Some(&NSString::from_str(
        "PDF 필기 · 선택 도구로 이동·크기 변경, 영역 선택 도구로 여러 필기 선택, Delete 또는 백스페이스로 삭제",
    )));
    root.addSubview(&overlay);
    let mut x = 8.0;
    for (tag, label, symbol, help) in [
        (0, "읽기", "doc.text", "읽기"),
        (1, "선택", "cursorarrow", "선택·이동"),
        (
            MARQUEE_TOOL,
            "영역 선택",
            "rectangle.dashed",
            "여러 필기 선택",
        ),
        (6, "강조", "highlighter", "텍스트 강조"),
        (2, "영역", "rectangle.inset.filled", "영역 강조"),
        (3, "박스", "rectangle", "박스 그리기"),
        (4, "메모", "text.bubble", "메모 쓰기"),
        (5, "펜", "pencil.tip", "펜 쓰기"),
        (7, "실행 취소", "arrow.uturn.backward", "실행 취소 (⌘Z)"),
        (8, "다시 실행", "arrow.uturn.forward", "다시 실행 (⇧⌘Z)"),
        (9, "삭제", "trash", "선택 삭제 (⌫)"),
    ] {
        let button = tooltip::button(&overlay, sel!(tool:), help);
        let width = 32.0;
        button.setFrame(NSRect::new(
            NSPoint::new(x, height + 26.0),
            NSSize::new(width, 28.0),
        ));
        button.setTag(tag);
        button.setRefusesFirstResponder(true);
        button.setAccessibilityLabel(Some(&NSString::from_str(label)));
        button.setAccessibilityHelp(Some(&NSString::from_str(help)));
        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            Some(&NSString::from_str(label)),
        )
        .or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str("questionmark.square"),
                Some(&NSString::from_str(label)),
            )
        });
        if let Some(image) = image {
            let configuration =
                NSImageSymbolConfiguration::configurationWithPointSize_weight(15.0, 0.0);
            let image = image
                .imageWithSymbolConfiguration(&configuration)
                .unwrap_or(image);
            button.setImage(Some(&image));
            button.setImagePosition(NSCellImagePosition::ImageOnly);
        }
        if tag <= 5 || tag == MARQUEE_TOOL {
            button.setButtonType(NSButtonType::PushOnPushOff);
            overlay
                .ivars()
                .tool_buttons
                .borrow_mut()
                .push(Weak::new(&*button));
        }
        button.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        root.addSubview(&button);
        x += width + 3.0;
    }
    overlay.update_tool_buttons();
    root
}
unsafe fn overlay(view: &NSView) -> Option<Retained<Overlay>> {
    for child in view.subviews().iter() {
        let yes: bool = msg_send![&*child,isKindOfClass:Overlay::class()];
        if yes {
            return Retained::retain(&*child as *const NSView as *mut Overlay);
        }
    }
    None
}
pub unsafe fn commit_pending(view: &NSView) -> bool {
    let Some(o) = overlay(view) else {
        return true;
    };
    if !o.finish_memo() {
        return false;
    }
    o.finish_gesture()
}

pub unsafe fn set_status(view: &NSView, text: &str) {
    if let Some(o) = overlay(view) {
        o.ivars().status.setStringValue(&NSString::from_str(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geometry_transform_keeps_absolute_page_coordinates() {
        let v = json!({"bounds":[30.0,40.0,100.0,50.0],"points":[[30.0,40.0],[130.0,90.0]],"quads":[[40.0,50.0]]});
        let out = transform(
            &v,
            NSRect::new(NSPoint::new(50.0, 60.0), NSSize::new(200.0, 100.0)),
        );
        assert_eq!(out["points"], json!([[50.0, 60.0], [250.0, 160.0]]));
        assert_eq!(out["quads"], json!([[70.0, 80.0]]));
    }
    #[test]
    fn inverse_restores_deleted_annotation() {
        let v = json!({"id":"a","page":2,"type":"box"});
        let c = command(v.clone(), Value::Null);
        assert_eq!(inverse(&c)["after"], v);
        assert_eq!(inverse(&inverse(&c)), c);
    }
    #[test]
    fn grouped_inverse_restores_items_in_reverse_order() {
        let a = command(json!({"id":"a","page":0}), Value::Null);
        let b = command(json!({"id":"b","page":1}), Value::Null);
        let grouped = json!({"commands":[a.clone(), b.clone()]});
        assert_eq!(
            inverse(&grouped),
            json!({"commands":[inverse(&b), inverse(&a)]})
        );
        assert_eq!(inverse(&inverse(&grouped)), grouped);
        assert_eq!(command_pages(&grouped), vec![0, 1]);
    }
    #[test]
    fn marquee_requires_full_bounds_inside_drag_rectangle() {
        let outer = NSRect::new(NSPoint::new(10.0, 20.0), NSSize::new(100.0, 80.0));
        assert!(contains_rect(outer, outer));
        assert!(contains_rect(
            outer,
            NSRect::new(NSPoint::new(20.0, 30.0), NSSize::new(20.0, 20.0))
        ));
        assert!(!contains_rect(
            outer,
            NSRect::new(NSPoint::new(5.0, 30.0), NSSize::new(20.0, 20.0))
        ));
        assert!(!contains_rect(
            outer,
            NSRect::new(NSPoint::new(90.0, 80.0), NSSize::new(30.0, 30.0))
        ));
    }
    #[test]
    fn fractional_annotation_payload_survives_json_without_ulp_drift() {
        // Regression: the default serde_json parser rounded these actual drag
        // coordinates by one ULP and falsely rejected an otherwise valid PDF.
        let value = json!({"schema":1,"id":"C617E0B7-FEA0-4FFE-8F0C-98F2058DA7E0","page":0,"type":"box","bounds":[193.33170656813786,563.3144329896895,199.74226804123708,79.89690721649504],"points":[[193.33170656813786,643.2113402061846]],"quads":null,"text":""});
        let decoded: Value = serde_json::from_str(&value.to_string()).unwrap();
        assert_eq!(decoded, value);
    }
    #[test]
    fn production_annotations_survive_worker_write_reopen_and_delete() {
        // A one-page PDF with a foreign note; no UI/AppKit objects cross threads.
        objc2::rc::autoreleasepool(|_| unsafe {
            use objc2_foundation::NSURL;
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("annotation-roundtrip.pdf");
            let objects=["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [5 0 R] >>", "<< /Length 0 >>\nstream\n\nendstream", "<< /Type /Annot /Subtype /Text /Rect [400 400 420 420] /Contents (foreign note) >>"];
            let mut bytes = b"%PDF-1.4\n".to_vec();
            let mut offsets = vec![0];
            for (i, o) in objects.iter().enumerate() {
                offsets.push(bytes.len());
                bytes.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, o).as_bytes());
            }
            let xref = bytes.len();
            bytes.extend_from_slice(
                format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes(),
            );
            for offset in offsets.iter().skip(1) {
                bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
            }
            bytes.extend_from_slice(
                format!(
                    "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                    offsets.len()
                )
                .as_bytes(),
            );
            std::fs::write(&path, bytes).unwrap();
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let doc: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            let mut commands = Vec::new();
            for (i, kind) in ["box", "area", "note", "ink", "highlight"]
                .iter()
                .enumerate()
            {
                let v = json!({"schema":1,"id":format!("test-{kind}"),"page":0,"type":kind,"bounds":[30.0,50.0+i as f64*100.0,120.0,60.0],"text":"한글 메모\n두 번째 줄","points":[[35.0,355.0],[70.0,390.0]],"quads":[[30.0,510.0],[150.0,510.0],[30.0,450.0],[150.0,450.0]]});
                let c = command(Value::Null, v);
                apply(&doc, &c).unwrap();
                commands.push(c);
            }
            // A moved/resized note must retain real PDF contents, not merely
            // the editing payload, while its native bounds change.
            let original = commands[2]["after"].clone();
            let resized = transform(
                &original,
                NSRect::new(NSPoint::new(70.0, 260.0), NSSize::new(180.0, 90.0)),
            );
            apply(&doc, &command(original, resized.clone())).unwrap();
            commands[2] = command(Value::Null, resized);
            let ok: bool = msg_send![&*doc,writeToURL:&*url];
            assert!(ok);
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let reopened: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            validate_roundtrip(&reopened, &commands).unwrap();
            let p = page(&reopened, 0).unwrap();
            let all: Retained<NSArray<AnyObject>> = msg_send![&*p, annotations];
            assert_eq!(all.len(), 6);
            for a in all.iter() {
                let Some(v) = model(&a) else {
                    continue;
                };
                let saved_bounds: NSRect = msg_send![&*a, bounds];
                assert_eq!(saved_bounds, bounds(&v));
                if v["type"] == "note" {
                    let text: Retained<NSString> = msg_send![&*a, contents];
                    assert_eq!(text.to_string(), "한글 메모\n두 번째 줄");
                }
                if v["type"] == "highlight" {
                    let quad: Retained<NSArray<NSValue>> = msg_send![&*a, quadrilateralPoints];
                    assert_eq!(quad.len(), 4);
                    assert_eq!(quad.objectAtIndex(0).pointValue(), NSPoint::new(0.0, 60.0));
                }
                if v["type"] == "ink" {
                    let paths: Retained<NSArray<AnyObject>> = msg_send![&*a, paths];
                    assert_eq!(paths.len(), 1);
                }
            }
            // Reproduce a fractional drag followed by undo/redo. Exact JSON
            // validation must still pass when the final command is a redo.
            let original_box = commands[0]["after"].clone();
            let moved_box = transform(
                &original_box,
                NSRect::new(
                    NSPoint::new(193.33170656813786, 563.3144329896895),
                    NSSize::new(199.74226804123708, 79.89690721649504),
                ),
            );
            let moved = command(original_box, moved_box);
            let replay = vec![moved.clone(), inverse(&moved), moved];
            for c in &replay {
                apply(&reopened, c).unwrap();
            }
            let ok: bool = msg_send![&*reopened,writeToURL:&*url];
            assert!(ok);
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let after_redo: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            validate_roundtrip(&after_redo, &replay).unwrap();
            assert_eq!(read_page(&after_redo, 0).len(), 5);
            // A grouped deletion is one command through save/reopen/undo and
            // roundtrip validation must inspect every child UUID.
            let models = read_page(&after_redo, 0);
            let grouped = json!({"commands":models.iter().take(2).cloned()
                .map(|v| command(v, Value::Null)).collect::<Vec<_>>()});
            apply(&after_redo, &grouped).unwrap();
            assert_eq!(read_page(&after_redo, 0).len(), 3);
            let ok: bool = msg_send![&*after_redo,writeToURL:&*url];
            assert!(ok);
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let grouped_reopened: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            let mut grouped_history = replay.clone();
            grouped_history.push(grouped.clone());
            validate_roundtrip(&grouped_reopened, &grouped_history).unwrap();
            apply(&grouped_reopened, &inverse(&grouped)).unwrap();
            assert_eq!(read_page(&grouped_reopened, 0).len(), 5);
            grouped_history.push(inverse(&grouped));
            validate_roundtrip(&grouped_reopened, &grouped_history).unwrap();
            // A later invalid child must not leave an earlier deletion applied.
            let invalid = json!({"commands":[grouped["commands"][0],
                {"id":"missing-page","page":999,"before":null,"after":null}]});
            assert!(apply(&grouped_reopened, &invalid).is_err());
            assert_eq!(read_page(&grouped_reopened, 0).len(), 5);
            validate_roundtrip(&grouped_reopened, &grouped_history).unwrap();
            // Replacement is UUID-idempotent, deletion preserves the foreign note.
            apply(&reopened, &commands[0]).unwrap();
            for c in &commands {
                apply(&reopened, &inverse(c)).unwrap();
            }
            let all: Retained<NSArray<AnyObject>> = msg_send![&*p, annotations];
            assert_eq!(all.len(), 1);
            let foreign: Retained<NSString> = msg_send![&*all.objectAtIndex(0), contents];
            assert_eq!(foreign.to_string(), "foreign note");
        });
    }
}
