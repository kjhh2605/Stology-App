//! Native memo bounds regression. Hidden windows and temporary PDF files only.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pdf-note-fit-smoke
#[path = "support/controls.rs"]
#[allow(dead_code)]
mod controls;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
use controls::{choose, key};
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
    MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString, NSURL};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::Rc,
};
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

fn fixture(path: &Path) {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
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
    std::fs::write(path, bytes).unwrap();
}
unsafe fn mouse(
    overlay: &NSView,
    window: &NSWindow,
    kind: NSEventType,
    location: NSPoint,
    clicks: isize,
) {
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, location, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, clicks, 1.).unwrap();
    let _: () = match kind {
        NSEventType::LeftMouseDown => msg_send![overlay,mouseDown:&*event],
        NSEventType::LeftMouseDragged => msg_send![overlay,mouseDragged:&*event],
        _ => msg_send![overlay,mouseUp:&*event],
    };
}
unsafe fn text_editor(window: &NSWindow) -> Retained<NSTextView> {
    let responder = window.firstResponder().unwrap();
    let yes: bool = msg_send![&*responder,isKindOfClass:class!(NSTextView)];
    assert!(yes, "memo must own keyboard focus");
    Retained::retain(&*responder as *const NSResponder as *mut NSTextView).unwrap()
}
unsafe fn replace_text(window: &NSWindow, value: &str) {
    let text = text_editor(window);
    let range = NSRange::new(0, text.string().to_string().encode_utf16().count());
    let _: () = msg_send![&*text,insertText:&*NSString::from_str(value),replacementRange:range];
}
fn rect(model: &Value) -> NSRect {
    let b = &model["bounds"];
    NSRect::new(
        NSPoint::new(b[0].as_f64().unwrap(), b[1].as_f64().unwrap()),
        NSSize::new(b[2].as_f64().unwrap(), b[3].as_f64().unwrap()),
    )
}
fn top(rect: NSRect) -> f64 {
    rect.origin.y + rect.size.height
}
fn assert_anchor(before: NSRect, after: NSRect) {
    assert!(
        (before.origin.x - after.origin.x).abs() < 0.001,
        "horizontal text anchor moved"
    );
    assert!(
        (top(before) - top(after)).abs() < 0.001,
        "top text anchor moved"
    );
}
struct Fixture {
    _temp: tempfile::TempDir,
    url: Retained<NSURL>,
    document: Retained<AnyObject>,
    page: Retained<AnyObject>,
    pdf: Retained<NSView>,
    root: Retained<NSView>,
    overlay: Retained<NSView>,
    window: Retained<NSWindow>,
    commands: Rc<RefCell<Vec<Value>>>,
    reject: Rc<Cell<bool>>,
}
impl Fixture {
    unsafe fn new() -> Self {
        let mtm = MainThreadMarker::new().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("note.pdf");
        fixture(&path);
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
        let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
        let document: Retained<AnyObject> = msg_send![allocated,initWithURL:&*url];
        let page: Retained<AnyObject> = msg_send![&*document,pageAtIndex:0usize];
        let pdf: Retained<NSView> = msg_send![class!(PDFView), new];
        let _: () = msg_send![&*pdf,setDocument:&*document];
        let _: () = msg_send![&*pdf,setAutoScales:false];
        let commands = Rc::new(RefCell::new(Vec::new()));
        let recorded = commands.clone();
        let reject = Rc::new(Cell::new(false));
        let reject_edits = reject.clone();
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 700.));
        let root = pdf_annotations::create(
            &pdf,
            frame,
            Rc::new(move |c| {
                if reject_edits.get() {
                    Err("injected journal rejection".into())
                } else {
                    recorded.borrow_mut().push(c);
                    Ok(())
                }
            }),
        );
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setContentView(Some(&root));
        root.layoutSubtreeIfNeeded();
        let overlay = root
            .subviews()
            .iter()
            .find(|view| {
                let yes: bool =
                    msg_send![&**view,isKindOfClass:class!(StologyPDFAnnotationOverlay)];
                yes
            })
            .unwrap();
        let result = Self {
            _temp: temp,
            url,
            document,
            page,
            pdf,
            root,
            overlay,
            window,
            commands,
            reject,
        };
        result.layout(0, 0.8);
        result
    }
    unsafe fn layout(&self, rotation: isize, scale: f64) {
        let _: () = msg_send![&*self.page,setRotation:rotation];
        let _: () = msg_send![&*self.pdf,setScaleFactor:scale];
        let _: () = msg_send![&*self.pdf, layoutDocumentView];
    }
    unsafe fn point(&self, x: f64, y: f64) -> NSPoint {
        let p: NSPoint = msg_send![&*self.pdf,convertPoint:NSPoint::new(x,y),fromPage:&*self.page];
        self.pdf.convertPoint_toView(p, None)
    }
    unsafe fn click(&self, p: NSPoint, count: isize) {
        mouse(
            &self.overlay,
            &self.window,
            NSEventType::LeftMouseDown,
            p,
            count,
        );
        mouse(
            &self.overlay,
            &self.window,
            NSEventType::LeftMouseUp,
            p,
            count,
        );
    }
    unsafe fn note(&self) -> Value {
        let models = pdf_annotations::read_page(&self.document, 0);
        assert_eq!(models.len(), 1, "exactly one memo expected");
        assert_eq!(models[0]["type"], "note");
        models[0].clone()
    }
    unsafe fn create_note(&self, text: &str) -> NSRect {
        choose(&self.root, 4);
        self.click(self.point(200., 450.), 1);
        let initial = rect(&self.note());
        replace_text(&self.window, text);
        initial
    }
    unsafe fn edit(&self) {
        choose(&self.root, 1);
        let r = rect(&self.note());
        self.click(
            self.point(
                r.origin.x + r.size.width / 2.,
                r.origin.y + r.size.height / 2.,
            ),
            2,
        );
        self.assert_in_place(true);
    }
    /// The editor covers the stored memo exactly and hides its drawing.
    unsafe fn assert_in_place(&self, reopened: bool) {
        let editor = text_editor(&self.window);
        let note = self.note();
        let r: NSRect = msg_send![&*self.pdf,convertRect:rect(&note),fromPage:&*self.page];
        let r = self.overlay.convertRect_fromView(r, Some(&self.pdf));
        let e = editor.frame();
        assert!(
            (e.origin.x - r.origin.x).abs() < 0.5
                && (top(e) - top(r)).abs() < 0.5
                && (!reopened || (e.size.width - r.size.width).abs() < 0.5),
            "editor {e:?} is not on memo {r:?}"
        );
        assert!(!self.displayed(), "stored memo still drawn under editor");
    }
    unsafe fn displayed(&self) -> bool {
        let all: Retained<objc2_foundation::NSArray<AnyObject>> =
            msg_send![&*self.page, annotations];
        all.iter().any(|a| {
            let shown: bool = msg_send![&*a, shouldDisplay];
            shown
        })
    }
    unsafe fn edit_sized(&self) {
        choose(&self.root, 1);
        let r = rect(&self.note());
        self.click(
            self.point(
                r.origin.x + r.size.width / 2.,
                r.origin.y + r.size.height / 2.,
            ),
            2,
        );
        self.assert_in_place(true);
    }
    unsafe fn marquee(&self, r: NSRect) {
        choose(&self.root, 10);
        let a = self.point(r.origin.x, r.origin.y);
        let b = self.point(r.origin.x + r.size.width, r.origin.y + r.size.height);
        mouse(
            &self.overlay,
            &self.window,
            NSEventType::LeftMouseDown,
            a,
            1,
        );
        mouse(
            &self.overlay,
            &self.window,
            NSEventType::LeftMouseDragged,
            b,
            1,
        );
        mouse(&self.overlay, &self.window, NSEventType::LeftMouseUp, b, 1);
    }
    unsafe fn roundtrip(&self) {
        let ok: bool = msg_send![&*self.document,writeToURL:&*self.url];
        assert!(ok);
        let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
        let reopened: Retained<AnyObject> = msg_send![allocation,initWithURL:&*self.url];
        pdf_annotations::validate_roundtrip(&reopened, &self.commands.borrow()).unwrap();
        assert_eq!(
            pdf_annotations::read_page(&reopened, 0),
            pdf_annotations::read_page(&self.document, 0)
        );
    }
}
unsafe fn post(window: &NSWindow, kind: NSEventType, location: NSPoint) {
    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, location, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
    window.postEvent_atStart(&event, false);
}
unsafe fn empty_and_resize() {
    // A fresh memo closed without text vanishes and leaves no undo step.
    let f = Fixture::new();
    f.create_note("");
    assert!(pdf_annotations::commit_pending(&f.root));
    assert!(pdf_annotations::read_page(&f.document, 0).is_empty());
    choose(&f.root, 7);
    assert!(pdf_annotations::read_page(&f.document, 0).is_empty());
    f.roundtrip();
    // The editor's bottom-right handle sets a width that survives completion
    // and reopening; the text rewraps and the height never clips it.
    let f = Fixture::new();
    // Queued events keep window coordinates only for an on-screen origin.
    if let Some(screen) = f.window.screen() {
        let v = screen.visibleFrame();
        f.window
            .setFrameTopLeftPoint(NSPoint::new(v.origin.x, v.origin.y + v.size.height));
    }
    let text = "가나다라마바사아자차카타파하".repeat(3);
    f.create_note(&text);
    let editor = text_editor(&f.window);
    let frame = editor.frame();
    let corner = editor.superview().unwrap().convertPoint_toView(
        NSPoint::new(frame.origin.x + frame.size.width - 3., frame.origin.y + 3.),
        None,
    );
    let target = NSPoint::new(corner.x - 80., corner.y - 60.);
    post(&f.window, NSEventType::LeftMouseDragged, target);
    post(&f.window, NSEventType::LeftMouseUp, target);
    let down = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::LeftMouseDown, corner, NSEventModifierFlags::empty(), 0., f.window.windowNumber(), None, 0, 1, 1.).unwrap();
    let _: () = msg_send![&*editor,mouseDown:&*down];
    let resized = editor.frame();
    assert!(
        (resized.size.width - (frame.size.width - 80.)).abs() < 1.,
        "handle drag did not narrow editor: {frame:?} -> {resized:?}"
    );
    assert!(
        (top(resized) - top(frame)).abs() < 0.001,
        "handle drag moved the top"
    );
    assert!(pdf_annotations::commit_pending(&f.root));
    let sized = f.note();
    let r = rect(&sized);
    assert_eq!(sized["sized"], true);
    assert!(
        (r.size.width - (frame.size.width - 80.) / 0.8).abs() < 1. && r.size.height > 60.,
        "sized memo lost chosen width or clipped text: {r:?}"
    );
    f.roundtrip();
    f.edit_sized();
    assert!(pdf_annotations::commit_pending(&f.root));
    assert_eq!(f.note(), sized, "reopening changed a sized memo");
    choose(&f.root, 7);
    assert!((rect(&f.note()).size.width - r.size.width).abs() > 1.);
    choose(&f.root, 8);
    // The selection tool's corner handle also fixes the width and keeps text whole.
    choose(&f.root, 1);
    let start = f.point(r.origin.x + r.size.width - 1., r.origin.y + 1.);
    let end = f.point(
        r.origin.x + r.size.width + 100.,
        r.origin.y + r.size.height - 5.,
    );
    mouse(&f.overlay, &f.window, NSEventType::LeftMouseDown, start, 1);
    mouse(&f.overlay, &f.window, NSEventType::LeftMouseDragged, end, 1);
    mouse(&f.overlay, &f.window, NSEventType::LeftMouseUp, end, 1);
    let wide = f.note();
    let w = rect(&wide);
    assert_eq!(wide["sized"], true);
    assert!((w.size.width - (r.size.width + 100.)).abs() < 1., "{w:?}");
    assert!(w.size.height >= 20., "selection resize clipped text: {w:?}");
    assert!((top(w) - top(r)).abs() < 0.001);
    f.edit_sized();
    assert!(pdf_annotations::commit_pending(&f.root));
    assert_eq!(rect(&f.note()).size.width, w.size.width);
    f.roundtrip();
}
fn main() {
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            let _app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            let f = Fixture::new();
            let initial = f.create_note("ABC");
            f.assert_in_place(false);
            assert!(pdf_annotations::commit_pending(&f.root));
            assert!(f.displayed(), "memo stayed hidden after editing");
            let short = f.note();
            let compact = rect(&short);
            assert!(
                compact.size.width < 90. && compact.size.height < 40.,
                "short note still has oversized bounds: {:?}",
                compact
            );
            assert_anchor(initial, compact);
            // A small rectangle around the visible text must now contain the note.
            let small = NSRect::new(
                NSPoint::new(initial.origin.x - 2., top(initial) - 40.),
                NSSize::new(94., 42.),
            );
            f.marquee(small);
            key(&f.window, "\u{8}", 51);
            assert!(
                pdf_annotations::read_page(&f.document, 0).is_empty(),
                "compact marquee failed to select short memo"
            );
            choose(&f.root, 7);
            assert_eq!(f.note(), short, "delete undo lost text, geometry, or UUID");
            f.roundtrip();
            // Editing adds lines beyond the old 80pt editor without clipping stored bounds.
            f.edit();
            let editing_height = text_editor(&f.window).frame().size.height;
            let multiline =
                "첫째 줄\n둘째 줄\n셋째 줄\n넷째 줄\n다섯째 줄\n여섯째 줄\n일곱째 줄\n여덟째 줄";
            replace_text(&f.window, multiline);
            assert!(
                text_editor(&f.window).frame().size.height > editing_height,
                "live multiline editor did not grow vertically"
            );
            choose(&f.root, 1);
            let tall = f.note();
            let tall_rect = rect(&tall);
            assert_eq!(tall["text"], multiline);
            assert!(
                tall_rect.size.height > 100.,
                "multiline note did not grow vertically: {:?}",
                tall_rect
            );
            assert_anchor(compact, tall_rect);
            f.roundtrip();
            choose(&f.root, 7);
            assert_eq!(
                f.note(),
                short,
                "edit undo lost prior text or fitted bounds"
            );
            choose(&f.root, 8);
            assert_eq!(f.note(), tall, "edit redo lost new text or fitted bounds");
            // Text deletion edits characters and refitting shrinks again on completion.
            f.edit();
            replace_text(&f.window, "가나");
            key(&f.window, "\u{7f}", 51);
            assert_eq!(text_editor(&f.window).string().to_string(), "가");
            assert_eq!(pdf_annotations::read_page(&f.document, 0).len(), 1);
            choose(&f.root, 1);
            let tiny = f.note();
            let tiny_rect = rect(&tiny);
            assert_eq!(tiny["text"], "가");
            assert!(tiny_rect.size.width < 45. && tiny_rect.size.height < 40.);
            assert_anchor(tall_rect, tiny_rect);
            f.roundtrip();
            // Geometry admission failure must keep the live editor and last accepted PDF.
            f.edit();
            replace_text(&f.window, "다시 작성한 메모");
            let accepted = f.note();
            let commands = f.commands.borrow().len();
            f.reject.set(true);
            assert!(!pdf_annotations::commit_pending(&f.root));
            assert_eq!(
                f.note(),
                accepted,
                "rejected fit changed accepted PDF contents"
            );
            assert_eq!(f.commands.borrow().len(), commands);
            assert_eq!(
                text_editor(&f.window).string().to_string(),
                "다시 작성한 메모"
            );
            f.reject.set(false);
            assert!(pdf_annotations::commit_pending(&f.root));
            assert_eq!(f.note()["text"], "다시 작성한 메모");
            f.roundtrip();
            // Surrounding whitespace is kept; a memo emptied of text is deleted
            // and one undo restores it.
            f.edit();
            replace_text(&f.window, "  가 나  ");
            assert!(pdf_annotations::commit_pending(&f.root));
            assert_eq!(f.note()["text"], "  가 나  ");
            let spaced = f.note();
            for text in ["", "\n", "   "] {
                f.edit();
                replace_text(&f.window, text);
                assert!(pdf_annotations::commit_pending(&f.root));
                assert!(
                    pdf_annotations::read_page(&f.document, 0).is_empty(),
                    "emptied memo {text:?} left an annotation"
                );
                f.roundtrip();
                choose(&f.root, 7);
                assert_eq!(f.note(), spaced, "undo did not restore emptied memo");
            }
            f.roundtrip();
            // Word wrapping and unbroken runs retain contents and stable geometry
            // when merely reopening the memo editor and completing it again.
            for content in [
                "한글 메모에 긴 문장을 입력하면 여러 줄로 자동 줄바꿈됩니다. ".repeat(8),
                "가나다라마바사아자차카타파하".repeat(12),
            ] {
                f.edit();
                replace_text(&f.window, &content);
                assert!(pdf_annotations::commit_pending(&f.root));
                let wrapped = f.note();
                let r = rect(&wrapped);
                assert_eq!(wrapped["text"], content);
                assert!(
                    r.size.height > 80. && r.size.width <= 221.,
                    "long note did not wrap within edit width: {:?}",
                    r
                );
                f.roundtrip();
                f.edit();
                assert!(pdf_annotations::commit_pending(&f.root));
                assert_eq!(
                    f.note(),
                    wrapped,
                    "reopening a note changed fitted geometry"
                );
            }
            empty_and_resize();
            for rotation in [0isize, 90, 180, 270] {
                for scale in [0.5, 0.8] {
                    let f = Fixture::new();
                    f.layout(rotation, scale);
                    let initial = f.create_note("짧은 메모");
                    choose(&f.root, 1);
                    let note = f.note();
                    let r = rect(&note);
                    assert!(
                        r.size.width < 120. && r.size.height < 40.,
                        "rotation {rotation} scale {scale}: {:?}",
                        r
                    );
                    assert_anchor(initial, r);
                    f.marquee(NSRect::new(
                        NSPoint::new(r.origin.x - 2., r.origin.y - 2.),
                        NSSize::new(r.size.width + 4., r.size.height + 4.),
                    ));
                    key(&f.window, "\u{f728}", 117);
                    assert!(
                        pdf_annotations::read_page(&f.document, 0).is_empty(),
                        "compact marquee at rotation {rotation} scale {scale}"
                    );
                    choose(&f.root, 7);
                    assert_eq!(f.note(), note);
                    f.roundtrip();
                }
            }
            println!("PASS PDF note fit: empty memo removal, editor/selection resize handles, short text bounds + stable top-left anchor, compact marquee delete/undo UUID, Korean multiline height, native character deletion and shrink, rejected fit retains editor/data, whitespace content, 4 rotations at 50/80%, PDF save/reopen");
        });
    }
}
