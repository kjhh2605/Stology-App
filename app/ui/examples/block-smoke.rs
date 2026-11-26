//! Native block layout checks: attachment rows, side blocks and block moves.
//! cargo run --manifest-path app/ui/Cargo.toml --example block-smoke
#[path = "../src/editor.rs"]
#[allow(dead_code)]
mod editor;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "../src/pdf_save.rs"]
#[allow(dead_code)]
mod pdf_save;
#[path = "../src/pdf_session.rs"]
#[allow(dead_code)]
mod pdf_session;
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
use objc2::{class, msg_send, rc::Retained, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSDictionary, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString};
use std::path::Path;

unsafe fn overlays(text: &NSTextView, class: &objc2::runtime::AnyClass) -> Vec<NSRect> {
    let mut frames: Vec<_> = text.subviews().iter().filter(|v| v.isKindOfClass(class)).map(|v| v.frame()).collect();
    frames.sort_by(|a, b| a.origin.y.total_cmp(&b.origin.y).then(a.origin.x.total_cmp(&b.origin.x)));
    frames
}
unsafe fn images(text: &NSTextView) -> Vec<NSRect> {
    overlays(text, class!(StologyInlineMarkdownImage))
}
unsafe fn tables(text: &NSTextView) -> Vec<NSRect> {
    overlays(text, class!(StologyInlineMarkdownTable))
}
unsafe fn rect_at(text: &NSTextView, window: &NSWindow, needle: &str) -> NSRect {
    let source = text.string().to_string();
    let at = source[..source.find(needle).unwrap_or_else(|| panic!("{needle:?} in {source:?}"))].encode_utf16().count();
    let screen: NSRect = msg_send![text, firstRectForCharacterRange:NSRange::new(at, 1), actualRange:std::ptr::null_mut::<NSRange>()];
    text.convertRect_fromView(window.convertRectFromScreen(screen), None)
}
/// Open another note, as the app does: no undo history and scrolled to the top.
unsafe fn show(text: &NSTextView, base: &Path, source: &str) {
    text.setString(&NSString::from_str(source));
    text.undoManager().unwrap().removeAllActions();
    text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
    editor::style_with_base(text, Some(base));
    text.sizeToFit();
    text.scrollPoint(NSPoint::ZERO);
}
unsafe fn mouse(window: &NSWindow, kind: NSEventType, point: NSPoint) -> Retained<NSEvent> {
    NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 0, 1, 1.).unwrap()
}
/// Press at `from`, pass through `via` and release at the last point, the
/// way AppKit delivers a drag to the view under the press.
unsafe fn drag(view: &NSView, text: &NSTextView, window: &NSWindow, from: NSPoint, via: &[NSPoint]) {
    let to_window = |point: NSPoint| text.convertPoint_toView(point, None);
    let release = mouse(window, NSEventType::LeftMouseUp, to_window(*via.last().unwrap_or(&from)));
    // A text view tracks its own clicks and reads the release from the queue.
    window.discardEventsMatchingMask_beforeEvent(NSEventMask::LeftMouseUp | NSEventMask::LeftMouseDragged, None);
    if via.is_empty() && view.isKindOfClass(class!(NSTextView)) {
        window.postEvent_atStart(&release, true);
    }
    let _: () = msg_send![view, mouseDown:&*mouse(window, NSEventType::LeftMouseDown, to_window(from))];
    for point in via {
        let _: () = msg_send![view, mouseDragged:&*mouse(window, NSEventType::LeftMouseDragged, to_window(*point))];
    }
    let _: () = msg_send![view, mouseUp:&*release];
}
unsafe fn click(text: &NSTextView, window: &NSWindow, point: NSPoint) {
    drag(text, text, window, point, &[]);
}
unsafe fn key(text: &NSTextView, window: &NSWindow, code: u16, characters: &str) {
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::ZERO, NSEventModifierFlags::empty(), 0., window.windowNumber(), None,
        &NSString::from_str(characters), &NSString::from_str(characters), false, code).unwrap();
    text.keyDown(&event);
}
unsafe fn insert(text: &NSTextView, value: &str) {
    let selection: NSRange = msg_send![text, selectedRange];
    let _: () = msg_send![text, insertText:&*NSString::from_str(value), replacementRange:selection];
}
unsafe fn caret(text: &NSTextView, window: &NSWindow) -> NSRect {
    let selection: NSRange = msg_send![text, selectedRange];
    let screen: NSRect = msg_send![text, firstRectForCharacterRange:selection, actualRange:std::ptr::null_mut::<NSRange>()];
    text.convertRect_fromView(window.convertRectFromScreen(screen), None)
}
unsafe fn image_view(text: &NSTextView) -> Retained<NSView> {
    text.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyInlineMarkdownImage))).expect("image overlay")
}
/// Hover the block that holds `needle` and return its handle.
unsafe fn handle_at(text: &NSTextView, window: &NSWindow, point: NSPoint) -> Retained<NSView> {
    let _: () = msg_send![text, mouseMoved:&*mouse(window, NSEventType::MouseMoved, text.convertPoint_toView(point, None))];
    let handle = text.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyBlockHandle))).expect("block handle");
    assert!(!handle.isHidden(), "no handle for the block at {point:?}");
    handle
}
unsafe fn undo(text: &NSTextView, base: &Path) {
    text.undoManager().unwrap().undo();
    editor::style_with_base(text, Some(base));
}
unsafe fn mid(rect: NSRect) -> NSPoint {
    NSPoint::new(rect.origin.x + rect.size.width / 2., rect.origin.y + rect.size.height / 2.)
}
unsafe fn capture(scroll: &NSScrollView, name: &str) {
    let Ok(directory) = std::env::var("STOLOGY_BLOCK_CAPTURE") else { return };
    scroll.layoutSubtreeIfNeeded();
    let bitmap = scroll.bitmapImageRepForCachingDisplayInRect(scroll.bounds()).expect("offscreen bitmap");
    scroll.cacheDisplayInRect_toBitmapImageRep(scroll.bounds(), &bitmap);
    let png = bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()).expect("PNG export");
    let output = format!("{directory}/{name}.png");
    assert!(png.writeToFile_atomically(&NSString::from_str(&output), true));
    println!("CAPTURE: {output}");
}

fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 700.));
        let (scroll, text) = editor::make_editor(frame);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm), frame, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false);
        window.setReleasedWhenClosed(false);
        // Events queued for a window that was never shown come back measured
        // from the top of the screen. Put the window there so clicks land.
        if let Some(screen) = NSScreen::mainScreen(mtm) {
            window.setFrameTopLeftPoint(NSPoint::new(0., screen.frame().origin.y + screen.frame().size.height));
        }
        window.setContentView(Some(&scroll));
        window.makeFirstResponder(Some(&text));
        let base = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("a.svg"), r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><rect width="200" height="120" fill="#6556D9"/></svg>"##).unwrap();
        let image = |placement: &str| format!("![그림](<a.svg> \"stology:width=200;{placement}\")");
        let long = "옆 칸 안에서만 줄바꿈하는 긴 문단입니다. ".repeat(6);

        // Left attachment: one side block, the next block starts below the row.
        let source = format!("위 문단\n{}\n\n옆글 {long}\n아래 문단\n", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        capture(&scroll, "left-text");
        let picture = images(&text)[0];
        let side = rect_at(&text, &window, "옆글");
        let below = rect_at(&text, &window, "아래 문단");
        assert_eq!(picture.origin.x, 25.);
        assert!(side.origin.x >= picture.origin.x + picture.size.width + 20., "side text starts in the right column");
        assert!((side.origin.y - picture.origin.y).abs() < 8., "side text starts at the attachment's top");
        assert!(below.origin.y >= picture.origin.y + picture.size.height, "next block clears the attachment");
        assert!(below.origin.x < 40., "next block returns to the full width");

        // A short side block still ends the row below the attachment.
        let source = format!("{}\n\n짧은 옆글\n아래 문단\n", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        capture(&scroll, "left-short");
        let picture = images(&text)[0];
        let below = rect_at(&text, &window, "아래 문단");
        assert!(below.origin.y >= picture.origin.y + picture.size.height);
        assert!(below.origin.y < picture.origin.y + picture.size.height + 30., "no extra gap below the row: {below:?}");

        // Right attachment: starts at the right column's left edge, text ends before it.
        let source = format!("{}\n\n왼글 {long}\n아래 문단\n", image("x=1.000;wrap=1"));
        show(&text, base.path(), &source);
        capture(&scroll, "right-text");
        let picture = images(&text)[0];
        let side = rect_at(&text, &window, "왼글");
        assert!(picture.origin.x > 380. && picture.origin.x < 420., "right column starts at the middle: {picture:?}");
        assert!(side.origin.x < 40.);
        let tail = rect_at(&text, &window, "아래 문단");
        assert!(tail.origin.y >= picture.origin.y + picture.size.height);

        // Whole row: nothing beside, following text below.
        let source = format!("{}\n\n아래 문단\n", image("x=0.700;wrap=0"));
        show(&text, base.path(), &source);
        capture(&scroll, "full");
        let picture = images(&text)[0];
        let below = rect_at(&text, &window, "아래 문단");
        assert_eq!(picture.origin.x, 25.);
        assert!(below.origin.y >= picture.origin.y + picture.size.height && below.origin.x < 40.);

        // Two attachments share a row; the table starts at its column's left edge.
        let table = "| 가 | 나 |\n| --- | --- |\n| 1 | 2 |\n\n";
        let source = format!("{}\n\n{table}아래 문단\n", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        capture(&scroll, "image-table");
        let (picture, grid) = (images(&text)[0], tables(&text)[0]);
        let below = rect_at(&text, &window, "아래 문단");
        assert!((grid.origin.x - (picture.origin.x + picture.size.width + 24.)).abs() < 1.);
        assert!((grid.origin.y - picture.origin.y).abs() < 1.);
        assert!(below.origin.y >= picture.origin.y + picture.size.height && below.origin.y >= grid.origin.y + grid.size.height - 24.);

        // A table on the right with text on the left, then a code block beside an image.
        let right_table = "| <!--stology-grid {\"side\":\"right\",\"wrap\":true}-->가 | 나 |\n| --- | --- |\n| 1 | 2 |\n\n";
        let source = format!("{right_table}왼글 {long}\n\n{}\n\n```rust\nlet a = 1;\n```\n- 목록\n", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        capture(&scroll, "right-table-code");
        let grid = tables(&text)[0];
        assert!(grid.origin.x > 380. && grid.origin.x < 420.);
        assert!(rect_at(&text, &window, "왼글").origin.x < 40.);
        let picture = images(&text)[0];
        let code = rect_at(&text, &window, "let a");
        assert!(code.origin.x > picture.origin.x + picture.size.width + 24.);
        assert!(rect_at(&text, &window, "목록").origin.y >= picture.origin.y + picture.size.height);
        assert_eq!(text.string().to_string(), source, "layout never edits the note");

        // Dragging an image past the middle moves it to the right column and
        // the block after it comes up beside it. One undo restores the note.
        let source = format!("위 문단\n{}\n\n아래 문단\n", image("x=0.000;wrap=0"));
        show(&text, base.path(), &source);
        let view = image_view(&text);
        let start = mid(view.frame());
        let events = [(NSEventType::LeftMouseDown, 0.), (NSEventType::LeftMouseDragged, 200.), (NSEventType::LeftMouseDragged, 420.), (NSEventType::LeftMouseUp, 420.)];
        for (kind, dx) in events {
            let event = mouse(&window, kind, text.convertPoint_toView(NSPoint::new(start.x + dx, start.y), None));
            match kind {
                NSEventType::LeftMouseDown => view.mouseDown(&event),
                NSEventType::LeftMouseDragged => {
                    view.mouseDragged(&event);
                    assert_eq!(text.string().to_string(), source, "a drag only previews");
                }
                _ => view.mouseUp(&event),
            }
        }
        let moved = text.string().to_string();
        assert!(moved.contains("x=1.000;wrap=1") && !moved.contains(";y="), "{moved:?}");
        assert!(text.subviews().iter().all(|v| v.downcast_ref::<NSTextView>().is_none()), "preview removed");
        let picture = images(&text)[0];
        let beside = rect_at(&text, &window, "아래 문단");
        // The moved image stays selected, so its frame includes the selection margin.
        assert!(picture.origin.x > 380. && (beside.origin.y - picture.origin.y).abs() < 14. && beside.origin.x < 40., "{picture:?} {beside:?}");
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);

        // Dragging down moves the image between blocks; its separator goes with it.
        let source = format!("{}\n\n가 문단\n나 문단\n다 문단\n", image("x=0.000;wrap=0"));
        show(&text, base.path(), &source);
        let view = image_view(&text);
        let start = mid(view.frame());
        let target = rect_at(&text, &window, "다 문단").origin.y + 4.;
        let dy = target - view.frame().origin.y;
        for (kind, progress) in [(NSEventType::LeftMouseDown, 0.), (NSEventType::LeftMouseDragged, 0.5), (NSEventType::LeftMouseDragged, 1.), (NSEventType::LeftMouseUp, 1.)] {
            let event = mouse(&window, kind, text.convertPoint_toView(NSPoint::new(start.x, start.y + dy * progress), None));
            match kind {
                NSEventType::LeftMouseDown => view.mouseDown(&event),
                NSEventType::LeftMouseDragged => view.mouseDragged(&event),
                _ => view.mouseUp(&event),
            }
        }
        assert_eq!(text.string().to_string(), format!("가 문단\n나 문단\n{}\n\n다 문단\n", image("x=0.000;wrap=0")));
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);

        // The attachment menu sets the placement and marks the current one.
        let view = image_view(&text);
        let menu: Retained<NSMenu> = msg_send![&*view, menuForEvent:&*mouse(&window, NSEventType::RightMouseDown, NSPoint::ZERO)];
        let titles: Vec<_> = (0..menu.numberOfItems()).map(|i| menu.itemAtIndex(i).unwrap().title().to_string()).collect();
        assert_eq!(titles, ["한 줄 전체", "왼쪽에 배치", "오른쪽에 배치", "", "삭제"]);
        assert_eq!(menu.itemAtIndex(0).unwrap().state(), 1);
        menu.performActionForItemAtIndex(2);
        assert!(text.string().to_string().contains("x=1.000;wrap=1"));
        let view = image_view(&text);
        let menu: Retained<NSMenu> = msg_send![&*view, menuForEvent:&*mouse(&window, NSEventType::RightMouseDown, NSPoint::ZERO)];
        assert_eq!(menu.itemAtIndex(2).unwrap().state(), 1);
        menu.performActionForItemAtIndex(0);
        assert_eq!(text.string().to_string(), source);

        // Clicking the empty space beside a whole-row attachment opens one
        // side block there. The text that followed stays below the row.
        for block in [image("x=0.000;wrap=0") + "\n\n", table.to_owned()] {
            let source = format!("{block}아래 문단\n");
            show(&text, base.path(), &source);
            let frame = text.subviews().into_iter()
                .find(|v| v.isKindOfClass(class!(StologyInlineMarkdownImage)) || v.isKindOfClass(class!(StologyInlineMarkdownTable)))
                .unwrap().frame();
            click(&text, &window, NSPoint::new(frame.origin.x + frame.size.width + 120., frame.origin.y + 30.));
            insert(&text, "옆에 쓴 글");
            let written = text.string().to_string();
            assert!(written.ends_with("옆에 쓴 글\n아래 문단\n") && (written.contains("wrap=1") || written.contains("\"wrap\":true")), "{written:?}");
            let side = rect_at(&text, &window, "옆에 쓴 글");
            let below = rect_at(&text, &window, "아래 문단");
            assert!(side.origin.x > frame.origin.x + frame.size.width && side.origin.y < frame.origin.y + frame.size.height, "{side:?} beside {frame:?}");
            assert!(below.origin.x < 40. && below.origin.y >= frame.origin.y + frame.size.height - 24., "{below:?} below {frame:?}");
            // A second click in the filled column only moves the caret.
            click(&text, &window, NSPoint::new(frame.origin.x + frame.size.width + 120., frame.origin.y + frame.size.height - 12.));
            assert_eq!(text.string().to_string(), written);
            insert(&text, "!");
            assert!(text.string().to_string().contains("옆에 쓴 글!\n아래 문단"));
        }

        // The page margin beside an attachment is not a column: a click there
        // leaves the note alone.
        let source = format!("{}\n\n아래 문단\n", image("x=0.000;wrap=0"));
        show(&text, base.path(), &source);
        let picture = images(&text)[0];
        click(&text, &window, NSPoint::new(8., picture.origin.y + 30.));
        click(&text, &window, NSPoint::new(text.bounds().size.width - 6., picture.origin.y + 30.));
        assert_eq!(text.string().to_string(), source);

        // The cell menu's table actions move the whole table; its handles stay
        // on the table's right and bottom edges.
        let source = format!("{table}옆 문단\n아래 문단\n");
        show(&text, base.path(), &source);
        text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
        let place = |tag: isize| {
            let grid = text.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyInlineMarkdownTable))).unwrap();
            let cell = grid.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyMarkdownTableCell))).unwrap();
            let item = NSMenuItem::new(mtm);
            item.setTag(tag);
            let _: () = msg_send![&*cell, gridPlace:&*item];
        };
        place(2);
        let placed = text.string().to_string();
        assert!(placed.contains("\"side\":\"right\"") && placed.contains("\"wrap\":true"), "{placed:?}");
        let grid = tables(&text)[0];
        let beside = rect_at(&text, &window, "옆 문단");
        let below = rect_at(&text, &window, "아래 문단");
        assert!(grid.origin.x > 380. && beside.origin.x < 40. && (beside.origin.y - grid.origin.y).abs() < 12., "{grid:?} {beside:?}");
        assert!(below.origin.y >= grid.origin.y + grid.size.height - 24. && below.origin.x < 40., "{below:?} below {grid:?}");
        let view = text.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyInlineMarkdownTable))).unwrap();
        let handles: Vec<NSRect> = view.subviews().iter().filter(|v| v.isKindOfClass(class!(StologyTableHandle))).map(|v| v.frame()).collect();
        assert!(handles.len() == 2 && handles.iter().any(|h| h.origin.x > 200. && h.origin.y == 0.) && handles.iter().any(|h| h.origin.x == 0. && h.origin.y > 40.), "{handles:?}");
        capture(&scroll, "right-table");
        place(1);
        assert!(!text.string().to_string().contains("side") && text.string().to_string().contains("\"wrap\":true"));
        assert!(tables(&text)[0].origin.x == 25. && rect_at(&text, &window, "옆 문단").origin.x > 300.);
        place(0);
        assert!(!text.string().to_string().contains("wrap") && rect_at(&text, &window, "옆 문단").origin.x < 40.);
        for _ in 0..3 { text.undoManager().unwrap().undo(); }
        assert_eq!(text.string().to_string(), source);

        // The last line can sit beside an attachment. A click below the row
        // starts a paragraph under it.
        let source = format!("{}\n\n옆 글", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        let picture = images(&text)[0];
        click(&text, &window, NSPoint::new(80., picture.origin.y + picture.size.height + 20.));
        insert(&text, "아래에 쓴 글");
        let below = rect_at(&text, &window, "아래에 쓴 글");
        assert!(text.string().to_string().ends_with("옆 글\n아래에 쓴 글") && below.origin.x < 40. && below.origin.y >= picture.origin.y + picture.size.height, "{:?} {below:?}", text.string().to_string());

        // After a finished side line, the end-of-note caret is below the row.
        let source = format!("{}\n\n옆 글\n", image("x=0.000;wrap=1"));
        show(&text, base.path(), &source);
        let picture = images(&text)[0];
        let end = caret(&text, &window);
        assert!(end.origin.y >= picture.origin.y + picture.size.height && end.origin.x < 40., "{end:?} below {picture:?}");
        insert(&text, "끝");
        let typed = rect_at(&text, &window, "끝");
        assert!((typed.origin.y - end.origin.y).abs() < 2., "typing does not move the line: {typed:?} after {end:?}");

        // Arrow keys never leave the caret in an image's hidden lines.
        let source = format!("위 문단\n{}\n\n아래 문단\n", image("x=0.000;wrap=0"));
        show(&text, base.path(), &source);
        text.setSelectedRange(NSRange::new(1, 0));
        let _: () = msg_send![&*text, moveDown:std::ptr::null::<objc2::runtime::AnyObject>()];
        let selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(selection.location, source[..source.find("아래").unwrap()].encode_utf16().count());
        let _: () = msg_send![&*text, moveLeft:std::ptr::null::<objc2::runtime::AnyObject>()];
        let selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(selection.location, "위 문단".encode_utf16().count());

        // The handle appears beside the hovered block and moves it between
        // blocks; a click on it selects the block.
        let source = "가 문단\n- 나 목록\n  - 나의 하위\n다 문단\n라 문단\n";
        show(&text, base.path(), source);
        let item = rect_at(&text, &window, "나 목록");
        let handle = handle_at(&text, &window, NSPoint::new(item.origin.x + 30., item.origin.y + 8.));
        assert!(handle.frame().origin.x < item.origin.x && (handle.frame().origin.y - item.origin.y).abs() < 14., "{:?} beside {item:?}", handle.frame());
        capture(&scroll, "handle");
        let end = rect_at(&text, &window, "라 문단");
        drag(&handle, &text, &window, mid(handle.frame()), &[NSPoint::new(40., item.origin.y + 40.), NSPoint::new(40., end.origin.y + 2.)]);
        assert_eq!(text.string().to_string(), "가 문단\n다 문단\n- 나 목록\n  - 나의 하위\n라 문단\n", "a list item moves with its nested items");
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);
        let first = rect_at(&text, &window, "가 문단");
        let handle = handle_at(&text, &window, NSPoint::new(first.origin.x + 30., first.origin.y + 8.));
        drag(&handle, &text, &window, mid(handle.frame()), &[]);
        key(&text, &window, 51, "\u{7f}");
        assert_eq!(text.string().to_string(), "- 나 목록\n  - 나의 하위\n다 문단\n라 문단\n", "Backspace deletes the selected block");
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);

        // Dropping a block on an attachment's right edge puts it beside the attachment.
        let source = format!("{}\n\n가 문단\n나 문단\n", image("x=0.000;wrap=0"));
        show(&text, base.path(), &source);
        let picture = images(&text)[0];
        let line = rect_at(&text, &window, "나 문단");
        let handle = handle_at(&text, &window, NSPoint::new(line.origin.x + 30., line.origin.y + 8.));
        let edge = NSPoint::new(picture.origin.x + picture.size.width - 6., picture.origin.y + picture.size.height / 2.);
        if std::env::var("STOLOGY_BLOCK_CAPTURE").is_ok() {
            let to_window = |point: NSPoint| text.convertPoint_toView(point, None);
            let _: () = msg_send![&*handle, mouseDown:&*mouse(&window, NSEventType::LeftMouseDown, to_window(mid(handle.frame())))];
            let _: () = msg_send![&*handle, mouseDragged:&*mouse(&window, NSEventType::LeftMouseDragged, to_window(edge))];
            capture(&scroll, "drop-beside");
            let _: () = msg_send![&*handle, mouseDragged:&*mouse(&window, NSEventType::LeftMouseDragged, to_window(NSPoint::new(120., picture.origin.y - 2.)))];
            capture(&scroll, "drop-before");
            let _: () = msg_send![&*handle, mouseDragged:&*mouse(&window, NSEventType::LeftMouseDragged, to_window(mid(handle.frame())))];
            let _: () = msg_send![&*handle, mouseUp:&*mouse(&window, NSEventType::LeftMouseUp, to_window(mid(handle.frame())))];
            key(&text, &window, 53, "\u{1b}");
            assert_eq!(text.string().to_string(), source, "a drag back to the start changes nothing");
        }
        drag(&handle, &text, &window, mid(handle.frame()), &[NSPoint::new(120., line.origin.y), edge]);
        assert_eq!(text.string().to_string(), format!("{}\n\n나 문단\n가 문단\n", image("x=0.000;wrap=1")));
        let picture = images(&text)[0];
        let beside = rect_at(&text, &window, "나 문단");
        let below = rect_at(&text, &window, "가 문단");
        assert!(beside.origin.x > picture.origin.x + picture.size.width && below.origin.y >= picture.origin.y + picture.size.height);
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);

        // A drag from the page margin selects whole blocks; Escape lets go,
        // Backspace deletes them in one undo step and copy takes their Markdown.
        let source = "가 문단\n나 문단\n다 문단\n라 문단\n";
        show(&text, base.path(), source);
        let (second, third) = (rect_at(&text, &window, "나 문단"), rect_at(&text, &window, "다 문단"));
        let sweep = |text: &NSTextView| drag(text, text, &window, NSPoint::new(8., second.origin.y + 6.), &[NSPoint::new(12., second.origin.y + 12.), NSPoint::new(14., third.origin.y + 8.)]);
        sweep(&text);
        capture(&scroll, "selected");
        key(&text, &window, 53, "\u{1b}");
        key(&text, &window, 51, "\u{7f}");
        assert_ne!(text.string().to_string(), "가 문단\n라 문단\n", "Escape cleared the block selection");
        show(&text, base.path(), source);
        sweep(&text);
        let board = NSPasteboard::generalPasteboard();
        let saved: Vec<_> = board.types().into_iter().flat_map(|types| types.to_vec())
            .filter_map(|t| board.dataForType(&t).map(|d| (t, d))).collect();
        let _: () = msg_send![&*text, copy:std::ptr::null::<objc2::runtime::AnyObject>()];
        let copied = board.stringForType(NSPasteboardTypeString).map(|s| s.to_string());
        board.clearContents();
        for (t, d) in saved { board.setData_forType(Some(&d), &t); }
        assert_eq!(copied.as_deref(), Some("나 문단\n다 문단\n"));
        key(&text, &window, 51, "\u{7f}");
        assert_eq!(text.string().to_string(), "가 문단\n라 문단\n");
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), source);
        // A plain click in the margin is still an ordinary click.
        click(&text, &window, NSPoint::new(8., second.origin.y + 6.));
        insert(&text, "!");
        assert!(text.string().to_string().contains("!나 문단") || text.string().to_string().contains("나 문단!"), "{:?}", text.string().to_string());
        let _ = caret(&text, &window);

        // A rule does not depend on the block above it: typed right under a
        // text line it is the same rule, and that line stays body text.
        show(&text, base.path(), "본문\n\n---\n뒤\n");
        let spaced = (rect_at(&text, &window, "본문"), rect_at(&text, &window, "---"), rect_at(&text, &window, "뒤"));
        show(&text, base.path(), "본문\n--");
        insert(&text, "-");
        insert(&text, "뒤");
        editor::style_with_base(&text, Some(base.path()));
        assert_eq!(text.string().to_string(), "본문\n---\n뒤");
        capture(&scroll, "rule-under-text");
        let tight = (rect_at(&text, &window, "본문"), rect_at(&text, &window, "---"), rect_at(&text, &window, "뒤"));
        assert_eq!(tight.0.size.height, spaced.0.size.height, "the line above stays body text");
        assert_eq!(tight.2.origin.y - tight.1.origin.y, spaced.2.origin.y - spaced.1.origin.y, "same rule row");
        assert_eq!(tight.1.origin.y - tight.0.origin.y, spaced.0.size.height.max(26.) + 2., "the rule starts right under the text line");

        // The caret treats a rule as one block: arrows and clicks pass over
        // its hidden marks, and Backspace below it removes it at once.
        let rule_at = "본문\n".encode_utf16().count();
        let position = |text: &NSTextView| { let range: NSRange = msg_send![text, selectedRange]; range.location };
        let nil = std::ptr::null::<objc2::runtime::AnyObject>();
        let _: () = msg_send![&*text, moveLeft:nil];
        assert_eq!(position(&text), rule_at + 4, "left from 뒤 stops at the start of its line");
        let _: () = msg_send![&*text, moveLeft:nil];
        assert_eq!(position(&text), rule_at - 1, "left again lands at the end of 본문");
        let _: () = msg_send![&*text, moveRight:nil];
        assert_eq!(position(&text), rule_at + 4, "right crosses the rule in one step");
        let _: () = msg_send![&*text, moveUp:nil];
        assert!(position(&text) < rule_at, "up crosses the rule");
        let _: () = msg_send![&*text, moveDown:nil];
        assert!(position(&text) >= rule_at + 4, "down crosses the rule");
        click(&text, &window, NSPoint::new(300., tight.1.origin.y + tight.1.size.height / 2.));
        assert_eq!(position(&text), rule_at + 4, "a click on the rule puts the caret below it");
        // Reopen so the undo step holds only the Backspace.
        show(&text, base.path(), "본문\n---\n뒤");
        text.setSelectedRange(NSRange::new(rule_at + 4, 0));
        key(&text, &window, 51, "\u{7f}");
        assert_eq!(text.string().to_string(), "본문\n뒤", "one Backspace removes the rule");
        assert_eq!(position(&text), rule_at);
        undo(&text, base.path());
        assert_eq!(text.string().to_string(), "본문\n---\n뒤", "one undo restores it");
        // However undo leaves the selection, a rule never stays highlighted.
        text.setSelectedRange(NSRange::new(rule_at, 4));
        let _: () = msg_send![&*text, stologySettleRule:nil];
        let after: NSRange = msg_send![&*text, selectedRange];
        assert_eq!((after.location, after.length), (rule_at + 4, 0), "the restored rule is not left selected");
        // Undo restyles by itself, without waiting for the app's next pass:
        // a restored table gets its overlay back straight away.
        let table = "앞\n\n| 가 | 나 |\n| --- | --- |\n| 1 | 2 |\n\n뒤";
        show(&text, base.path(), table);
        text.setSelectedRange(NSRange::new(table.encode_utf16().count() - 1, 0));
        key(&text, &window, 51, "\u{7f}");
        assert!(tables(&text).is_empty() && !text.string().to_string().contains("가"), "Backspace removed the table");
        text.undoManager().unwrap().undo();
        assert_eq!(text.string().to_string(), table);
        assert_eq!(tables(&text).len(), 1, "undo redraws what it restores");
        text.undoManager().unwrap().redo();
        assert!(tables(&text).is_empty(), "redo redraws too");
        println!("PASS: attachment rows (left/right/whole row, pairs, right tables, code beside an image), image drag and menu placement, table placement from the cell menu, side blocks by click and drop, block handle moves, margin selection with delete/copy/undo, hidden image lines skipped");
    }
}
