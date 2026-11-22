//! Native image paste, wrapping, deletion, undo and read-only regression checks.
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/editor.rs"]
#[allow(dead_code)]
mod editor;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "../src/pdf_save.rs"]
#[allow(dead_code)]
mod pdf_save;
#[path = "../src/pdf_session.rs"]
#[allow(dead_code)]
mod pdf_session;
use objc2::{class, msg_send, rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSData, NSPoint, NSRange, NSRect, NSSize, NSString};
use std::path::PathBuf;
fn import(bytes: &[u8], _: &str) -> Result<(String, PathBuf), String> {
    let base = PathBuf::from(std::env::var("STOLOGY_IMAGE_TEST_DIR").unwrap());
    std::fs::create_dir_all(base.join("이미지")).unwrap();
    std::fs::write(base.join("이미지/test.png"), bytes).unwrap();
    Ok(("이미지/test.png".into(), base))
}
unsafe fn image(text: &NSTextView) -> Retained<NSView> {
    text.subviews()
        .iter()
        .find(|v| {
            let yes: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownImage)];
            yes
        })
        .expect("image overlay")
        .clone()
}
unsafe fn drag_body(view: &NSView, window: &NSWindow, dx: f64, dy: f64) {
    let center = view.convertPoint_toView(view.bounds().mid(), None);
    let parent = view.superview().unwrap().downcast::<NSTextView>().unwrap();
    let before = parent.string().to_string();
    let mut predicted = String::new();
    let mut predicted_frame = NSRect::ZERO;
    let mut wrap_rects = Vec::<NSRect>::new();
    for (kind, progress) in [(NSEventType::LeftMouseDown, 0.), (NSEventType::LeftMouseDragged, 0.5),
                             (NSEventType::LeftMouseDragged, 1.), (NSEventType::LeftMouseUp, 1.)] {
        let point = NSPoint::new(center.x + dx * progress, center.y - dy * progress);
        let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
        match kind {
            NSEventType::LeftMouseDown => view.mouseDown(&event),
            NSEventType::LeftMouseDragged => {
                view.mouseDragged(&event);
                assert_eq!(parent.string().to_string(), before);
                let preview = parent.subviews().into_iter().find_map(|child| child.downcast::<NSTextView>().ok()).expect("live preview before mouse-up");
                predicted = preview.string().to_string();
                predicted_frame = image(&preview).frame();
                wrap_rects = preview.textContainer().unwrap().exclusionPaths().iter().map(|p| p.bounds()).collect();
                assert_ne!(predicted, before);
            }
            _ => {
                view.mouseUp(&event);
                assert_eq!(parent.string().to_string(), predicted);
                assert_eq!(image(&parent).frame(), predicted_frame);
                let final_wrap: Vec<_> = parent.textContainer().unwrap().exclusionPaths().iter().map(|p| p.bounds()).collect();
                assert_eq!(wrap_rects, final_wrap);
                assert!(parent.subviews().iter().all(|v| v.downcast_ref::<NSTextView>().is_none()));
            }
        }
    }
}
fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 600.));
        let (scroll, text) = editor::make_editor(frame);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setReleasedWhenClosed(false);
        window.setContentView(Some(&scroll));
        window.makeFirstResponder(Some(&text));
        editor::set_image_importer(import);
        let png = std::fs::read(std::env::var("STOLOGY_IMAGE_FIXTURE").unwrap()).unwrap();
        let board = NSPasteboard::generalPasteboard();
        // Preserve the general pasteboard across this native test, including image representations.
        let saved: Vec<_> = board
            .types()
            .into_iter()
            .flat_map(|types| types.to_vec())
            .filter_map(|t| board.dataForType(&t).map(|d| (t, d)))
            .collect();
        board.clearContents();
        board.setData_forType(Some(&NSData::with_bytes(&png)), NSPasteboardTypePNG);
        text.setString(&NSString::from_str("앞 문단\n\n뒷 문단"));
        text.setSelectedRange(NSRange::new(6, 0));
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("붙여넣기"),
            Some(objc2::sel!(paste:)),
            &NSString::from_str("v"),
        );
        let enabled: bool = msg_send![&*text,validateUserInterfaceItem:&*item];
        assert!(enabled, "image-only clipboard must enable Cmd+V");
        let _: () = msg_send![&*text,paste:std::ptr::null::<objc2::runtime::AnyObject>()];
        board.clearContents();
        for (t, d) in saved {
            board.setData_forType(Some(&d), &t);
        }
        let pasted = text.string().to_string();
        assert!(pasted.contains("이미지/test.png"));
        assert!(pasted.contains("앞 문단"));
        assert!(pasted.contains("뒷 문단"));
        assert_eq!(text.textContainer().unwrap().exclusionPaths().len(), 1);
        assert!(image(&text).frame().size.width > 200.);
        let original=image(&text);
        drag_body(&original, &window, 800., 0.);
        assert!(text.string().to_string().contains("x=1.000"));
        text.undoManager().unwrap().removeAllActions();
        let right = image(&text);
        assert!(right.frame().origin.x > 300.);
        for (key, characters) in [(51, "\u{7f}"), (117, "\u{f728}")] {
            let view = image(&text);
            let point = view.convertPoint_toView(view.bounds().mid(), None);
            for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
                if kind == NSEventType::LeftMouseDown { view.mouseDown(&event); } else { view.mouseUp(&event); }
            }
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0., window.windowNumber(), None,
                &NSString::from_str(characters), &NSString::from_str(characters), false, key).unwrap();
            text.keyDown(&event);
            assert!(!text.string().to_string().contains("!["));
            text.undoManager().unwrap().undo();
            editor::style_with_base(&text, None);
            assert!(text.string().to_string().contains("!["));
        }
        let right = image(&text);
        let _: () = msg_send![&*right,deleteImage:std::ptr::null::<objc2::runtime::AnyObject>()];
        assert!(!text.string().to_string().contains("!["));
        assert_eq!(text.textContainer().unwrap().exclusionPaths().len(), 0);
        let undo = text.undoManager().unwrap();
        undo.undo();
        editor::style_with_base(&text, None);
        assert!(text.string().to_string().contains("!["));
        image(&text);
        text.setEditable(false);
        let before = text.string().to_string();
        let _: () =
            msg_send![&*image(&text),deleteImage:std::ptr::null::<objc2::runtime::AnyObject>()];
        assert_eq!(text.string().to_string(), before);
        text.setEditable(true);
        let block=text.string().to_string().replace("wrap=1","wrap=0");
        text.setString(&NSString::from_str(&block));
        editor::style_with_base(&text,None);
        assert_eq!(text.textContainer().unwrap().exclusionPaths().len(), 0);
        assert!(text.string().to_string().contains("wrap=0"));
        // Verify actual TextKit line geometry, not just the presence of exclusion paths.
        let base = PathBuf::from(std::env::var("STOLOGY_IMAGE_TEST_DIR").unwrap());
        let source = format!(
            "![이미지](이미지/test.png \"stology:width=240;x=0.000;wrap=1\")\n\n{}",
            "이미지 옆에 이어지는 필기 문장입니다. ".repeat(40)
        );
        text.setString(&NSString::from_str(&source));
        editor::style_with_base(&text, Some(&base));
        let manager = text.textLayoutManager().unwrap();
        let content = manager.textContentManager().unwrap();
        let offset = source.find("이미지 옆").unwrap();
        let location = content
            .locationFromLocation_withOffset(
                &content.documentRange().location(),
                source[..offset].encode_utf16().count() as isize,
            )
            .unwrap();
        let fragment = manager.textLayoutFragmentForLocation(&location).unwrap();
        let line = fragment
            .textLineFragments()
            .objectAtIndex(0)
            .typographicBounds();
        let origin = text.textContainerOrigin();
        let img = image(&text).frame();
        println!(
            "image={img:?}, first text line={line:?}, fragment={:?}, origin={origin:?}",
            fragment.layoutFragmentFrame()
        );
        assert!(
            line.origin.x + fragment.layoutFragmentFrame().origin.x + origin.x
                >= img.origin.x + img.size.width,
            "text overlaps floating image"
        );
        // Real mouse events through the native overlay, followed by a fresh layout.
        text.undoManager().unwrap().removeAllActions();
        drag_body(&image(&text), &window, 40., 64.);
        let moved = text.string().to_string();
        assert!(moved.contains(";y=64"), "{moved}");
        assert_eq!(moved.split_once("\n\n").unwrap().1, source.split_once("\n\n").unwrap().1);
        text.setString(&NSString::from_str(&moved));
        editor::style_with_base(&text, Some(&base));
        assert!((image(&text).frame().origin.y - img.origin.y - 64.).abs() < 1.);
        text.undoManager().unwrap().removeAllActions();
        drag_body(&image(&text), &window, 0., -24.);
        assert!(text.string().to_string().contains(";y=40"));
        text.undoManager().unwrap().undo();
        assert_eq!(text.string().to_string(), moved);

        let paragraphs = format!("{}\n\n첫 문단\n\n둘째 문단\n\n셋째 문단", source.split_once("\n\n").unwrap().0);
        text.setString(&NSString::from_str(&paragraphs));
        editor::style_with_base(&text, Some(&base));
        text.undoManager().unwrap().removeAllActions();
        drag_body(&image(&text), &window, 0., 300.);
        let relocated = text.string().to_string();
        assert!(relocated.starts_with("첫 문단\n\n둘째 문단\n\n!["), "{relocated}");
        assert!(relocated.ends_with("\n\n셋째 문단"));
        text.undoManager().unwrap().undo();
        assert_eq!(text.string().to_string(), paragraphs);
        for protected in ["```rust\nlet a = 1;\n```", "| 항목 | 값 |\n|---|---|\n| 가 | 나 |"] {
            let before = format!("{}\n\n첫 문단\n\n{protected}", source.split_once("\n\n").unwrap().0);
            text.setString(&NSString::from_str(&before));
            editor::style_with_base(&text, Some(&base));
            drag_body(&image(&text), &window, 0., 280.);
            let after = text.string().to_string();
            assert!(after.starts_with("첫 문단\n\n!["), "{after}");
            assert!(after.ends_with(protected) && after.contains("wrap=0"), "{after}");
        }
        let multiple="![one](이미지/test.png \"stology:width=240;x=0;wrap=1\")\n\n![two](이미지/test.png \"stology:width=240;x=0;wrap=1\")\n\n필기";
        text.setString(&NSString::from_str(multiple));
        editor::style_with_base(&text, Some(&base));
        let mut frames: Vec<NSRect> = text
            .subviews()
            .iter()
            .filter(|v| {
                let yes: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownImage)];
                yes
            })
            .map(|v| v.frame())
            .collect();
        frames.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
        assert_eq!(frames.len(), 2);
        assert!(
            frames[1].origin.y >= frames[0].origin.y + frames[0].size.height,
            "consecutive images overlap"
        );
        editor::style_with_base(&text, Some(&base));
        let mut again: Vec<NSRect> = text
            .subviews()
            .iter()
            .filter(|v| {
                let yes: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownImage)];
                yes
            })
            .map(|v| v.frame())
            .collect();
        again.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y));
        assert_eq!(frames, again, "image layout drifts between ticks");
        // Cmd+A must not copy or remove image Markdown, even with Unicode and multiple images.
        let select_source="앞 😀 필기\n\n![one](이미지/test.png)\n\n중간 필기\n\n![two](이미지/test.png)\n\n끝 필기";
        text.setString(&NSString::from_str(select_source));
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style_with_base(&text, Some(&base));
        text.undoManager().unwrap().removeAllActions();
        let _: () = msg_send![&*text,selectAll:std::ptr::null::<objc2::runtime::AnyObject>()];
        let selected: Vec<_> = text
            .selectedRanges()
            .iter()
            .map(|v| v.rangeValue())
            .collect();
        assert_eq!(
            selected.len(),
            3,
            "Cmd+A should select three text runs around images"
        );
        let copy_backup: Vec<_> = board
            .types()
            .into_iter()
            .flat_map(|types| types.to_vec())
            .filter_map(|t| board.dataForType(&t).map(|d| (t, d)))
            .collect();
        let _: () = msg_send![&*text,copy:std::ptr::null::<objc2::runtime::AnyObject>()];
        let copied = board
            .stringForType(NSPasteboardTypeString)
            .unwrap()
            .to_string();
        board.clearContents();
        for (t, d) in copy_backup {
            board.setData_forType(Some(&d), &t);
        }
        assert!(!copied.contains("!["));
        assert!(!copied.contains("test.png"));
        assert_eq!(copied.matches("필기").count(), 3);
        let _: () = msg_send![&*text,deleteBackward:std::ptr::null::<objc2::runtime::AnyObject>()];
        let remaining = text.string().to_string();
        println!("Cmd+A deletion: {remaining:?}");
        assert!(remaining.contains("![one](이미지/test.png)"));
        assert!(remaining.contains("![two](이미지/test.png)"));
        assert!(!remaining.contains("필기"));
        assert_eq!(text.textContainer().unwrap().exclusionPaths().len(), 0);
        text.undoManager().unwrap().undo();
        editor::style_with_base(&text, Some(&base));
        assert_eq!(text.string().to_string(), select_source);
        let _: () = msg_send![&*text,selectAll:std::ptr::null::<objc2::runtime::AnyObject>()];
        let _: () = msg_send![&*text,insertText:&*NSString::from_str("새 필기"),replacementRange:NSRange::new(usize::MAX,0)];
        let replaced = text.string().to_string();
        println!("Cmd+A typing: {replaced:?}");
        assert!(replaced.contains("![one](이미지/test.png)"));
        assert!(replaced.contains("![two](이미지/test.png)"));
        assert!(!replaced.contains("앞"));
        assert!(!replaced.contains("중간"));
        assert!(!replaced.contains("끝"));
        text.setString(&NSString::from_str(select_source));
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style_with_base(&text, Some(&base));
        let _: () = msg_send![&*text,selectAll:std::ptr::null::<objc2::runtime::AnyObject>()];
        let _: () = msg_send![&*text,setMarkedText:&*NSString::from_str("ㅎ"),selectedRange:NSRange::new(1,0),replacementRange:NSRange::new(usize::MAX,0)];
        let _: () = msg_send![&*text,insertText:&*NSString::from_str("한글"),replacementRange:NSRange::new(usize::MAX,0)];
        let ime = text.string().to_string();
        assert!(ime.contains("한글"));
        assert!(ime.contains("![one]"));
        assert!(ime.contains("![two]"));
        assert!(!ime.contains("필기"));
        text.setString(&NSString::from_str("![only](이미지/test.png)"));
        text.setSelectedRange(NSRange::new(0, 0));
        let _: () = msg_send![&*text,selectAll:std::ptr::null::<objc2::runtime::AnyObject>()];
        let _: () = msg_send![&*text,deleteBackward:std::ptr::null::<objc2::runtime::AnyObject>()];
        let _: () = msg_send![&*text,deleteForward:std::ptr::null::<objc2::runtime::AnyObject>()];
        assert_eq!(text.string().to_string(), "![only](이미지/test.png)");
        let diagram_dir = base.join(".diagram");
        std::fs::create_dir_all(&diagram_dir).unwrap();
        std::fs::write(diagram_dir.join("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"80\"><rect width=\"120\" height=\"80\" fill=\"red\"/></svg>").unwrap();
        let diagram = "![도형](<.diagram/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.svg>)\n\n본문";
        for (key, characters) in [(51, "\u{7f}"), (117, "\u{f728}")] {
            text.setString(&NSString::from_str(diagram));
            editor::style_with_base(&text, Some(&base));
            text.undoManager().unwrap().removeAllActions();
            let view = image(&text);
            let point = view.convertPoint_toView(view.bounds().mid(), None);
            for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                    kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap();
                if kind == NSEventType::LeftMouseDown { view.mouseDown(&event); } else { view.mouseUp(&event); }
            }
            let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
                NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0., window.windowNumber(), None,
                &NSString::from_str(characters), &NSString::from_str(characters), false, key).unwrap();
            text.keyDown(&event);
            assert!(!text.string().to_string().contains(".diagram/"));
            assert!(text.string().to_string().contains("본문"));
            text.undoManager().unwrap().undo();
            assert_eq!(text.string().to_string(), diagram);
        }
        println!("PASS: paste / relative image / wrapping geometry / alignment / delete / undo / read-only / block");
    }
}
