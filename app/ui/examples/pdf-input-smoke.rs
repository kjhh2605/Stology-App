//! Native PDF input regression. No application window is shown and no user data is touched.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pdf-input-smoke
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
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString, NSURL};
use serde_json::Value;
use std::{cell::RefCell, path::Path, rc::Rc};
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
unsafe fn page_to_window(pdf: &NSView, page: &AnyObject, x: f64, y: f64) -> NSPoint {
    let point: NSPoint = msg_send![pdf,convertPoint:NSPoint::new(x,y),fromPage:page];
    pdf.convertPoint_toView(point, None)
}
fn main() {
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            let mtm = MainThreadMarker::new().unwrap();
            let _app = NSApplication::sharedApplication(mtm);
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("input.pdf");
            fixture(&path);
            let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str().unwrap()));
            let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let document: Retained<AnyObject> = msg_send![allocated,initWithURL:&*url];
            let page: Retained<AnyObject> = msg_send![&*document,pageAtIndex:0usize];
            let pdf: Retained<NSView> = msg_send![class!(PDFView), new];
            let _: () = msg_send![&*pdf,setDocument:&*document];
            let _: () = msg_send![&*pdf,setAutoScales:false];
            let commands = Rc::new(RefCell::new(Vec::<Value>::new()));
            let recorded = commands.clone();
            let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 700.));
            let root = pdf_annotations::create(
                &pdf,
                frame,
                Rc::new(move |command| {
                    recorded.borrow_mut().push(command);
                    Ok(())
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
            for tag in [0, 1, 6, 2, 3, 4, 5, 7, 8, 9] {
                let tool = button(&root, tag);
                assert_eq!(tool.title().to_string(), "", "visible text on tool {tag}");
                assert!(tool.image().is_some(), "missing tool icon {tag}");
                assert!(
                    tool.toolTip().is_none(),
                    "duplicate native tooltip on tool {tag}"
                );
                assert!(
                    NSAccessibility::accessibilityHelp(&*tool)
                        .is_some_and(|s| !s.to_string().is_empty()),
                    "missing accessible help {tag}"
                );
                assert!(
                    NSAccessibility::accessibilityLabel(&*tool)
                        .is_some_and(|s| !s.to_string().is_empty()),
                    "missing accessible name {tag}"
                );
            }
            // A click, tiny pointer jitter, or save during an un-dragged press must not
            // create a PDF annotation, an undo step, or a recovery command at any zoom.
            for scale in [0.5, 1.0, 2.0, 4.0] {
                let _: () = msg_send![&*pdf,setScaleFactor:scale];
                let _: () = msg_send![&*pdf, layoutDocumentView];
                let local = NSPoint::new(
                    pdf.bounds().size.width / 2.0,
                    pdf.bounds().size.height / 2.0,
                );
                let hit: Option<Retained<AnyObject>> =
                    msg_send![&*pdf,pageForPoint:local,nearest:false];
                assert!(hit.is_some(), "zoom fixture center must hit PDF page");
                let p = pdf.convertPoint_toView(local, None);
                for tag in [3, 2] {
                    choose(&root, tag);
                    let before = commands.borrow().len();
                    mouse(&overlay, &window, NSEventType::LeftMouseDown, p);
                    mouse(&overlay, &window, NSEventType::LeftMouseUp, p);
                    assert_eq!(
                        commands.borrow().len(),
                        before,
                        "click emitted command at scale {scale}"
                    );
                    assert!(
                        pdf_annotations::read_page(&document, 0).is_empty(),
                        "click created annotation"
                    );
                    mouse(&overlay, &window, NSEventType::LeftMouseDown, p);
                    let jitter = NSPoint::new(p.x + 0.5, p.y + 0.5);
                    mouse(&overlay, &window, NSEventType::LeftMouseDragged, jitter);
                    assert!(pdf_annotations::commit_pending(&root));
                    mouse(&overlay, &window, NSEventType::LeftMouseUp, jitter);
                    assert_eq!(
                        commands.borrow().len(),
                        before,
                        "jitter/save emitted command at scale {scale}"
                    );
                    mouse(&overlay, &window, NSEventType::LeftMouseDown, p);
                    assert!(pdf_annotations::commit_pending(&root));
                    mouse(&overlay, &window, NSEventType::LeftMouseUp, p);
                    assert_eq!(
                        commands.borrow().len(),
                        before,
                        "save during click emitted command"
                    );
                }
            }
            let _: () = msg_send![&*pdf,setScaleFactor:1.0f64];
            let _: () = msg_send![&*pdf, layoutDocumentView];
            let a = page_to_window(&pdf, &page, 100., 500.);
            let b = page_to_window(&pdf, &page, 250., 420.);
            choose(&root, 3);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, a);
            mouse(&overlay, &window, NSEventType::LeftMouseDragged, b);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, b);
            assert_eq!(
                pdf_annotations::read_page(&document, 0).len(),
                1,
                "real box drag missing"
            );
            let id = pdf_annotations::read_page(&document, 0)[0]["id"].clone();
            // Test both keyboards' backward deletion values, forward-delete, and native
            // responder actions. Undo restores the same annotation UUID every time.
            for (text, keycode) in [("\u{8}", 51), ("\u{7f}", 51), ("\u{f728}", 117)] {
                key(&window, text, keycode);
                assert!(
                    pdf_annotations::read_page(&document, 0).is_empty(),
                    "{text:?} did not delete selected annotation"
                );
                choose(&root, 7);
                assert_eq!(pdf_annotations::read_page(&document, 0)[0]["id"], id);
                choose(&root, 1);
                let center = page_to_window(&pdf, &page, 170., 460.);
                mouse(&overlay, &window, NSEventType::LeftMouseDown, center);
                mouse(&overlay, &window, NSEventType::LeftMouseUp, center);
            }
            let responder = window.firstResponder().unwrap();
            let _: () = msg_send![&*responder,deleteForward:std::ptr::null::<AnyObject>()];
            assert!(
                pdf_annotations::read_page(&document, 0).is_empty(),
                "deleteForward action did not delete"
            );
            choose(&root, 7);
            assert_eq!(pdf_annotations::read_page(&document, 0)[0]["id"], id);
            // Text deletion in a native memo editor must edit the text, not delete its annotation.
            choose(&root, 4);
            let note_point = page_to_window(&pdf, &page, 350., 500.);
            mouse(&overlay, &window, NSEventType::LeftMouseDown, note_point);
            mouse(&overlay, &window, NSEventType::LeftMouseUp, note_point);
            let responder = window.firstResponder().unwrap();
            let is_text: bool = msg_send![&*responder,isKindOfClass:class!(NSTextView)];
            assert!(is_text);
            let text = &*(&*responder as *const NSResponder as *const NSTextView);
            let _: () = msg_send![text,insertText:&*NSString::from_str("가나"),replacementRange:NSRange::new(0,0)];
            let _: () = msg_send![text,deleteBackward:std::ptr::null::<AnyObject>()];
            assert_eq!(text.string().to_string(), "가");
            assert_eq!(
                pdf_annotations::read_page(&document, 0).len(),
                2,
                "text deletion removed memo annotation"
            );
            assert!(pdf_annotations::commit_pending(&root));
            let annotations = pdf_annotations::read_page(&document, 0);
            assert_eq!(
                annotations.iter().find(|v| v["type"] == "note").unwrap()["text"],
                "가"
            );
            println!("PASS PDF input: icon/help/AX toolbar, box/area click+jitter+flush no-op at 50–400%, actual drag, backward/forward delete + undo UUID, native memo text deletion preserved");
        });
    }
}
