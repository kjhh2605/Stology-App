//! Native marquee regression. Uses hidden windows and temporary PDFs only.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pdf-marquee-smoke
#[path = "../src/font.rs"]
mod font;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "support/controls.rs"]
#[allow(dead_code)]
mod controls;
use controls::{button, choose, mouse, key};
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
    MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSDate, NSPoint, NSRange, NSRect, NSRunLoop, NSSize, NSString, NSURL,
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    path::Path,
    rc::Rc,
};
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

const MARQUEE: isize = 10;

fn fixture(path: &Path) {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << >> /Contents 4 0 R /Annots [5 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Annot /Subtype /Text /Rect [120 410 130 420] /Contents (foreign note) >>",
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
unsafe fn page_to_window(pdf: &NSView, page: &AnyObject, x: f64, y: f64) -> NSPoint {
    let point: NSPoint = msg_send![pdf,convertPoint:NSPoint::new(x,y),fromPage:page];
    pdf.convertPoint_toView(point, None)
}
unsafe fn select_pair(
    root: &NSView,
    overlay: &NSView,
    window: &NSWindow,
    pdf: &NSView,
    page: &AnyObject,
) {
    choose(root, MARQUEE);
    assert_eq!(button(root, MARQUEE).state(), NSControlStateValueOn);
    let a = page_to_window(pdf, page, 95., 390.);
    let b = page_to_window(pdf, page, 280., 450.);
    mouse(overlay, window, NSEventType::LeftMouseDown, a);
    mouse(overlay, window, NSEventType::LeftMouseDragged, b);
    mouse(overlay, window, NSEventType::LeftMouseUp, b);
}
unsafe fn ids(document: &AnyObject) -> BTreeSet<String> {
    pdf_annotations::read_page(document, 0)
        .iter()
        .map(|v| v["id"].as_str().unwrap().to_owned())
        .collect()
}
unsafe fn assert_foreign(page: &AnyObject, app_count: usize) {
    let all: Retained<NSArray<AnyObject>> = msg_send![page, annotations];
    assert_eq!(all.len(), app_count + 1, "foreign annotation was removed");
    assert!(all.iter().any(|a| {
        let contents: Option<Retained<NSString>> = msg_send![&*a, contents];
        contents.is_some_and(|s| s.to_string() == "foreign note")
    }));
}

fn main() {
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            let mtm = MainThreadMarker::new().unwrap();
            let _app = NSApplication::sharedApplication(mtm);
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("marquee.pdf");
            fixture(&path);
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
            let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let document: Retained<AnyObject> = msg_send![allocated,initWithURL:&*url];
            let page: Retained<AnyObject> = msg_send![&*document,pageAtIndex:0usize];
            let pdf: Retained<NSView> = msg_send![class!(PDFView), new];
            let _: () = msg_send![&*pdf,setDocument:&*document];
            let _: () = msg_send![&*pdf,setAutoScales:false];
            let commands = Rc::new(RefCell::new(Vec::<Value>::new()));
            let reject = Rc::new(Cell::new(false));
            let recorded = commands.clone();
            let reject_edits = reject.clone();
            let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 700.));
            let root = pdf_annotations::create(
                &pdf,
                frame,
                Rc::new(move |command| {
                    if reject_edits.get() {
                        Err("injected journal rejection".into())
                    } else {
                        recorded.borrow_mut().push(command);
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
            let _: () = msg_send![&*pdf,setScaleFactor:0.6f64];
            let _: () = msg_send![&*pdf, layoutDocumentView];
            let overlay = root
                .subviews()
                .iter()
                .find(|view| {
                    let yes: bool =
                        msg_send![&**view,isKindOfClass:class!(StologyPDFAnnotationOverlay)];
                    yes
                })
                .unwrap();
            // The custom tooltip panel is verified through Aside's real mouse hover.
            // Hidden-window tests check one help source and accessible descriptions.
            for tag in [0, 1, MARQUEE, 6, 2, 3, 4, 5, 7, 8, 9] {
                let tool = button(&root, tag);
                assert_eq!(tool.title().to_string(), "");
                assert!(tool.image().is_some(), "missing icon {tag}");
                assert!(
                    tool.toolTip().is_none(),
                    "duplicate native tooltip on tool {tag}"
                );
                assert!(
                    NSAccessibility::accessibilityLabel(&*tool)
                        .is_some_and(|s| !s.to_string().is_empty()),
                    "missing AX label {tag}"
                );
                assert!(
                    NSAccessibility::accessibilityHelp(&*tool)
                        .is_some_and(|s| !s.to_string().is_empty()),
                    "missing AX help {tag}"
                );
            }
            let mut seed_commands = Vec::new();
            for (id, kind, x) in [
                ("inside-box", "box", 100.),
                ("inside-area", "area", 210.),
                // Only five points overlap the marquee: partial overlap is not containment.
                ("outside-box", "box", 275.),
            ] {
                let v = json!({"schema":1,"id":id,"page":0,"type":kind,"bounds":[x,400.0,60.0,40.0],"text":"","points":[],"quads":[]});
                let c = json!({"id":id,"page":0,"before":null,"after":v});
                pdf_annotations::apply(&document, &c).unwrap();
                seed_commands.push(c);
            }
            let all_ids = ids(&document);
            let outside_ids = BTreeSet::from(["outside-box".to_owned()]);
            // A marquee click/tiny jitter must neither mutate the document nor leave an item selected.
            choose(&root, MARQUEE);
            let point = page_to_window(&pdf, &page, 120., 420.);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, point);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, point);
            key(&window, "\u{8}", 51);
            assert!(commands.borrow().is_empty());
            assert_eq!(ids(&document), all_ids);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, point);
            let jitter = NSPoint::new(point.x + 0.5, point.y + 0.5);
            mouse(&overlay, &window, NSEventType::LeftMouseDragged, jitter);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, jitter);
            key(&window, "\u{8}", 51);
            assert!(commands.borrow().is_empty());
            assert_eq!(ids(&document), all_ids);
            // Both backward deletion encodings and forward delete remove one group.
            for (text, keycode) in [("\u{8}", 51), ("\u{7f}", 51), ("\u{f728}", 117)] {
                let before = commands.borrow().len();
                select_pair(&root, &overlay, &window, &pdf, &page);
                assert_eq!(
                    commands.borrow().len(),
                    before,
                    "marquee should not journal"
                );
                key(&window, text, keycode);
                assert_eq!(ids(&document), outside_ids, "group deletion {text:?}");
                assert_eq!(
                    commands.borrow().len(),
                    before + 1,
                    "one durable command per group"
                );
                assert_eq!(
                    commands.borrow().last().unwrap()["commands"]
                        .as_array()
                        .unwrap()
                        .len(),
                    2
                );
                assert_foreign(&page, 1);
                choose(&root, 7);
                assert_eq!(ids(&document), all_ids, "one undo must restore every UUID");
                choose(&root, 8);
                assert_eq!(
                    ids(&document),
                    outside_ids,
                    "one redo must remove every selected item"
                );
                choose(&root, 7);
                assert_eq!(ids(&document), all_ids);
            }
            // Toolbar trash follows the same path, including admission failure rollback.
            select_pair(&root, &overlay, &window, &pdf, &page);
            let before = commands.borrow().len();
            reject.set(true);
            choose(&root, 9);
            assert_eq!(commands.borrow().len(), before);
            assert_eq!(
                ids(&document),
                all_ids,
                "rejected group must not partially delete"
            );
            assert_foreign(&page, 3);
            reject.set(false);
            key(&window, "\u{8}", 51);
            assert_eq!(
                ids(&document),
                outside_ids,
                "selection must survive rejected delete"
            );
            choose(&root, 7);
            select_pair(&root, &overlay, &window, &pdf, &page);
            choose(&root, 9);
            assert_eq!(
                ids(&document),
                outside_ids,
                "trash must remove the selected group"
            );
            // Validate both the saved PDF and replay on an independent document.
            let final_commands: Vec<_> = seed_commands
                .iter()
                .chain(commands.borrow().iter())
                .cloned()
                .collect();
            let ok: bool = msg_send![&*document,writeToURL:&*url];
            assert!(ok);
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let reopened: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            pdf_annotations::validate_roundtrip(&reopened, &final_commands).unwrap();
            assert_eq!(ids(&reopened), outside_ids);
            let replay_path = temp.path().join("replay.pdf");
            fixture(&replay_path);
            let replay_url =
                NSURL::fileURLWithPath(&NSString::from_str(replay_path.to_str().unwrap()));
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let replay: Retained<AnyObject> = msg_send![allocation,initWithURL:&*replay_url];
            for command in &final_commands {
                pdf_annotations::apply(&replay, command).unwrap();
            }
            pdf_annotations::validate_roundtrip(&replay, &final_commands).unwrap();
            assert_eq!(ids(&replay), outside_ids, "journal replay differs from UI");
            // Marquee uses displayed coordinates at zoom and all right-angle rotations.
            choose(&root, 7);
            for rotation in [0isize, 90, 180, 270] {
                let _: () = msg_send![&*page,setRotation:rotation];
                for scale in [0.5f64, 0.8] {
                    let _: () = msg_send![&*pdf,setScaleFactor:scale];
                    let _: () = msg_send![&*pdf, layoutDocumentView];
                    select_pair(&root, &overlay, &window, &pdf, &page);
                    key(&window, "\u{f728}", 117);
                    assert_eq!(
                        ids(&document),
                        outside_ids,
                        "rotation {rotation} scale {scale}"
                    );
                    choose(&root, 7);
                    assert_eq!(ids(&document), all_ids);
                }
            }
            // Native memo text deletion remains text editing, even after multi-selection.
            let _: () = msg_send![&*page,setRotation:0isize];
            let _: () = msg_send![&*pdf,setScaleFactor:0.6f64];
            let _: () = msg_send![&*pdf, layoutDocumentView];
            choose(&root, 4);
            let p = page_to_window(&pdf, &page, 330., 520.);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, p);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, p);
            let responder = window.firstResponder().unwrap();
            let is_text: bool = msg_send![&*responder,isKindOfClass:class!(NSTextView)];
            assert!(is_text);
            let text = &*(&*responder as *const NSResponder as *const NSTextView);
            let _: () = msg_send![text,insertText:&*NSString::from_str("가나"),replacementRange:NSRange::new(0,0)];
            let _: () = msg_send![text,deleteBackward:std::ptr::null::<AnyObject>()];
            assert_eq!(text.string().to_string(), "가");
            assert_eq!(pdf_annotations::read_page(&document, 0).len(), 4);
            assert!(pdf_annotations::commit_pending(&root));
            assert_foreign(&page, 4);
            // A single screen-space marquee can span two simultaneously visible pages.
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let extra_document: Retained<AnyObject> =
                msg_send![allocation,initWithURL:&*replay_url];
            let extra_page: Retained<AnyObject> = msg_send![&*extra_document,pageAtIndex:0usize];
            let _: () = msg_send![&*document,insertPage:&*extra_page,atIndex:1usize];
            for (index, id, y) in [
                (0usize, "page-zero-bottom", 50.),
                (1usize, "page-one-top", 700.),
            ] {
                let v = json!({"schema":1,"id":id,"page":index,"type":"box","bounds":[100.0,y,60.0,40.0],"text":"","points":[],"quads":[]});
                pdf_annotations::apply(
                    &document,
                    &json!({"id":id,"page":index,"before":null,"after":v}),
                )
                .unwrap();
            }
            let _: () = msg_send![&*pdf,setDisplayMode:1isize];
            // Rebind after changing the fixture's page tree so PDFView rebuilds its layout.
            let _: () = msg_send![&*pdf,setDocument:std::ptr::null::<AnyObject>()];
            let _: () = msg_send![&*pdf,setDocument:&*document];
            let _: () = msg_send![&*pdf,setScaleFactor:0.35f64];
            let _: () = msg_send![&*pdf, layoutDocumentView];
            let _: () = msg_send![&*pdf,goToPage:&*page];
            NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.05));
            let visible: Retained<NSArray<AnyObject>> = msg_send![&*pdf, visiblePages];
            assert_eq!(
                visible.len(),
                2,
                "both pages must be visible for cross-page fixture"
            );
            choose(&root, MARQUEE);
            let a = page_to_window(&pdf, &page, 90., 100.);
            let b = page_to_window(&pdf, &extra_page, 180., 690.);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, a);
            mouse(&overlay, &window, NSEventType::LeftMouseDragged, b);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, b);
            let before = commands.borrow().len();
            key(&window, "\u{8}", 51);
            assert_eq!(commands.borrow().len(), before + 1);
            assert_eq!(
                commands.borrow().last().unwrap()["commands"]
                    .as_array()
                    .unwrap()
                    .len(),
                2
            );
            assert_eq!(pdf_annotations::read_page(&document, 0).len(), 4);
            assert!(pdf_annotations::read_page(&document, 1).is_empty());
            choose(&root, 7);
            assert!(ids(&document).contains("page-zero-bottom"));
            assert_eq!(
                pdf_annotations::read_page(&document, 1)[0]["id"],
                "page-one-top"
            );
            choose(&root, 8);
            assert!(!ids(&document).contains("page-zero-bottom"));
            assert!(pdf_annotations::read_page(&document, 1).is_empty());
            let ok: bool = msg_send![&*document,writeToURL:&*url];
            assert!(ok);
            let allocation: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let reopened: Retained<AnyObject> = msg_send![allocation,initWithURL:&*url];
            pdf_annotations::validate_roundtrip(
                &reopened,
                &[commands.borrow().last().unwrap().clone()],
            )
            .unwrap();
            assert_eq!(pdf_annotations::read_page(&reopened, 0).len(), 4);
            assert!(pdf_annotations::read_page(&reopened, 1).is_empty());
            println!("PASS PDF marquee: icon/help/AX + no duplicate native tooltip, click no-op, enclosed pair/partial overlap excluded, three delete keys + trash, atomic rejection rollback, group undo/redo UUID, foreign preservation, save/reopen + journal replay, 0/90/180/270° at 50/80%, native memo deletion, two visible pages selected/deleted/undone as one group");
        });
    }
}
