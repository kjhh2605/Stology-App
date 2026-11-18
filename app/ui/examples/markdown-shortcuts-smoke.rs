//! Native source, presentation, key-equivalent and undo regression checks.
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/editor.rs"]
#[allow(dead_code, unused_imports)]
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
use objc2::{msg_send, rc::Retained, runtime::AnyObject, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};

unsafe fn insert(text: &NSTextView, value: &str) {
    let range: NSRange = msg_send![text, selectedRange];
    let _: () = msg_send![text, insertText:&*NSString::from_str(value), replacementRange:range];
}
unsafe fn shortcut(text: &NSTextView, key: u16, flags: NSEventModifierFlags) {
    let event = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
        NSEventType::KeyDown, NSPoint::new(0.0,0.0), flags, 0.0, 0, None,
        &NSString::from_str(""), &NSString::from_str(""), false, key).unwrap();
    assert!(text.performKeyEquivalent(&event), "unhandled key {key}");
}
unsafe fn reset(text: &NSTextView, source: &str) {
    text.setString(&NSString::from_str(source));
    text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
    editor::style(text);
}
fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(700.0, 500.0));
        let (scroll, text) = editor::make_editor(frame);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setContentView(Some(&scroll));
        window.makeFirstResponder(Some(&text));
        // Inspect actual attributed text: bare angle brackets must retain ink,
        // while committed quotes are gray and inline links retain their color.
        for (source, index, expected_red) in [
            (">", 0, 0.09), (">>>한글", 3, 0.09),
            (">>> 한글", 4, 0.09), (">text", 1, 0.09),
            ("> quote\n>>>plain", 10, 0.09),
            ("> quote", 2, 0.40), ("> > nested", 4, 0.40),
            ("> [link](https://example.com)", 3, 0.31),
        ] {
            reset(&text, source);
            let storage = text.textStorage().unwrap();
            let color: Retained<NSColor> = msg_send![&*storage, attribute:NSForegroundColorAttributeName, atIndex:index as usize, effectiveRange:std::ptr::null_mut::<NSRange>()];
            assert!((color.redComponent() - expected_red).abs() < 0.001, "wrong color: {source}");
        }
        for marker in ["-", "*", "+", "12.", ">", "|", "\""] {
            reset(&text, "");
            insert(&text, marker);
            insert(&text, " ");
            let expected = if matches!(marker, "|" | "\"") {
                ">"
            } else {
                marker
            };
            assert_eq!(text.string().to_string(), format!("{expected} "));
            let storage = text.textStorage().unwrap();
            let paragraph: Retained<NSParagraphStyle> = msg_send![&*storage, attribute:NSParagraphStyleAttributeName, atIndex:0usize, effectiveRange:std::ptr::null_mut::<NSRange>()];
            assert!(paragraph.headIndent() >= 24.0, "no indent for {marker}");
            let font: Retained<NSFont> = msg_send![&*storage, attribute:NSFontAttributeName, atIndex:0usize, effectiveRange:std::ptr::null_mut::<NSRange>()];
            assert!(font.pointSize() < 0.1, "visible marker {marker}");
            insert(&text, "한글 😀");
            let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
            if expected == "12." {
                assert_eq!(text.string().to_string(), "12. 한글 😀\n13. ");
            } else if expected == ">" {
                assert_eq!(text.string().to_string(), "> 한글 😀\n\n");
            } else {
                assert_eq!(
                    text.string().to_string(),
                    format!("{expected} 한글 😀\n{expected} ")
                );
            }
        }
        reset(&text, "- 첫째\n- ");
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 첫째\n");
        reset(&text, "> 인용");
        text.setSelectedRange(NSRange::new(2, 0));
        let _: () = msg_send![&*text, deleteBackward:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "인용");
        reset(&text, "```text\n|\n```");
        text.setSelectedRange(NSRange::new(9, 0));
        insert(&text, " ");
        assert_eq!(text.string().to_string(), "```text\n| \n```");
        reset(&text, "");
        insert(&text, "| a | b |\n| --- | --- |\n| c | d |");
        assert!(
            text.string().to_string().starts_with("| a |"),
            "paste became quote"
        );
        let command = NSEventModifierFlags::Command;
        for (key, flags, open, close, attribute) in [
            (11, command, "**", "**", NSFontAttributeName),
            (34, command, "*", "*", NSFontAttributeName),
            (32, command, "<u>", "</u>", NSUnderlineStyleAttributeName),
            (
                1,
                command | NSEventModifierFlags::Shift,
                "~~",
                "~~",
                NSStrikethroughStyleAttributeName,
            ),
            (14, command, "`", "`", NSFontAttributeName),
        ] {
            reset(&text, "한글 😀");
            text.setSelectedRange(NSRange::new(0, 5));
            shortcut(&text, key, flags);
            assert_eq!(text.string().to_string(), format!("{open}한글 😀{close}"));
            let storage = text.textStorage().unwrap();
            let value: Option<Retained<AnyObject>> = msg_send![&*storage, attribute:attribute, atIndex:open.len(), effectiveRange:std::ptr::null_mut::<NSRange>()];
            assert!(value.is_some(), "missing style {open}");
            shortcut(&text, key, flags);
            assert_eq!(text.string().to_string(), "한글 😀", "toggle off {open}");
        }
        reset(&text, "Undo 😀");
        text.setSelectedRange(NSRange::new(0, 7));
        let undo: Retained<AnyObject> = msg_send![&*text, undoManager];
        let _: () = msg_send![&*undo, removeAllActions];
        shortcut(&text, 1, command | NSEventModifierFlags::Shift);
        let _: () = msg_send![&*undo, undo];
        assert_eq!(text.string().to_string(), "Undo 😀");
        let _: () = msg_send![&*undo, redo];
        assert_eq!(text.string().to_string(), "~~Undo 😀~~");
        reset(&text, "한글 😀");
        text.setSelectedRange(NSRange::new(0, 5));
        shortcut(&text, 32, command);
        shortcut(&text, 11, command);
        shortcut(&text, 32, command);
        assert_eq!(
            text.string().to_string(),
            "**한글 😀**",
            "remove outer underline while retaining bold"
        );
        shortcut(&text, 11, command);
        assert_eq!(text.string().to_string(), "한글 😀");
        reset(&text, "> 인용");
        let _: () = msg_send![&*text, insertLineBreak:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "> 인용\n> ");
        for (key, prefix) in [
            (29, ""),
            (18, "# "),
            (19, "## "),
            (20, "### "),
            (21, "- [ ] "),
            (23, "- "),
            (22, "1. "),
            (28, "```text\n"),
        ] {
            reset(&text, "내용");
            shortcut(&text, key, command | NSEventModifierFlags::Option);
            assert!(text
                .string()
                .to_string()
                .starts_with(&format!("{prefix}내용")));
        }
        reset(&text, "```text\ncode\n```");
        text.setSelectedRange(NSRange::new(8, 4));
        shortcut(&text, 11, command);
        assert_eq!(text.string().to_string(), "```text\ncode\n```");
        reset(&text, "- 부모\n- 자식 😀");
        let _: () = msg_send![&*undo, removeAllActions];
        let _: () = msg_send![&*text, insertTab:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 부모\n  - 자식 😀");
        let storage = text.textStorage().unwrap();
        let parent: Retained<NSParagraphStyle> = msg_send![&*storage, attribute:NSParagraphStyleAttributeName, atIndex:0usize, effectiveRange:std::ptr::null_mut::<NSRange>()];
        let child: Retained<NSParagraphStyle> = msg_send![&*storage, attribute:NSParagraphStyleAttributeName, atIndex:7usize, effectiveRange:std::ptr::null_mut::<NSRange>()];
        assert_eq!(child.headIndent() - parent.headIndent(), 16.0);
        let _: () = msg_send![&*undo, undo];
        assert_eq!(text.string().to_string(), "- 부모\n- 자식 😀");
        let _: () = msg_send![&*undo, redo];
        assert_eq!(text.string().to_string(), "- 부모\n  - 자식 😀");
        text.setSelectedRange(NSRange::new(text.string().length(), 0));
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 부모\n  - 자식 😀\n  - ");
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 부모\n  - 자식 😀\n- ");
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 부모\n  - 자식 😀\n");
        reset(&text, "- 부모\n  - 자식");
        let _: () = msg_send![&*text, insertBacktab:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- 부모\n- 자식");
        for (token, symbol) in [("->", "→"), ("<-", "←"), ("<->", "↔"), ("=>", "⇒"), ("<=>", "⇔")] {
            let original = format!("한글 😀 {token}");
            reset(&text, &original);
            let _: () = msg_send![&*undo, removeAllActions];
            insert(&text, " ");
            assert_eq!(text.string().to_string(), format!("한글 😀 {symbol} "));
            let selected: NSRange = msg_send![&*text, selectedRange];
            assert_eq!(selected, NSRange::new(text.string().length(), 0));
            let _: () = msg_send![&*undo, undo];
            assert_eq!(text.string().to_string(), original);
            let _: () = msg_send![&*undo, redo];
            assert_eq!(text.string().to_string(), format!("한글 😀 {symbol} "));
        }
        reset(&text, "");
        insert(&text, "-> <-> => ");
        assert_eq!(text.string().to_string(), "-> <-> => ", "bulk input rewritten");
        reset(&text, "```rust\n->\n```");
        text.setSelectedRange(NSRange::new(10, 0));
        insert(&text, " ");
        assert_eq!(text.string().to_string(), "```rust\n-> \n```");

        let source = "한글 😀 (hello world). Next.\n\nLast.";
        reset(&text, source);
        let _: () = msg_send![&*undo, removeAllActions];
        text.setSelectedRange(NSRange::new(9, 0));
        for expected in ["hello", "hello world", "(hello world)"] {
            shortcut(&text, 2, command);
            let range: NSRange = msg_send![&*text, selectedRange];
            assert_eq!(text.string().substringWithRange(range).to_string(), expected);
            assert_eq!(text.string().to_string(), source);
        }
        for _ in 0..12 { shortcut(&text, 2, command); }
        let selected: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(selected, NSRange::new(0, source.encode_utf16().count()));
        let can_undo: bool = msg_send![&*undo, canUndo];
        assert!(!can_undo, "selection changed undo history");
        // A new caret starts a new expansion, with no stale selection history.
        text.setSelectedRange(NSRange::new(1, 0));
        shortcut(&text, 2, command);
        let selected: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(text.string().substringWithRange(selected).to_string(), "한글");
        let _: () = msg_send![&*text, setMarkedText:&*NSString::from_str("한"), selectedRange:NSRange::new(1,0), replacementRange:selected];
        let before: NSRange = msg_send![&*text, selectedRange];
        let marked: NSRange = msg_send![&*text, markedRange];
        shortcut(&text, 2, command);
        let after: NSRange = msg_send![&*text, selectedRange];
        let after_marked: NSRange = msg_send![&*text, markedRange];
        assert_eq!(before, after);
        assert_eq!(marked, after_marked);
        let _: () = msg_send![&*text, unmarkText];
        println!("markdown-shortcuts-smoke PASS: lists/quotes, formatting, undo/redo, block shortcuts, code/table protection, arrows, Cmd+D structural selection, Unicode and marked-text protection");
    }
}
