//! Native table cell edit/undo and field-editor lifetime checks.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example table-smoke
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
use objc2::{class, msg_send, rc::Retained, runtime::AnyObject, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString};
unsafe fn cells(text: &NSTextView) -> Vec<Retained<NSTextField>> {
    let mut result = Vec::new();
    for table in text.subviews() {
        if !table.isKindOfClass(class!(StologyInlineMarkdownTable)) {
            continue;
        }
        for cell in table.subviews() {
            if cell.isKindOfClass(class!(StologyMarkdownTableCell)) {
                result.push(Retained::retain(Retained::as_ptr(&cell) as *mut NSTextField).unwrap());
            }
        }
    }
    result
}
thread_local! {
    static DRAG_COLUMNS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static DRAG_EDITOR: std::cell::RefCell<Option<Retained<NSTextView>>> = const {std::cell::RefCell::new(None)};
    static DRAG_FRAMES: std::cell::RefCell<Vec<(usize, String)>> = const {std::cell::RefCell::new(Vec::new())};
}
objc2::define_class!(
    #[unsafe(super = NSWindow)]
    #[name = "StologyTableSmokeWindow"]
    #[thread_kind = MainThreadOnly]
    struct SmokeWindow;
    impl SmokeWindow {
        #[unsafe(method(displayIfNeeded))]
        fn display_if_needed(&self) { unsafe {
            let _: () = msg_send![super(self), displayIfNeeded];
            DRAG_EDITOR.with_borrow(|active| {
                if let Some(text) = active {
                    assert!(NSCursor::currentCursor().isEqual(Some(&*if DRAG_COLUMNS.get() { NSCursor::resizeLeftRightCursor() } else { NSCursor::resizeUpDownCursor() })), "table preview replaced resize cursor");
                    DRAG_FRAMES.with_borrow_mut(|frames| frames.push((cells(text).len(),text.string().to_string())));
                }
            });
        }}
    }
);

unsafe fn check_drag(text: &Retained<NSTextView>, window: &NSWindow, columns: bool) {
    let source = text.string().to_string();
    let table = text
        .subviews()
        .into_iter()
        .find(|v| v.isKindOfClass(class!(StologyInlineMarkdownTable)))
        .unwrap();
    let handle = table
        .subviews()
        .into_iter()
        .find(|v| {
            v.isKindOfClass(class!(StologyTableHandle))
                && (v.frame().size.width < v.frame().size.height) == columns
        })
        .unwrap();
    let unit = if columns { handle.frame().origin.x / 2.0 } else { 30.0 };
    let event = |kind, steps: f64| {
        let distance = steps * unit;
        let point = if columns {
            NSPoint::new(100.0 + distance, 100.0)
        } else {
            NSPoint::new(100.0, 100.0 - distance)
        };
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind, point, NSEventModifierFlags::empty(), 0.0, window.windowNumber(), None, 1, 1, 1.0).unwrap()
    };
    NSCursor::IBeamCursor().set();
    let _: () = msg_send![&*handle, cursorUpdate:&*event(NSEventType::MouseMoved, 0.0)];
    assert!(NSCursor::currentCursor().isEqual(Some(&*if columns { NSCursor::resizeLeftRightCursor() } else { NSCursor::resizeUpDownCursor() })), "handle hover has wrong resize cursor");
    // Queue a reversal before release. Both previews must use the untouched source.
    for (kind, distance) in [
        (NSEventType::LeftMouseUp, 1.0),
        (NSEventType::LeftMouseDragged, -1.0),
        (NSEventType::LeftMouseDragged, 2.0),
    ] {
        window.postEvent_atStart(&event(kind, distance), true);
    }
    DRAG_COLUMNS.set(columns);
    DRAG_EDITOR.set(Some(text.clone()));
    let _: () = msg_send![&*handle, mouseDown:&*event(NSEventType::LeftMouseDown,0.0)];
    DRAG_EDITOR.set(None);
    let frames = DRAG_FRAMES.take();
    assert!(frames.iter().any(|(n, s)| *n == 8 && s == &source));
    assert!(frames.iter().any(|(n, s)| *n == 2 && s == &source));
    assert_eq!(cells(text).len(), 6);
    let undo: Retained<AnyObject> = msg_send![&**text, undoManager];
    let _: () = msg_send![&*undo, undo];
    assert_eq!(text.string().to_string(), source);
    text.setSelectedRange(NSRange::new(0, 0));
    editor::style(text);
}
unsafe fn check_grid_divider(text: &NSTextView, window: &NSWindow, columns: bool) {
    let original = text.string().to_string();
    let controls = cells(text);
    let table = controls[0].superview().unwrap();
    let divider = table.subviews().into_iter().find(|view| {
        view.isKindOfClass(class!(StologyGridDivider)) && (view.frame().size.width < view.frame().size.height) == columns
    }).unwrap();
    let boundary = controls[0].frame();
    for offset in [-4., 4.] {
        let point = if columns { NSPoint::new(boundary.size.width + 1. + offset, 10.) }
                    else { NSPoint::new(10., boundary.size.height + 1. + offset) };
        let hit = table.hitTest(table.convertPoint_toView(point, table.superview().as_deref())).unwrap();
        assert!(hit.isEqual(Some(&*divider)), "divider missed a point 4pt from its center");
    }
    let event = |kind, delta| {
        let point = if columns { NSPoint::new(100. + delta, 100.) } else { NSPoint::new(100., 100. - delta) };
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind, point, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 1, 1, 1.).unwrap()
    };
    window.postEvent_atStart(&event(NSEventType::LeftMouseUp, 36.), true);
    window.postEvent_atStart(&event(NSEventType::LeftMouseDragged, 24.), true);
    let _: () = msg_send![&*divider,mouseDown:&*event(NSEventType::LeftMouseDown,0.)];
    let result = cells(text);
    assert_eq!(result.len(), 4);
    for i in 0..4 {
        let before = controls[i].frame();
        let after = result[i].frame();
        if columns {
            let change = if i % 2 == 0 {36.} else {-36.};
            assert_eq!(after.size.width, before.size.width + change);
            assert_eq!(after.origin.y, before.origin.y);
        } else if i < 2 {
            assert_eq!(after.size.height, before.size.height + 36.);
        } else {
            assert_eq!(after.origin.y, before.origin.y + 36.);
            assert_eq!(after.size.height, before.size.height);
        }
    }
    let undo: Retained<AnyObject> = msg_send![text, undoManager];
    let _: () = msg_send![&*undo, undo];
    assert_eq!(text.string().to_string(), original);
    text.setSelectedRange(NSRange::new(0,0));
    editor::style(text);
}
unsafe fn check_after_table_caret(text: &NSTextView, window: &NSWindow) {
    let table = "|  |  |\n| --- | --- |\n|  |  |\n\n";
    let caret_top = |suffix: &str| {
        let source = format!("{table}{suffix}");
        text.setString(&NSString::from_str(&source));
        let caret = NSRange::new(source.encode_utf16().count(), 0);
        text.setSelectedRange(caret);
        window.makeFirstResponder(Some(text));
        editor::style(text);
        let rect: NSRect = msg_send![text, firstRectForCharacterRange:caret actualRange:std::ptr::null_mut::<NSRange>()];
        assert!(rect.size.height > 0.0);
        assert_eq!(text.string().to_string(), source);
        assert_eq!(cells(text).len(), 4);
        rect.origin.y + rect.size.height
    };
    let empty = caret_top("");
    let typed = caret_top("본문");
    let extra_blank = caret_top("\n본문");
    assert!((empty - typed).abs() < 1.0, "typing after the table shifted the caret vertically: empty={empty}, typed={typed}");
    assert!((typed - extra_blank - 26.0).abs() < 1.0, "an extra blank line should remain visible: typed={typed}, extra={extra_blank}");
}

unsafe fn check_block_clicks(text: &NSTextView, window: &NSWindow) {
    let click = |point| {
        let position = text.convertPoint_toView(point, None);
        let event = |kind| NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            kind, position, NSEventModifierFlags::empty(), 0., window.windowNumber(), None, 1, 1, 1.).unwrap();
        window.discardEventsMatchingMask_beforeEvent(NSEventMask::LeftMouseUp, None);
        window.postEvent_atStart(&event(NSEventType::LeftMouseUp), true);
        let _: () = msg_send![text, mouseDown:&*event(NSEventType::LeftMouseDown)];
    };
    let caret_rect = || {
        let selection: NSRange = msg_send![text, selectedRange];
        let screen: NSRect = msg_send![text,firstRectForCharacterRange:selection actualRange:std::ptr::null_mut::<NSRange>()];
        text.convertRect_fromView(window.convertRectFromScreen(screen), None)
    };
    let insert = |value| {
        let selection: NSRange = msg_send![text,selectedRange];
        let _: () = msg_send![text,insertText:&*NSString::from_str(value),replacementRange:selection];
    };
    let table = "| 열 | 값 |\n| --- | --- |\n| 😀 | 2 |\n\n";
    text.setString(&NSString::from_str(&format!("{table}뒤 문단")));
    text.setSelectedRange(NSRange::new(text.string().length(),0));
    editor::style(text);
    let frame = cells(text)[0].superview().unwrap().frame();
    let side = NSPoint::new(frame.origin.x + frame.size.width + 25., frame.origin.y + 38.);
    click(side);
    insert("오른쪽");
    let caret = caret_rect();
    assert!(caret.origin.x > frame.origin.x + frame.size.width && (side.y - caret.origin.y).abs() < 26.,
        "side click {side:?} placed text at {caret:?}");
    assert!(text.string().to_string().ends_with("뒤 문단") && cells(text).len() == 4);
    click(NSPoint::new(side.x, side.y - 26.));
    insert("윗줄");
    assert!(text.string().to_string().contains("윗줄\n오른쪽"), "{:?}, caret {:?}, table {:?}", text.string().to_string(), caret_rect(), frame);
    for block in [table, "```rs\nfn example() {}\n```\n", "```rs\nfn example() {}\n```"] {
        text.setString(&NSString::from_str(block));
        text.setSelectedRange(NSRange::new(text.string().length(),0));
        editor::style(text);
        let below = NSPoint::new(80., 280.);
        click(below);
        insert("아래 본문");
        let caret = caret_rect();
        assert!((below.y - caret.origin.y).abs() < 26.,
            "below click {below:?} placed text at {caret:?} for {:?}", text.string().to_string());
        assert!(text.string().to_string().ends_with("\n아래 본문"));
    }
    let base = tempfile::tempdir().unwrap();
    let filename = ".diagram/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.svg";
    std::fs::create_dir(base.path().join(".diagram")).unwrap();
    std::fs::write(base.path().join(filename), r#"<svg xmlns="http://www.w3.org/2000/svg" width="170" height="80"><rect width="170" height="80"/></svg>"#).unwrap();
    let source = format!("![그림](<{filename}> \"stology:width=170;x=0;wrap=1\")\n\n");
    text.setString(&NSString::from_str(&source));
    text.setSelectedRange(NSRange::new(text.string().length(), 0));
    editor::style_with_base(text, Some(base.path()));
    let image = text.subviews().into_iter().find(|v| v.isKindOfClass(class!(StologyInlineMarkdownImage))).unwrap();
    let frame = image.frame();
    let below = NSPoint::new(64., frame.origin.y + frame.size.height + 3.);
    click(below);
    let at = caret_rect();
    assert!(at.origin.x < 64., "caret for {below:?} is beside diagram {frame:?}: {at:?}");
    let _: () = msg_send![text, deleteBackward:std::ptr::null::<AnyObject>()];
    assert!(text.string().to_string().trim().is_empty(), "diagram still present: {:?}", text.string().to_string());

}

fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(0., 0.), NSSize::new(800., 600.));
        let (scroll, text) = editor::make_editor(frame);
        let allocated = SmokeWindow::alloc(mtm);
        let window: Retained<SmokeWindow> = msg_send![allocated, initWithContentRect:frame styleMask:NSWindowStyleMask::Titled backing:NSBackingStoreType::Buffered defer:false];
        window.setContentView(Some(&scroll));
        check_after_table_caret(&text, &window);
        check_block_clicks(&text, &window);
        // Each fixture replaces the document, as the app does when opening another note.
        let undo: Retained<AnyObject> = msg_send![&*text, undoManager];
        let _: () = msg_send![&*undo, removeAllActions];
        let original = "# Table\n\n| **열** | 값 |\n|:---|---:|\n| 한글 | 😀 |\n\n끝";
        text.setString(&NSString::from_str(original));
        text.setSelectedRange(NSRange::new(0, 0));
        window.makeFirstResponder(Some(&text));
        editor::style(&text);
        check_drag(&text, &window, true);
        check_drag(&text, &window, false);
        check_grid_divider(&text,&window,true);
        check_grid_divider(&text,&window,false);
        let controls = cells(&text);
        assert_eq!(controls.len(), 4);
        assert!(controls.iter().all(|cell| cell.isEditable()));
        // Commit one cell without reserializing the header or separator row.
        controls[2].setStringValue(&NSString::from_str("수정 | 한글"));
        let _: () = msg_send![&*controls[2], commitCell:std::ptr::null::<AnyObject>()];
        let expected = original.replace("| 한글 |", "| 수정 \\| 한글 |");
        assert_eq!(text.string().to_string(), expected);
        assert_eq!(cells(&text).len(), 4);
        // A second cell must use the shifted UTF-16 range after the first edit.
        controls[3].setStringValue(&NSString::from_str("새 값"));
        let _: () = msg_send![&*controls[3], commitCell:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), expected.replace('😀', "새 값"));
        let undo: Retained<AnyObject> = msg_send![&*text, undoManager];
        let _: () = msg_send![&*undo, undo];
        // AppKit may group adjacent edits in this synchronous smoke invocation.
        assert!(text.string().to_string() == original || text.string().to_string() == expected);
        let _: () = msg_send![&*undo, redo];
        assert_eq!(text.string().to_string(), expected.replace('😀', "새 값"));
        let _: () = msg_send![&*undo, removeAllActions];
        text.setString(&NSString::from_str(original));
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style(&text);
        let controls = cells(&text);
        controls[2].selectText(None);
        assert!(
            editor::table_cell_is_editing(&text),
            "no native field editor"
        );
        let retained_cell = Retained::as_ptr(&controls[2]);
        editor::style(&text);
        assert!(
            cells(&text)
                .iter()
                .any(|cell| Retained::as_ptr(cell) == retained_cell),
            "style destroyed focused cell"
        );
        let field = controls[2]
            .currentEditor()
            .expect("native cell field editor");
        let delegate: Retained<AnyObject> = msg_send![&*field, delegate];
        assert!(Retained::as_ptr(&delegate) == Retained::as_ptr(&controls[2]).cast(), "context-click filter must recognize the active cell");
        let _: () = msg_send![&*field, setSelectedRange:NSRange::new(2, 0)];
        let _: () = msg_send![&*field, selectAll:std::ptr::null::<AnyObject>()];
        let selected: NSRange = msg_send![&*field, selectedRange];
        assert_eq!(
            selected,
            NSRange::new(0, 2),
            "cell select-all left an insertion cursor"
        );
        let _: () = msg_send![&*field, insertText:&*NSString::from_str("Beta"), replacementRange:NSRange::new(usize::MAX,0)];
        assert_eq!(
            field.string().to_string(),
            "Beta",
            "typing appended despite select-all"
        );
        let _: () = msg_send![&*field, selectAll:std::ptr::null::<AnyObject>()];
        let _: () = msg_send![&*field, setMarkedText:&*NSString::from_str("한"), selectedRange:NSRange::new(1,0), replacementRange:NSRange::new(usize::MAX,0)];
        editor::style(&text);
        let marked: bool = msg_send![&*field, hasMarkedText];
        assert!(marked, "restyle disturbed marked cell text");
        let _: () = msg_send![&*field, unmarkText];
        window.endEditingFor(None);
        assert!(
            text.string().to_string().contains("| 한 |"),
            "ending cell editing failed to commit field text"
        );
        text.setEditable(false);
        editor::style(&text);
        assert!(cells(&text).iter().all(|cell| !cell.isEditable()));
        window.setContentView(None);
        println!("PASS: clicks beside and below blocks, diagram boundary backspace, live row and column previews, drag reversal before commit, exact table cell edits, Unicode and pipe escaping, shifted neighbor offsets, native undo, field editor lifetime, marked-text preservation, read-only cells.");
        println!("NOT TESTED: real keyboard/IME or pointer navigation.");
    }
}
