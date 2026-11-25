// Block-level edits shared by attachment placement, block handles and block
// selection. Every edit is one native undo step that touches only what changed.

/// Blocks of the note and the rows its placed attachments form.
fn note_rows(source: &str) -> (Vec<note_blocks::Block>, Vec<note_blocks::Row>) {
    use note_blocks::Kind;
    let note = note_structure(source);
    let images = image_edit::markdown::images(source);
    let tables = table_previews(source);
    let rows = note_blocks::rows(&note, |index| match note[index].kind {
        Kind::Image => images.iter().find(|i| note[index].bytes.contains(&i.bytes.start)).map(image_edit::placement),
        Kind::Table => tables.iter().find(|t| note[index].bytes.contains(&t.bytes.start)).map(InlineTableSpec::placement),
        _ => None,
    });
    (note, rows)
}

/// `wrap` opens the column beside a table; `side` moves the table to the right one.
fn set_table_placement(layout: &mut serde_json::Value, placement: Placement) {
    let Some(values) = layout.as_object_mut() else {
        return;
    };
    values.remove("wrap");
    values.remove("side");
    if placement != Placement::Full {
        values.insert("wrap".into(), true.into());
    }
    if placement == Placement::Right {
        values.insert("side".into(), "right".into());
    }
}

/// The grid comment that stores a table's placement with its other layout.
fn table_placed(table: &InlineTableSpec, placement: Placement) -> (Range<usize>, String) {
    let mut layout = table.layout.clone();
    set_table_placement(&mut layout, placement);
    let cell = table.cell_ranges[0][0].start;
    let marker = table.marker.clone().unwrap_or(cell..cell);
    let empty = layout.as_object().is_none_or(|values| values.is_empty());
    (marker, if empty { String::new() } else { format!("<!--stology-grid {layout}-->") })
}

/// The note with one attachment block moved to another placement.
fn with_placement(source: &str, block: &note_blocks::Block, placement: Placement, width: f64) -> Option<String> {
    let (range, markup) = match block.kind {
        note_blocks::Kind::Image => {
            let link = image_edit::markdown::images(source).into_iter().find(|i| block.bytes.contains(&i.bytes.start))?;
            let markup = image_edit::placed(&link, link.width.unwrap_or(width), placement);
            (link.bytes, markup)
        }
        note_blocks::Kind::Table => {
            let table = table_previews(source).into_iter().find(|t| block.bytes.contains(&t.bytes.start))?;
            table_placed(&table, placement)
        }
        _ => return None,
    };
    let mut result = source.to_owned();
    result.replace_range(range, &markup);
    Some(result)
}

/// Bottom of the text lines of the paragraph at `index`, without the spacing
/// below it. `extra_line` skips the caret line TextKit adds at the note's end.
unsafe fn paragraph_text_bottom(text: &NSTextView, index: usize, extra_line: bool) -> Option<f64> {
    let fragment = layout_fragment(text, index)?;
    let lines = fragment.textLineFragments();
    let last = lines.count().checked_sub(1 + usize::from(extra_line && lines.count() > 1))?;
    let bounds = lines.objectAtIndex(last).typographicBounds();
    Some(text.textContainerOrigin().y + fragment.layoutFragmentFrame().origin.y + bounds.origin.y + bounds.size.height)
}

/// Replace the note with `result` as one undoable edit of the changed span.
/// `caret` is a byte offset in `result`.
unsafe fn commit_source(text: &NSTextView, result: &str, caret: Option<usize>) -> bool {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else {
        return false;
    };
    let marked: bool = msg_send![text, hasMarkedText];
    let source = text.string().to_string();
    let (span, replacement) = note_blocks::edit_span(&source, result);
    if marked || !text.isEditable() || (span.is_empty() && replacement.is_empty()) {
        return false;
    }
    text.breakUndoCoalescing();
    editor.insert_block_text(&NSString::from_str(replacement), utf16_range(&source, span));
    if let Some(caret) = caret {
        text.setSelectedRange(NSRange::new(result[..caret].encode_utf16().count(), 0));
    }
    text.breakUndoCoalescing();
    true
}
