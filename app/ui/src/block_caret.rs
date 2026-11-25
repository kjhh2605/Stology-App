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
            let byte = if let Some(table) = child.downcast_ref::<InlineTable>() {
                byte_for_utf16(&self.string().to_string(), table.ivars().range.get().location)
            } else if let Some(image) = child.downcast_ref::<InlineImage>() {
                image.ivars().link.bytes.start
            } else {
                continue;
            };
            // The page margins are not a column, and a sliver beside a wide
            // attachment is not room for one.
            let right = frame.origin.x + frame.size.width;
            let edge = self.bounds().size.width - EDITOR_INSET;
            let beside = (point.x >= right && point.x < edge && edge - right >= MIN_COLUMN_WIDTH / 2.)
                || (point.x < frame.origin.x && point.x >= EDITOR_INSET && frame.origin.x - EDITOR_INSET >= MIN_COLUMN_WIDTH / 2.);
            if beside && point.y >= frame.origin.y && point.y < frame.origin.y + frame.size.height {
                return self.place_beside(byte, frame, point);
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
        // An unfinished last line can sit beside an attachment. Ending it puts
        // the caret line below the row, where the click was.
        let (note, placed) = note_rows(&source);
        if !source.ends_with('\n') && placed.last().is_some_and(|row| row.side == Some(note.len() - 1) && !is_attachment(&note[note.len() - 1])) {
            let floor = bottom.max(last.origin.y + last.size.height);
            let rows = 1 + ((point.y - floor) / NOTE_LINE_HEIGHT).floor().max(0.) as usize;
            self.breakUndoCoalescing();
            self.insert_block_text(&NSString::from_str(&ending.repeat(rows)), end);
            self.breakUndoCoalescing();
            self.setSelectedRange(NSRange::new(end.location + rows * ending.len(), 0));
            return true;
        }
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
        if rows > 0 {
            self.breakUndoCoalescing();
            // A real empty paragraph gets its own horizontal position. The
            // extra EOF fragment inherits the preceding wrapped line’s inset.
            self.insert_block_text(&NSString::from_str(&ending.repeat(rows)), end);
            self.breakUndoCoalescing();
        }
        self.setSelectedRange(NSRange::new(end.location + rows * ending.len(), 0));
        true
    }

    /// A click in the empty column beside an attachment writes there. An
    /// attachment without a side block gets a blank one; following text stays below.
    unsafe fn place_beside(&self, byte: usize, frame: NSRect, point: NSPoint) -> bool {
        let Some(window) = self.window() else {
            return false;
        };
        let source = self.string().to_string();
        let (note, rows) = note_rows(&source);
        let Some(index) = note_blocks::block_at(&note, byte) else {
            return false;
        };
        let Some(row) = rows.iter().find(|r| r.media == index || r.side == Some(index)) else {
            return false;
        };
        if let Some(side) = row.side {
            // Two attachments fill the row. Text beside one keeps native clicks
            // on its lines; below them the caret goes to the end of that block.
            if rows.iter().any(|r| r.media == side) || row.side == Some(index) || is_attachment(&note[side]) {
                return true;
            }
            let content = source[note[side].bytes.clone()].trim_end_matches(['\r', '\n']);
            let end = source[..note[side].bytes.start + content.len()].encode_utf16().count();
            let extra_line = note[side].bytes.end == source.len() && source.ends_with('\n');
            let below = content.is_empty()
                || paragraph_text_bottom(self, end - 1, extra_line).is_none_or(|bottom| point.y >= bottom);
            if !below || !window.makeFirstResponder(Some(self)) {
                return false;
            }
            self.setSelectedRange(NSRange::new(end, 0));
            return true;
        }
        let placement = if point.x < frame.origin.x { Placement::Right } else { Placement::Left };
        let Some(mut result) = with_placement(&source, &note[index], placement, frame.size.width) else {
            return false;
        };
        if !window.makeFirstResponder(Some(self)) {
            return false;
        }
        let ending = if source.contains("\r\n") { "\r\n" } else { "\n" };
        // The first blank line after an attachment is its Markdown separator,
        // so the side block is the line after that one.
        let mut end = result.len() - (source.len() - note[index].bytes.end);
        let block = &result[note[index].bytes.start..end];
        let missing = match block.strip_suffix('\n').map(|rest| rest.strip_suffix('\r').unwrap_or(rest)) {
            None => 2,
            Some(rest) if rest.ends_with('\n') => 0,
            Some(_) => 1,
        };
        result.insert_str(end, &ending.repeat(missing + 1));
        end += missing * ending.len();
        commit_source(self, &result, Some(end));
        true
    }
}

fn is_attachment(block: &note_blocks::Block) -> bool {
    matches!(block.kind, note_blocks::Kind::Image | note_blocks::Kind::Table)
}
