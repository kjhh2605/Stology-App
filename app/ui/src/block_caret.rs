// TextKit hit testing can only return a position in the existing string.
// Make blank writing rows addressable without placing the caret in a hidden
// table row, attachment link or closing fence.
// NOTE_LINE_HEIGHT is shared with the editor's body typography.

impl MarkdownEditor {
    unsafe fn place_block_caret(&self, event: &NSEvent) -> bool {
        let marked: bool = msg_send![self, hasMarkedText];
        if !self.isEditable()
            || marked
            || event.clickCount() != 1
            || event.modifierFlags().intersects(
                NSEventModifierFlags::Shift
                    | NSEventModifierFlags::Command
                    | NSEventModifierFlags::Control
                    | NSEventModifierFlags::Option,
            )
        {
            return false;
        }
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        let mut bottom = self
            .ivars()
            .code_panels
            .borrow()
            .iter()
            .map(|r| r.origin.y + r.size.height)
            .reduce(f64::max);
        for child in self.subviews() {
            let frame = child.frame();
            if let Some(table) = child.downcast_ref::<InlineTable>() {
                if point.y >= frame.origin.y
                    && point.y < (frame.origin.y + frame.size.height)
                    && point.x >= (frame.origin.x + frame.size.width)
                    && (frame.origin.x + frame.size.width) + LINE_PADDING * 2.
                        < self.bounds().size.width - EDITOR_INSET
                {
                    return self.place_beside_table(table, point);
                }
            } else if child.downcast_ref::<InlineImage>().is_none() {
                continue;
            }
            bottom = Some(bottom.unwrap_or(0.).max(frame.origin.y + frame.size.height));
        }
        let Some(bottom) = bottom.filter(|b| point.y >= *b) else {
            return false;
        };
        let Some(window) = self.window() else {
            return false;
        };
        if !window.makeFirstResponder(Some(self)) {
            return false;
        }
        let end = NSRange::new(self.string().length(), 0);
        let screen: NSRect = msg_send![self, firstRectForCharacterRange:end actualRange:std::ptr::null_mut::<NSRange>()];
        if screen.size.height <= 0.0 {
            return false;
        }
        let last = self.convertRect_fromView(window.convertRectFromScreen(screen), None);
        if point.y < last.origin.y {
            return false;
        }
        let source = self.string().to_string();
        let ending = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let mut rows = ((point.y - last.origin.y) / NOTE_LINE_HEIGHT).floor() as usize;
        // Imported blocks and the keyboard fence shortcut can end the document
        // without a terminating newline. Add the Markdown boundary before typing.
        if !source.ends_with('\n') {
            if table_previews(&source)
                .last()
                .is_some_and(|t| t.bytes.end == source.len())
            {
                rows = 2 + ((point.y - bottom) / NOTE_LINE_HEIGHT).floor() as usize;
            } else if code_blocks(&source)
                .last()
                .is_some_and(|b| !b.close.is_empty() && b.bytes.end == source.len())
                || image_edit::markdown::images(&source)
                    .last()
                    .is_some_and(|i| i.bytes.end == source.len())
            {
                rows = 1
                    + ((point.y - last.origin.y - last.size.height).max(0.) / NOTE_LINE_HEIGHT)
                        .floor() as usize;
            } else if rows == 0 {
                // The final line contains ordinary text. Let AppKit choose its column.
                return false;
            }
        }
        // The extra EOF line can still be beside a floating attachment even
        // when the pointer is underneath it. Clear the actual exclusion path.
        let mut clear_float = false;
        if let Some(container) = self.textContainer() {
            for path in container.exclusionPaths() {
                let obstacle = path.bounds();
                let clear_y =
                    self.textContainerOrigin().y + obstacle.origin.y + obstacle.size.height;
                if last.origin.y < clear_y {
                    clear_float = true;
                    rows = rows.max(((clear_y - last.origin.y) / NOTE_LINE_HEIGHT).ceil() as usize);
                }
            }
        }
        if rows > 0 {
            self.breakUndoCoalescing();
            // A real empty paragraph gets its own horizontal position. The
            // extra EOF fragment inherits the preceding wrapped line’s inset.
            self.insert_block_text(
                &NSString::from_str(&ending.repeat(rows + usize::from(clear_float))),
                end,
            );
            self.breakUndoCoalescing();
        }
        self.setSelectedRange(NSRange::new(end.location + rows * ending.len(), 0));
        true
    }

    unsafe fn place_beside_table(&self, view: &InlineTable, point: NSPoint) -> bool {
        let Some(window) = self.window() else {
            return false;
        };
        if !window.makeFirstResponder(Some(self)) {
            return false;
        }
        let source = self.string().to_string();
        let Some(mut table) = table_previews(&source)
            .into_iter()
            .find(|t| utf16_range(&source, t.bytes.clone()) == view.ivars().range.get())
        else {
            return false;
        };
        if table.wraps(self, TABLE_HANDLE_SIZE) {
            // Clicking an empty line next to an exclusion path may select the
            // hidden anchor or the rest of the paragraph in NSTextView.
            let start = table
                .separator_line(&source)
                .map_or(table.bytes.end, |r| r.end);
            let mut index = source[..start].encode_utf16().count();
            for line in source[start..].split_inclusive('\n') {
                let Some(frame) = layout_fragment_rect(self, index) else {
                    break;
                };
                if point.y < frame.origin.y {
                    break;
                }
                if point.y < frame.origin.y + frame.size.height {
                    if !line.trim().is_empty() {
                        return false;
                    }
                    self.setSelectedRange(NSRange::new(index, 0));
                    return true;
                }
                index += line.encode_utf16().count();
            }
            return false;
        }
        let ending = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let start = table.bytes.start;
        let end = table
            .separator_line(&source)
            .map_or(table.bytes.end, |r| r.end);
        let mut block = source[start..end].to_owned();
        let cell = table.cell_ranges[0][0].start;
        let marker = table.marker.unwrap_or(cell..cell);
        table.layout["wrap"] = true.into();
        block.replace_range(
            marker.start - start..marker.end - start,
            &format!("<!--stology-grid {}-->", table.layout),
        );
        if !block.ends_with('\n') {
            block.push_str(ending);
        }
        if end == table.bytes.end {
            block.push_str(ending);
        }
        let frame = view.frame();
        let row = ((point.y - frame.origin.y) / NOTE_LINE_HEIGHT).floor() as usize;
        let caret = source[..start].encode_utf16().count()
            + block.encode_utf16().count()
            + row * ending.len();
        // Keep following prose below the table on the first click. These are
        // normal Markdown paragraphs, editable by the main text view and IME.
        let rows = (frame.size.height / NOTE_LINE_HEIGHT).ceil() as usize;
        block.push_str(&ending.repeat(rows));
        self.breakUndoCoalescing();
        self.insert_block_text(
            &NSString::from_str(&block),
            utf16_range(&source, start..end),
        );
        self.setSelectedRange(NSRange::new(caret, 0));
        self.breakUndoCoalescing();
        true
    }
}

// An image or wrapped table can span several empty source lines. At the
// first visual line below it, those empty lines separate the caret from the
// block even though there is no intervening prose.
unsafe fn backspace_below_float(
    text: &NSTextView,
    source: &str,
    at: usize,
    selection: NSRange,
) -> Option<markdown_input::Edit> {
    if !source[..at].ends_with('\n') || text.textContainer()?.exclusionPaths().is_empty() {
        return None;
    }
    let window = text.window()?;
    let screen: NSRect = msg_send![text, firstRectForCharacterRange:selection actualRange:std::ptr::null_mut::<NSRange>()];
    let caret = text.convertRect_fromView(window.convertRectFromScreen(screen), None);
    for child in text.subviews() {
        let frame = child.frame();
        let distance = caret.origin.y - frame.origin.y - frame.size.height;
        if !(-NOTE_LINE_HEIGHT * 0.5..NOTE_LINE_HEIGHT + 8.).contains(&distance) {
            continue;
        }
        let range = if let Some(image) = child.downcast_ref::<InlineImage>() {
            if !image.ivars().link.wrap || image.diagram_link().is_none() {
                continue;
            }
            image.ivars().link.bytes.clone()
        } else if let Some(table) = child.downcast_ref::<InlineTable>() {
            let range = table.ivars().range.get();
            byte_for_utf16(source, range.location)
                ..byte_for_utf16(source, range.location + range.length)
        } else {
            continue;
        };
        if source
            .get(range.end..at)
            .is_some_and(|s| s.bytes().all(|b| matches!(b, b'\r' | b'\n')))
        {
            return Some(markdown_input::Edit {
                range: range.start..at,
                replacement: String::new(),
                caret: range.start,
            });
        }
    }
    None
}
