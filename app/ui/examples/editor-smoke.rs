//! Reproducible native API smoke test. Does not replace real keyboard/IME QA.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example editor-smoke
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
use objc2_foundation::{NSDictionary, NSPoint, NSRange, NSRect, NSSize, NSString};
unsafe fn font_at(text: &NSTextView, index: usize) -> Retained<NSFont> {
    let storage = text.textStorage().unwrap();
    msg_send![&*storage,attribute:NSFontAttributeName,atIndex:index,effectiveRange:std::ptr::null_mut::<NSRange>()]
}
unsafe fn color_at(text: &NSTextView, index: usize) -> Retained<NSColor> {
    let storage = text.textStorage().unwrap();
    msg_send![&*storage,attribute:NSForegroundColorAttributeName,atIndex:index,effectiveRange:std::ptr::null_mut::<NSRange>()]
}
unsafe fn native_insert(text: &NSTextView, value: &str) {
    let range: NSRange = msg_send![text, selectedRange];
    let _: () = msg_send![text,insertText:&*NSString::from_str(value),replacementRange:range];
}
unsafe fn code_language_button(text: &NSTextView) -> Retained<NSView> {
    text.subviews()
        .iter()
        .find(|v| {
            let found: bool = msg_send![&**v,isKindOfClass:class!(StologyCodeLanguageButton)];
            found
        })
        .expect("code language menu missing")
}
unsafe fn table_count(text: &NSTextView) -> usize {
    text.subviews()
        .iter()
        .filter(|v| {
            let result: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownTable)];
            result
        })
        .count()
}
unsafe fn button_count(text: &NSTextView) -> usize {
    text.subviews()
        .iter()
        .filter(|v| {
            let result: bool = msg_send![&**v,isKindOfClass:class!(NSButton)];
            result
        })
        .count()
}
fn position(source: &str, needle: &str) -> usize {
    source[..source.find(needle).unwrap()]
        .encode_utf16()
        .count()
}
fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().expect("main thread");
        let _app = NSApplication::sharedApplication(mtm);
        let directory = std::env::temp_dir().join(format!(
            "stology-editor-smoke-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let png = directory.join("pixel.png");
        std::fs::write(
            &png,
            [
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99, 252,
                255, 31, 0, 3, 3, 2, 0, 239, 154, 95, 91, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96,
                130,
            ],
        )
        .unwrap();
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
        let undo: Option<Retained<AnyObject>> = msg_send![&*text, undoManager];
        let undo = undo.expect("native undo manager");

        // Invoke real NSTextView editing selectors. No manual style/timer tick
        // is allowed between typing and the immediate presentation assertions.
        native_insert(&text, "##");
        assert_eq!(
            font_at(&text, 0).pointSize(),
            16.0,
            "bare hashes converted before space"
        );
        native_insert(&text, " ");
        assert!(
            font_at(&text, 0).pointSize() < 0.1,
            "committed heading marker is visible"
        );
        native_insert(&text, "한글 😀 제목");
        let heading = text.string().to_string();
        assert_eq!(heading, "## 한글 😀 제목");
        assert_eq!(font_at(&text, 3).pointSize(), 24.0);
        assert!(
            font_at(&text, 0).pointSize() < 0.1,
            "active heading revealed hashes"
        );
        let _: () = msg_send![&*text, moveToBeginningOfLine:std::ptr::null::<AnyObject>()];
        let visual_start: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(
            visual_start.location, 3,
            "Home/Cmd-Left entered hidden heading prefix"
        );
        let _: () = msg_send![&*text, moveLeft:std::ptr::null::<AnyObject>()];
        let left_at_start: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(
            left_at_start.location, 3,
            "Left arrow entered hidden heading prefix at document start"
        );
        text.setSelectedRange(NSRange::new(3, 0));
        let _: () = msg_send![&*text, breakUndoCoalescing];
        let _: () = msg_send![&*undo, removeAllActions];
        let _: () = msg_send![&*text, deleteBackward:std::ptr::null::<AnyObject>()];
        assert_eq!(
            text.string().to_string(),
            "한글 😀 제목",
            "Backspace did not remove entire prefix"
        );
        let _: () = msg_send![&*undo, undo];
        assert_eq!(
            text.string().to_string(),
            heading,
            "heading Backspace undo lost source"
        );
        editor::style(&text);
        assert!(font_at(&text, 0).pointSize() < 0.1);
        text.setSelectedRange(NSRange::new(heading.encode_utf16().count(), 0));
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        native_insert(&text, "본문");
        let after_heading = text.string().to_string();
        assert_eq!(after_heading, format!("{heading}\n본문"));
        assert_eq!(
            font_at(&text, position(&after_heading, "본문")).pointSize(),
            16.0
        );

        text.setString(&NSString::from_str(""));
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style(&text);
        native_insert(&text, "```java");
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "```java\n\n```");
        let code_selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(code_selection.location, "```java\n".len());
        assert!(
            font_at(&text, 0).pointSize() < 0.1,
            "opening fence remains visible"
        );
        let _: () = msg_send![&*text, insertTab:std::ptr::null::<AnyObject>()];
        native_insert(&text, "public class Demo { String s = \"한글\"; }");
        let java = text.string().to_string();
        assert_eq!(
            java,
            "```java\n  public class Demo { String s = \"한글\"; }\n```"
        );
        let code_end: NSRange = msg_send![&*text, selectedRange];
        text.setSelectedRange(NSRange::new("```java\n".len(), 0));
        let _: () = msg_send![&*text, deleteBackward:std::ptr::null::<AnyObject>()];
        assert_eq!(
            text.string().to_string(),
            java,
            "code-start Backspace corrupted opening fence"
        );
        let after_boundary: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(after_boundary.location, "```java\n".len());
        text.setSelectedRange(code_end);
        let keyword = color_at(&text, position(&java, "public"));
        let string = color_at(&text, position(&java, "한글"));
        let same: bool = msg_send![&*keyword, isEqual:&*string];
        assert!(
            !same,
            "Java keywords and strings have no syntax distinction"
        );
        let popup = code_language_button(&text);
        let old_selection: NSRange = msg_send![&*text, selectedRange];
        let _: () = msg_send![&*popup, selectItemWithTitle:&*NSString::from_str("Plain Text")];
        let _: () = msg_send![&*popup, changeLanguage:std::ptr::null::<AnyObject>()];
        let plain = text.string().to_string();
        assert_eq!(
            plain,
            java.replacen("```java", "```text", 1),
            "language choice changed code content"
        );
        let plain_keyword = color_at(&text, position(&plain, "public"));
        let plain_string = color_at(&text, position(&plain, "한글"));
        let same: bool = msg_send![&*plain_keyword, isEqual:&*plain_string];
        assert!(same, "Plain Text kept stale language coloring");
        let new_selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(
            new_selection, old_selection,
            "same-length language change moved selection"
        );
        let popup = code_language_button(&text);
        let _: () = msg_send![&*popup, selectItemWithTitle:&*NSString::from_str("Python")];
        let _: () = msg_send![&*popup, changeLanguage:std::ptr::null::<AnyObject>()];
        let python = text.string().to_string();
        assert_eq!(python, java.replacen("```java", "```python", 1));
        let shifted_selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(
            shifted_selection.location,
            old_selection.location + 2,
            "language length change lost code caret"
        );
        let popup = code_language_button(&text);
        let _: () = msg_send![&*popup, selectItemWithTitle:&*NSString::from_str("Java")];
        let _: () = msg_send![&*popup, changeLanguage:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), java);
        let popup = code_language_button(&text);
        text.setEditable(false);
        let _: () = msg_send![&*popup, selectItemWithTitle:&*NSString::from_str("Rust")];
        let _: () = msg_send![&*popup, changeLanguage:std::ptr::null::<AnyObject>()];
        assert_eq!(
            text.string().to_string(),
            java,
            "read-only code language was modified"
        );
        text.setEditable(true);
        editor::style(&text);
        let copies = text
            .subviews()
            .iter()
            .filter(|view| {
                let copy: bool = msg_send![&**view,isKindOfClass:class!(StologyCodeCopyButton)];
                copy
            })
            .count();
        assert_eq!(copies, 1, "code copy action missing or duplicated");
        // Do not write the real user's clipboard from this API smoke process.
        let _: () = msg_send![&*text, insertLineBreak:std::ptr::null::<AnyObject>()];
        native_insert(&text, "코드 밖");
        assert_eq!(text.string().to_string(), format!("{java}\n\n코드 밖"));
        let full = text.string().to_string();
        assert_eq!(font_at(&text, position(&full, "코드 밖")).pointSize(), 16.0);

        for (source, expected) in [
            ("**한😀**", "**한**"),
            ("**한**", ""),
            ("`😀`", ""),
            ("~~한~~", ""),
            ("***😀***", ""),
            ("**가👨‍👩‍👧‍👦**", "**가**"),
            (
                "[한😀](https://example.com/a_(b))",
                "[한](https://example.com/a_(b))",
            ),
        ] {
            text.setString(&NSString::from_str(source));
            text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
            editor::style(&text);
            let _: () = msg_send![&*text, breakUndoCoalescing];
            let _: () = msg_send![&*undo, removeAllActions];
            let _: () = msg_send![&*text, deleteBackward:std::ptr::null::<AnyObject>()];
            assert_eq!(
                text.string().to_string(),
                expected,
                "inline Backspace corrupted {source:?}"
            );
            let _: () = msg_send![&*undo, undo];
            assert_eq!(
                text.string().to_string(),
                source,
                "inline undo corrupted {source:?}"
            );
        }
        let inline = "앞 **한😀** 뒤";
        text.setString(&NSString::from_str(inline));
        editor::style(&text);
        for (from, backwards, expected) in
            [(9, true, 5), (7, false, 10), (4, true, 1), (2, false, 5)]
        {
            text.setSelectedRange(NSRange::new(from, 0));
            if backwards {
                let _: () = msg_send![&*text, moveLeft:std::ptr::null::<AnyObject>()];
            } else {
                let _: () = msg_send![&*text, moveRight:std::ptr::null::<AnyObject>()];
            }
            let moved: NSRange = msg_send![&*text, selectedRange];
            assert_eq!(
                moved.location, expected,
                "inline arrow from {from} left={backwards} entered hidden syntax"
            );
            assert_eq!(text.string().to_string(), inline);
        }

        let task = "- [x] 첫 항목";
        text.setString(&NSString::from_str(task));
        text.setSelectedRange(NSRange::new(task.encode_utf16().count(), 0));
        editor::style(&text);
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- [x] 첫 항목\n- [ ] ");
        let _: () = msg_send![&*text, insertNewline:std::ptr::null::<AnyObject>()];
        assert_eq!(text.string().to_string(), "- [x] 첫 항목\n");
        let _: () = msg_send![&*text, breakUndoCoalescing];
        let _: () = msg_send![&*undo, removeAllActions];

        let body = "# 한글 제목\n\n**굵게** 😀 [링크](https://example.com)\n";
        text.setString(&NSString::from_str(body));
        text.setSelectedRange(NSRange::new(3, 2));
        editor::style(&text);
        assert_eq!(text.string().to_string(), body);
        let selection: NSRange = msg_send![&*text, selectedRange];
        assert_eq!((selection.location, selection.length), (3, 2));
        let manager: *mut AnyObject = msg_send![&*text, textLayoutManager];
        assert!(!manager.is_null(), "TextKit 2 manager absent");
        assert!(editor::locate_paragraph(&text, "굵게", 2));
        assert!(!editor::locate_paragraph(&text, "굵게", 1));

        let body = "# **굵게 *기울임***\n\n![원본](pixel.png)\n\n| 열 | 값 |\n|---|---|\n| 한글 | 1 |\n\n- [ ] 확인\n";
        text.setString(&NSString::from_str(body));
        assert!(window.makeFirstResponder(Some(&text)));
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style_with_base(&text, Some(&directory));
        assert_eq!(text.string().to_string(), body);
        let nested = font_at(&text, position(body, "기울임"));
        assert_eq!(nested.pointSize(), 30.0, "nested heading font size changed");
        let traits = NSFontManager::sharedFontManager(mtm).traitsOfFont(&nested);
        assert!(
            traits.contains(NSFontTraitMask::BoldFontMask)
                && traits.contains(NSFontTraitMask::ItalicFontMask)
        );
        let table_start = position(body, "| 열");
        let task_start = position(body, "[ ]");
        assert!(
            font_at(&text, table_start).pointSize() < 0.1,
            "inactive table source remains visible"
        );
        assert!(
            font_at(&text, task_start).pointSize() < 0.1,
            "inactive task syntax remains visible"
        );
        text.setSelectedRange(NSRange::new(table_start, 0));
        editor::style_with_base(&text, Some(&directory));
        assert_eq!(
            table_count(&text),
            0,
            "active table has duplicate rendered grid"
        );
        assert!(
            font_at(&text, table_start).pointSize() > 10.0,
            "active table syntax hidden"
        );
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style_with_base(&text, Some(&directory));
        let table = text
            .subviews()
            .iter()
            .find(|v| {
                let result: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownTable)];
                result
            })
            .unwrap();
        let _: () = msg_send![&*table,mouseDown:std::ptr::null::<NSEvent>()];
        let clicked: NSRange = msg_send![&*text, selectedRange];
        assert_eq!(
            clicked.location, table_start,
            "table click mapped to wrong source block"
        );
        // No manual style/timer tick here: the mouse callback must reveal the
        // source before an immediate Korean composition event can arrive.
        assert_eq!(table_count(&text), 0, "table reveal deferred until timer");
        assert!(
            font_at(&text, table_start).pointSize() > 10.0,
            "table source still concealed immediately after click"
        );
        assert_eq!(
            text.subviews()
                .iter()
                .filter(|v| {
                    let image: bool = msg_send![&**v,isKindOfClass:class!(NSImageView)];
                    image
                })
                .count(),
            1,
            "relative image base lost during click restyle"
        );
        assert_eq!(text.string().to_string(), body);
        let attrs = text.typingAttributes();
        let typing_font: Retained<NSFont> = msg_send![&*attrs,objectForKey:NSFontAttributeName];
        assert!(
            typing_font.pointSize() > 10.0,
            "concealed font leaked into typing attributes"
        );
        text.setSelectedRange(NSRange::new(task_start, 0));
        editor::style_with_base(&text, Some(&directory));
        assert_eq!(button_count(&text), 1, "active task must keep one checkbox");
        assert!(
            font_at(&text, task_start).pointSize() < 0.1,
            "active task reveals source marker"
        );
        text.setSelectedRange(NSRange::new(0, 0));
        editor::style_with_base(&text, Some(&directory));
        assert_eq!(
            text.string().to_string(),
            body,
            "selection-driven styles changed Markdown"
        );
        let image_count = text
            .subviews()
            .iter()
            .filter(|v| {
                let result: bool = msg_send![&**v,isKindOfClass:class!(NSImageView)];
                result
            })
            .count();
        assert_eq!(image_count, 1);
        let table_count = text
            .subviews()
            .iter()
            .filter(|v| {
                let result: bool = msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownTable)];
                result
            })
            .count();
        assert_eq!(table_count, 1);
        let buttons: Vec<_> = text
            .subviews()
            .iter()
            .filter(|v| {
                let result: bool = msg_send![&**v,isKindOfClass:class!(NSButton)];
                result
            })
            .collect();
        assert_eq!(buttons.len(), 1);
        let _: () = msg_send![&*buttons[0],performClick:std::ptr::null::<AnyObject>()];
        assert!(text.string().to_string().contains("- [x] 확인"));
        let undo: Option<Retained<AnyObject>> = msg_send![&*text, undoManager];
        let undo = undo.expect("native undo manager");
        let _: () = msg_send![&*undo, undo];
        assert_eq!(text.string().to_string(), body);
        text.setEditable(false);
        let _: () = msg_send![&*buttons[0],performClick:std::ptr::null::<AnyObject>()];
        assert_eq!(
            text.string().to_string(),
            body,
            "immutable evidence changed"
        );
        text.setEditable(true);

        // Exercise marked-text APIs; this is not a real Korean IME keystroke run.
        text.setSelectedRange(NSRange::new(0, 0));
        let _: () = msg_send![&*text,setMarkedText:&*NSString::from_str("한"),selectedRange:NSRange::new(1,0),replacementRange:NSRange::new(usize::MAX,0)];
        let marked: bool = msg_send![&*text, hasMarkedText];
        assert!(marked);
        let composing = text.string().to_string();
        let range: NSRange = msg_send![&*text, markedRange];
        editor::style(&text);
        assert_eq!(text.string().to_string(), composing);
        let after: NSRange = msg_send![&*text, markedRange];
        assert_eq!(
            (range.location, range.length),
            (after.location, after.length)
        );
        let _: () = msg_send![&*text, unmarkText];

        // Open actual native image and generated PDF views in the same process.
        let image_view = editor::preview(png.to_str().unwrap(), frame, None).unwrap();
        editor::close_preview(&image_view);
        let pdf = directory.join("source.pdf");
        let page = NSView::initWithFrame(NSView::alloc(mtm), frame);
        assert!(page
            .dataWithPDFInsideRect(frame)
            .writeToFile_atomically(&NSString::from_str(pdf.to_str().unwrap()), true));
        let pdf_view = editor::preview(pdf.to_str().unwrap(), frame, Some(1)).unwrap();
        editor::close_preview(&pdf_view);
        assert!(
            editor::preview(directory.join("missing.pdf").to_str().unwrap(), frame, None).is_err()
        );
        {
            let fixture="# 세포와 에너지\n\n엽록체는 광합성이 일어나는 세포 소기관이다.\n\n빛이 있을 때 엽록체에서 광합성이 일어난다.\n\n- [x] **광합성**과 세포 호흡 비교\n- 미토콘드리아는 ATP를 생성한다.\n\n| 개념 | 역할 |\n|---|---|\n| 엽록체 | 광합성 |\n\n> 원본과 근거를 함께 기록합니다.";
            text.setString(&NSString::from_str(fixture));
            text.setSelectedRange(NSRange::new(fixture.encode_utf16().count(), 0));
            window.makeFirstResponder(Some(&text));
            editor::style(&text);
            scroll.layoutSubtreeIfNeeded();
            let rect_at = |needle: &str| {
                let index = position(fixture, needle);
                let screen: NSRect = msg_send![&*text,firstRectForCharacterRange:NSRange::new(index,1),actualRange:std::ptr::null_mut::<NSRange>()];
                text.convertRect_fromView(window.convertRectFromScreen(screen), None)
            };
            let checkbox = text
                .subviews()
                .iter()
                .find(|v| {
                    let yes: bool = msg_send![&**v,isKindOfClass:class!(NSButton)];
                    yes
                })
                .unwrap();
            let table = text
                .subviews()
                .iter()
                .find(|v| {
                    let yes: bool =
                        msg_send![&**v,isKindOfClass:class!(StologyInlineMarkdownTable)];
                    yes
                })
                .unwrap();
            let checklist = rect_at("광합성**과");
            let preceding = rect_at("미토콘드리아는");
            let following = rect_at("원본과 근거");
            println!(
                "GEOMETRY checkbox={:?} checklist={:?} preceding={:?} table={:?} following={:?}",
                checkbox.frame(),
                checklist,
                preceding,
                table.frame(),
                following
            );
            assert!(
                (checkbox.frame().origin.y - checklist.origin.y).abs() < 6.0,
                "checkbox placed on another paragraph"
            );
            assert!(
                table.frame().origin.y >= preceding.origin.y + preceding.size.height,
                "table overlaps preceding visible line"
            );
            assert!(
                table.frame().origin.y + table.frame().size.height <= following.origin.y,
                "table overlaps following visible line"
            );
            assert_eq!(text.string().to_string(), fixture);
            if let Ok(output) = std::env::var("STOLOGY_EDITOR_CAPTURE") {
                let bitmap = scroll
                    .bitmapImageRepForCachingDisplayInRect(scroll.bounds())
                    .expect("offscreen bitmap");
                scroll.cacheDisplayInRect_toBitmapImageRep(scroll.bounds(), &bitmap);
                let png = bitmap
                    .representationUsingType_properties(
                        NSBitmapImageFileType::PNG,
                        &NSDictionary::new(),
                    )
                    .expect("PNG export");
                assert!(png.writeToFile_atomically(&NSString::from_str(&output), true));
                println!("CAPTURE: {output}");
            }
        }
        // Verify actual TextKit geometry, including a wrapped list item. Extra
        // item/paragraph spacing must not become spacing on every wrapped row.
        {
            let fixture = format!(
                "### 첫 제목\n본문 첫째\n본문 둘째\n\n1. 첫 항목\n2. 두 번째 {}끝표시\n- [ ] 확인 항목\n### 다음 제목\n마지막 본문\n```rust\nlet value = 1;\n```\n",
                "한글과 이모지 😀로 길게 이어지는 설명 ".repeat(12),
            );
            text.setString(&NSString::from_str(&fixture));
            text.setSelectedRange(NSRange::new(fixture.encode_utf16().count(), 0));
            editor::style(&text);
            scroll.layoutSubtreeIfNeeded();
            let rect_at = |needle: &str| {
                let screen: NSRect = msg_send![&*text,firstRectForCharacterRange:NSRange::new(position(&fixture, needle),1),actualRange:std::ptr::null_mut::<NSRange>()];
                text.convertRect_fromView(window.convertRectFromScreen(screen), None)
            };
            let paragraph_at = |needle: &str| -> Retained<NSParagraphStyle> {
                let storage = text.textStorage().unwrap();
                msg_send![&*storage,attribute:NSParagraphStyleAttributeName,atIndex:position(&fixture, needle),effectiveRange:std::ptr::null_mut::<NSRange>()]
            };
            let body_pitch = rect_at("본문 둘째").origin.y - rect_at("본문 첫째").origin.y;
            let item_pitch = rect_at("두 번째").origin.y - rect_at("첫 항목").origin.y;
            assert!((body_pitch - 36.0).abs() < 0.1, "body pitch: {body_pitch}");
            assert!((item_pitch - 30.0).abs() < 0.1, "list pitch: {item_pitch}");
            let wrap_height = rect_at("끝표시").origin.y - rect_at("두 번째").origin.y;
            assert!(wrap_height >= 26.0);
            assert!((wrap_height / 26.0 - (wrap_height / 26.0).round()).abs() < 0.01, "wrapped list height: {wrap_height}");
            assert_eq!(paragraph_at("첫 제목").paragraphSpacingBefore(), 0.0);
            assert_eq!(paragraph_at("다음 제목").paragraphSpacingBefore(), 24.0);
            assert_eq!(paragraph_at("다음 제목").paragraphSpacing(), 8.0);
            assert_eq!(paragraph_at("두 번째").paragraphSpacing(), 4.0);
            assert_eq!(paragraph_at("확인 항목").paragraphSpacing(), 0.0);
            assert_eq!(paragraph_at("\n1. 첫 항목").paragraphSpacing(), 0.0);
            assert_eq!(paragraph_at("let value").paragraphSpacing(), 0.0);
            assert_eq!(text.string().to_string(), fixture);
            println!("PASS: B note spacing; body pitch={body_pitch}, item pitch={item_pitch}, wrapped height={wrap_height}; heading/blank/task/code boundaries and source preserved.");
        }
        {
            let source = "본문\n### ";
            text.setString(&NSString::from_str(source));
            let selection = NSRange::new(source.encode_utf16().count(), 0);
            text.setSelectedRange(selection);
            editor::style(&text);
            let placeholder = text.subviews().into_iter()
                .find(|view| {
                    let yes: bool = msg_send![&**view,isKindOfClass:class!(StologyHeadingPlaceholder)];
                    yes
                })
                .expect("empty heading placeholder");
            let screen: NSRect = msg_send![&*text,firstRectForCharacterRange:selection,actualRange:std::ptr::null_mut::<NSRange>()];
            let caret = text.convertRect_fromView(window.convertRectFromScreen(screen), None);
            assert!((placeholder.frame().origin.y - caret.origin.y).abs() < 6.0,
                "heading placeholder {:?} does not follow its caret {caret:?}", placeholder.frame());
            println!("PASS: empty heading placeholder follows spaced heading caret.");
        }
        text.setString(&NSString::from_str(""));
        editor::style(&text);
        let remaining = text
            .subviews()
            .iter()
            .filter(|v| {
                let button: bool = msg_send![&**v,isKindOfClass:class!(NSButton)];
                let image: bool = msg_send![&**v,isKindOfClass:class!(NSImageView)];
                button || image
            })
            .count();
        assert_eq!(remaining, 0, "blank document retained overlays");
        window.setContentView(None);
        std::fs::remove_dir_all(directory).unwrap();
        println!("PASS: native TextKit2; immediate heading input/Backspace/undo; heading Enter; code fence/Tab/ShiftEnter; language menu and color updates; task continuation/exit; Unicode/selection; paragraph evidence; active/inactive table and task presentation; click-to-source; nested heading traits; local image/table source retention; checkbox/undo/read-only guard; marked-text API guard; image/PDF preview; blank overlay cleanup.");
        println!(
            "NOT TESTED: real keyboard Korean IME, PPTX Quick Look rendering, visual layout QA."
        );
    }
}
