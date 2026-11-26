pub const DIAGRAM_SYMBOL: &str = "square.on.circle";

impl MarkdownEditor {
    unsafe fn note_insert_menu(&self) -> Option<Retained<NSMenu>> {
        let marked: bool = msg_send![self, hasMarkedText];
        if !self.isEditable() || marked {
            return None;
        }
        let menu = NSMenu::new(self.mtm());
        menu.setAutoenablesItems(false);
        menu.setAllowsContextMenuPlugIns(false);
        let source = self.string().to_string();
        if !self.selected_blocks(&source, &note_structure(&source)).is_empty() {
            self.add_block_menu_items(&menu);
            return Some(menu);
        }
        for (title, symbol, action) in [
            (
                "코드 블록",
                "chevron.left.forwardslash.chevron.right",
                sel!(insertNoteCode:),
            ),
            ("표", "tablecells", sel!(insertNoteTable:)),
            ("다이어그램", DIAGRAM_SYMBOL, sel!(createNoteDiagram:)),
        ] {
            if action == sel!(createNoteDiagram:) && self.ivars().create_diagram.get().is_none() {
                continue;
            }
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(title),
                Some(action),
                &NSString::new(),
            );
            let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(symbol),
                None,
            );
            item.setImage(icon.as_deref());
            item.setTarget(Some(self));
            menu.addItem(&item);
        }
        Some(menu)
    }

    /// Actions for the selected blocks, and asking the study assistant about them.
    unsafe fn add_block_menu_items(&self, menu: &NSMenu) {
        for (title, action, key, mine) in [
            ("잘라내기", sel!(cut:), "", true),
            ("복사", sel!(copy:), "", true),
            ("삭제", sel!(deleteSelectedBlocks:), "", true),
            ("학습도우미에게 묻기", sel!(askAssistant:), "l", false),
        ] {
            if !mine {
                menu.addItem(&NSMenuItem::separatorItem(self.mtm()));
            }
            let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            );
            // The app delegate answers askAssistant: through the responder chain.
            if mine { item.setTarget(Some(self)); }
            menu.addItem(&item);
        }
    }

    unsafe fn insert_block_text(&self, replacement: &NSString, range: NSRange) {
        // This is a complete Markdown edit. Bypass smart typing and multi-selection replacement.
        let _: () = msg_send![super(self), insertText:replacement, replacementRange:range];
    }
}

pub fn set_menu_diagram_action(text: &NSTextView, create: Option<fn()>) {
    if let Some(editor) = text.downcast_ref::<MarkdownEditor>() {
        editor.ivars().create_diagram.set(create);
    }
}

fn note_block_edit(
    source: &str,
    caret: usize,
    block: &str,
    block_caret: usize,
) -> markdown_input::Edit {
    let line_start = source[..caret].rfind('\n').map_or(0, |i| i + 1);
    let line = source[line_start..]
        .split_inclusive('\n')
        .next()
        .unwrap_or("");
    let next = line_start + line.len();
    let blank = line.trim().is_empty();
    let mut start = if blank { line_start } else { next };
    let following = source[start..].split_inclusive('\n').next().unwrap_or("");
    let mut end = if following.trim().is_empty() {
        start + following.len()
    } else {
        start
    };
    let newline = if line.ends_with("\r\n") || (!line.ends_with('\n') && source.contains("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };
    let mut replacement = String::new();
    if !blank && !line.ends_with('\n') {
        replacement.push_str(newline);
    }
    // Split an existing fenced block around a new sibling so the insertion is rendered.
    let enclosing = code_blocks(source).into_iter().find(|b| {
        b.open.end <= start
            && (start < b.close.start || (start == b.close.start && (b.close.is_empty() || !blank)))
            && (b.close.is_empty() || start < b.bytes.end)
    });
    let mut reopen = None;
    if let Some(b) = enclosing {
        if start == b.open.end {
            start = b.open.start;
        } else {
            replacement.push_str(&b.fence);
            replacement.push_str(newline);
        }
        if end < b.close.start {
            reopen = Some(&source[b.open]);
        } else if !b.close.is_empty() {
            // The last code line already has a closing fence. Move it ahead of the new block.
            end = b.close.start
                + source[b.close.start..]
                    .split_inclusive('\n')
                    .next()
                    .unwrap()
                    .len();
        }
    }
    let caret = start + replacement.len() + block[..block_caret].replace('\n', newline).len();
    replacement.push_str(&block.replace('\n', newline));
    if let Some(opener) = reopen {
        replacement.push_str(opener);
    }
    markdown_input::Edit {
        range: start..end,
        replacement,
        caret,
    }
}

unsafe fn insert_note_block(
    text: &NSTextView,
    selection: NSRange,
    block: &str,
    block_caret: usize,
) {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else {
        return;
    };
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return;
    }
    let source = text.string().to_string();
    let edit = note_block_edit(
        &source,
        byte_for_utf16(&source, selection.location),
        block,
        block_caret,
    );
    let range = utf16_range(&source, edit.range.clone());
    let offset = edit.caret - edit.range.start;
    let caret = NSRange::new(
        range.location + edit.replacement[..offset].encode_utf16().count(),
        0,
    );
    if let Some(window) = text.window() {
        window.makeFirstResponder(Some(text));
    }
    text.breakUndoCoalescing();
    editor.insert_block_text(&NSString::from_str(&edit.replacement), range);
    text.setSelectedRange(caret);
    text.scrollRangeToVisible(caret);
    text.breakUndoCoalescing();
}

#[cfg(test)]
mod note_insert_tests {
    use super::*;

    fn insert(source: &str, block: &str, caret_in_block: usize) -> (String, usize) {
        let at = source.find('¦').unwrap();
        let mut source = source.replace('¦', "");
        let edit = note_block_edit(&source, at, block, caret_in_block);
        source.replace_range(edit.range, &edit.replacement);
        (source, edit.caret)
    }

    #[test]
    fn inserts_at_blank_line_or_immediately_below_whole_text_line() {
        for (source, expected) in [
            ("¦", "BLOCK\n"),
            ("앞\n \t¦ \n뒤", "앞\nBLOCK\n뒤"),
            ("한¦😀글\n뒤", "한😀글\nBLOCK\n뒤"),
            ("¦문장\n  \n뒤", "문장\nBLOCK\n뒤"),
            ("마지¦막", "마지막\nBLOCK\n"),
            ("마지막\n¦", "마지막\nBLOCK\n"),
            ("앞\r\n\t¦\r\n뒤", "앞\r\nBLOCK\r\n뒤"),
        ] {
            let (actual, caret) = insert(source, "BLOCK\n", 1);
            assert_eq!(actual, expected);
            assert_eq!(&actual[caret..caret + 4], "LOCK");
        }
    }

    #[test]
    fn menu_blocks_render_between_existing_paragraphs() {
        for input in ["앞¦ 문장\n뒤 문장", "앞 문장\n¦\n뒤 문장"] {
            let (table, caret) = insert(input, EMPTY_NOTE_TABLE, EMPTY_NOTE_TABLE.len());
            assert_eq!(table_previews(&table)[0].rows.len(), 2);
            assert_eq!(&table[caret..], "뒤 문장");
            let (code, caret) = insert(input, "```text\n\n```\n", 8);
            assert_eq!(code_blocks(&code)[0].open.end, caret);
            assert!(code[caret..].starts_with("\n```\n뒤 문장"));
        }
    }

    #[test]
    fn inserting_after_the_last_code_line_does_not_leave_an_empty_panel() {
        let (source, _) = insert(
            "```rust\none¦();\n```\nafter",
            EMPTY_NOTE_TABLE,
            EMPTY_NOTE_TABLE.len(),
        );
        assert_eq!(code_blocks(&source).len(), 1);
        assert_eq!(
            source,
            format!("```rust\none();\n```\n{EMPTY_NOTE_TABLE}after")
        );
    }

    #[test]
    fn table_inserted_while_writing_code_is_a_rendered_sibling() {
        let (source, _) = insert(
            "```rust\none¦();\ntwo();\n```",
            EMPTY_NOTE_TABLE,
            EMPTY_NOTE_TABLE.len(),
        );
        assert_eq!(table_previews(&source).len(), 1);
        assert_eq!(code_blocks(&source).len(), 2);
        assert!(source.contains("```rust\ntwo();"));
    }
}
