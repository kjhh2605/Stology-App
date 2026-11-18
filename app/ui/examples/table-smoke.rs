//! Native table cell edit/undo and field-editor lifetime checks.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example table-smoke
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
        window.setContentView(Some(&scroll));
        let original = "# Table\n\n| **열** | 값 |\n|:---|---:|\n| 한글 | 😀 |\n\n끝";
        text.setString(&NSString::from_str(original));
        text.setSelectedRange(NSRange::new(0, 0));
        window.makeFirstResponder(Some(&text));
        editor::style(&text);
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
        println!("PASS: exact table cell edits, Unicode and pipe escaping, shifted neighbor offsets, native undo, field editor lifetime, marked-text preservation, read-only cells.");
        println!("NOT TESTED: real keyboard/IME or pointer navigation; row/column insertion and resize are not implemented.");
    }
}
