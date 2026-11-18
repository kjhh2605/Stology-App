//! Native selectAll: scope regression. Real Cmd+A/menu routing is verified in GUI QA.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example code-selection-smoke
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

use objc2::{msg_send, rc::Retained, runtime::AnyObject, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use std::ops::Range;

fn native_range(source: &str, bytes: Range<usize>) -> NSRange {
    NSRange::new(
        source[..bytes.start].encode_utf16().count(),
        source[bytes].encode_utf16().count(),
    )
}

unsafe fn select_all(text: &NSTextView) -> NSRange {
    let _: () = msg_send![text, selectAll:std::ptr::null::<AnyObject>()];
    msg_send![text, selectedRange]
}

unsafe fn fixture(text: &NSTextView, source: &str, selection: Range<usize>) {
    text.setString(&NSString::from_str(source));
    text.setSelectedRange(native_range(source, selection));
    editor::style(text);
}

unsafe fn assert_scoped(text: &NSTextView, source: &str, content: &str, selection: Range<usize>) {
    fixture(text, source, selection);
    let start = source.find(content).expect("fixture code content");
    let expected = native_range(source, start..start + content.len());
    assert_eq!(select_all(text), expected, "wrong code scope: {source:?}");
    assert_eq!(
        select_all(text),
        expected,
        "repeated select-all escaped block"
    );
    assert_eq!(
        text.string().to_string(),
        source,
        "selection changed Markdown"
    );
}

fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().expect("main thread");
        let _app = NSApplication::sharedApplication(mtm);
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(800.0, 600.0));
        let (scroll, text) = editor::make_editor(frame);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setContentView(Some(&scroll));
        assert!(window.makeFirstResponder(Some(&text)));

        let source = "앞 😀\n\n```rust\nfn first() {}\n```\n\n중간\n\n```python\nprint(\"한글 😀\")\nprint(2)\n```\n\n끝";
        let code = "print(\"한글 😀\")\nprint(2)";
        let start = source.find(code).unwrap();
        assert_scoped(&text, source, code, start..start);
        let korean = source.find("한글").unwrap();
        assert_scoped(&text, source, code, korean..korean + "한글 😀".len());
        assert_scoped(&text, source, code, start + code.len()..start + code.len());
        assert_scoped(&text, source, code, start..start + code.len() + 1);

        // The selected code alone is replaceable; fence, first block and prose survive.
        let undo: Retained<AnyObject> = msg_send![&*text, undoManager];
        let _: () = msg_send![&*text, breakUndoCoalescing];
        let _: () = msg_send![&*undo, removeAllActions];
        let _: () = msg_send![&*text, insertText:&*NSString::from_str("replacement()"), replacementRange:NSRange::new(usize::MAX, 0)];
        assert_eq!(
            text.string().to_string(),
            source.replacen(code, "replacement()", 1)
        );
        let _: () = msg_send![&*undo, undo];
        assert_eq!(
            text.string().to_string(),
            source,
            "scoped replacement undo changed surrounding Markdown"
        );

        let crlf = "앞 😀\r\n\r\n~~~java\r\nString text = \"한글\";\r\nint n = 2;\r\n~~~\r\n\r\n뒤";
        let code = "String text = \"한글\";\r\nint n = 2;";
        let start = crlf.find(code).unwrap();
        assert_scoped(&text, crlf, code, start..start + 6);

        // An unfinished closing fence still has selectable code to EOF.
        let unclosed = "설명\n\n```js\nconst 값 = \"😀\";\n";
        let code = "const 값 = \"😀\";\n";
        let start = unclosed.find(code).unwrap();
        assert_scoped(&text, unclosed, code, start..start);

        // Empty blocks must not expand the selection to the document/fences.
        for empty in ["앞\n\n```java\n\n```\n\n뒤", "앞\n\n```java\n```\n\n뒤"] {
            let body = empty.find("```java\n").unwrap() + "```java\n".len();
            fixture(&text, empty, body..body);
            let expected = native_range(empty, body..body);
            assert_eq!(
                select_all(&text),
                expected,
                "empty block selected fence/prose"
            );
            assert_eq!(
                select_all(&text),
                expected,
                "repeated empty block selection escaped"
            );
        }

        // Read-only evidence can still select/copy its code independently.
        let readonly = "앞\n\n```rust\nlet answer = 42;\n```\n\n뒤";
        let code = "let answer = 42;";
        let start = readonly.find(code).unwrap();
        text.setEditable(false);
        assert_scoped(&text, readonly, code, start..start);
        text.setEditable(true);

        // Paragraph selection and ranges spanning blocks retain native document scope.
        fixture(&text, source, 0..0);
        let whole = NSRange::new(0, source.encode_utf16().count());
        assert_eq!(
            select_all(&text),
            whole,
            "prose select-all became code-only"
        );
        let second = source.find("print(").unwrap();
        fixture(&text, source, source.find("중간").unwrap()..second + 5);
        assert_eq!(
            select_all(&text),
            whole,
            "cross-block selection incorrectly narrowed"
        );
        let not_committed = "앞\n\n```java";
        fixture(
            &text,
            not_committed,
            not_committed.len()..not_committed.len(),
        );
        assert_eq!(
            select_all(&text),
            NSRange::new(0, not_committed.encode_utf16().count()),
            "uncommitted fence is not a code block"
        );

        window.setContentView(None);
        println!("PASS: native code-only selectAll, repeated/partial selection, second block and Unicode, CRLF, empty/imported-empty/unclosed blocks, read-only, replacement+Undo, prose/cross-block fallback.");
        println!("NOT TESTED: real Cmd+A/menu event routing or actual IME keyboard input.");
    }
}
