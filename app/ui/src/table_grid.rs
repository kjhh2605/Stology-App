// The first header cell can carry a Markdown HTML comment with presentation
// overrides. Cell text edits exclude that comment from their source ranges.
impl InlineTableSpec {
    fn column_width(&self, c: usize, fallback: f64) -> f64 {
        self.layout
            .get(format!("w{c}"))
            .and_then(|v| v.as_f64())
            .map(|v| v.clamp(12., 4000.))
            .unwrap_or(fallback)
    }
    fn row_height(&self, r: usize) -> f64 {
        let lines = self.rows[r]
            .iter()
            .map(|s| s.lines().count())
            .max()
            .unwrap_or(1);
        self.layout
            .get(format!("h{r}"))
            .and_then(|v| v.as_f64())
            .unwrap_or(30.)
            .clamp(20., 4000.)
            .max(8. + lines as f64 * 17.)
    }
    fn alignment(&self, r: usize, c: usize) -> NSTextAlignment {
        match self
            .layout
            .get(format!("a{r},{c}"))
            .and_then(|v| v.as_i64())
        {
            Some(0) => NSTextAlignment::Left,
            Some(1) => NSTextAlignment::Center,
            Some(2) => NSTextAlignment::Right,
            _ => match self.alignments.get(c) {
                Some(pulldown_cmark::Alignment::Center) => NSTextAlignment::Center,
                Some(pulldown_cmark::Alignment::Right) => NSTextAlignment::Right,
                _ => NSTextAlignment::Left,
            },
        }
    }
}

fn after_grid_line(source: &str, at: usize) -> bool {
    let before = &source[..at];
    before.ends_with('\n')
        && before
            .trim_end_matches(['\r', '\n'])
            .lines()
            .last()
            .is_some_and(|l| l.contains('|'))
        && !before.ends_with("\n\n")
        && !before.ends_with("\r\n\r\n")
        && table_previews(before)
            .last()
            .is_some_and(|t| t.bytes.end == at)
}

define_class!(
    #[unsafe(super = NSTextFieldCell)]
    #[name = "StologyGridFieldCell"]
    #[thread_kind = MainThreadOnly]
    struct GridFieldCell;
    impl GridFieldCell {
        #[unsafe(method(selectWithFrame:inView:editor:delegate:start:length:))]
        fn select_editor(&self,frame:NSRect,view:&NSView,editor:&NSText,delegate:Option<&AnyObject>,start:isize,length:isize) {unsafe {
            let mut length = length;
            if let Some(field) = view.downcast_ref::<TableCell>().filter(|field|field.isEditable()) {
                let raw = field.ivars().markdown.borrow();
                let select_all = start == 0 && length >= field.stringValue().length() as isize;
                self.setStringValue(&NSString::from_str(&raw));
                field.ivars().editing_markdown.set(true);
                field.ivars().needs_layout.set(true);
                if select_all {length = raw.encode_utf16().count() as isize;}
            }
            let _:() = msg_send![super(self),selectWithFrame:frame,inView:view,editor:editor,delegate:delegate,start:start,length:length];
            if let (Some(field),Some(text)) = (view.downcast_ref::<TableCell>(), editor.downcast_ref::<NSTextView>()) {
                restyle_grid_editor(field,text);
            }
        }}

        #[unsafe(method(drawingRectForBounds:))]
        fn drawing_rect(&self, bounds:NSRect) -> NSRect { unsafe {
            let mut rect:NSRect = msg_send![super(self), drawingRectForBounds:bounds];
            let height = self.cellSizeForBounds(bounds).height.min(rect.size.height);
            rect.origin.y += (rect.size.height - height) * 0.5;
            rect.size.height = height;
            rect
        }}
    }
);

impl InlineTable {
    fn cells(&self) -> Vec<Retained<TableCell>> {
        self.subviews()
            .into_iter()
            .filter_map(|v| v.downcast::<TableCell>().ok())
            .collect()
    }
    fn select(&self, a: (usize, usize), b: (usize, usize)) {
        for cell in self.cells() {
            let (r, c) = cell.position();
            let selected = (a.0.min(b.0)..=a.0.max(b.0)).contains(&r)
                && (a.1.min(b.1)..=a.1.max(b.1)).contains(&c);
            cell.ivars().selected.set(selected);
            let (red, green, blue) = if selected {
                (0.8, 0.84, 1.)
            } else if r == 0 {
                (0.91, 0.92, 0.99)
            } else {
                (0.97, 0.975, 0.99)
            };
            cell.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                red, green, blue, 1.,
            )));
        }
    }
}
impl InlineTable {
    /// Every cell goes back to its resting color.
    fn clear_selection(&self) {
        if self.cells().iter().any(|cell| cell.ivars().selected.get()) {
            self.select((usize::MAX, usize::MAX), (usize::MAX, usize::MAX));
        }
    }
}
impl TableCell {
    fn position(&self) -> (usize, usize) {
        (self.ivars().row, self.ivars().column)
    }
    unsafe fn grid(&self) -> Option<Retained<InlineTable>> {
        self.superview()?.downcast().ok()
    }
}

unsafe fn track_grid_selection(cell: &TableCell) {
    let (Some(view), Some(window)) = (cell.grid(), cell.window()) else {
        return;
    };
    if !window.makeFirstResponder(Some(&view)) {
        return;
    }
    let anchor = cell.position();
    view.select(anchor, anchor);
    let cells = view.cells();
    while let Some(event) =
        window.nextEventMatchingMask(NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp)
    {
        let p = view.convertPoint_fromView(event.locationInWindow(), None);
        if let Some(end) = cells.iter().find(|c| {
            let f = c.frame();
            p.x >= f.origin.x
                && p.y >= f.origin.y
                && p.x < f.origin.x + f.size.width
                && p.y < f.origin.y + f.size.height
        }) {
            view.select(anchor, end.position());
        }
        if event.r#type() == NSEventType::LeftMouseUp {
            break;
        }
    }
}

unsafe fn move_grid_selection(view: &InlineTable, key: u16) -> bool {
    let cells = view.cells();
    let Some(first) = cells.iter().find(|c| c.ivars().selected.get()) else {
        return false;
    };
    let (r, c) = first.position();
    let next = match key {
        123 => (r, c.saturating_sub(1)),
        124 => (r, c + 1),
        125 => (r + 1, c),
        126 => (r.saturating_sub(1), c),
        _ => return false,
    };
    if let Some(cell) = cells.iter().find(|c| c.position() == next) {
        view.select(next, next);
        view.scrollRectToVisible(cell.frame());
    }
    true
}

// During editing AppKit routes clicks directly to the window's shared field
// editor. Consume a context click inside its table cell before focus changes.
unsafe fn editing_grid_cell_at_click(click: &NSEvent) -> Option<Retained<TableCell>> {
    if click.r#type() != NSEventType::RightMouseDown
        && !click
            .modifierFlags()
            .contains(NSEventModifierFlags::Control)
    {
        return None;
    }
    let window = click.window(MainThreadMarker::new()?)?;
    let editor = window.firstResponder()?.downcast::<NSTextView>().ok()?;
    if !editor.isFieldEditor() {
        return None;
    }
    let delegate: Option<Retained<AnyObject>> = msg_send![&*editor, delegate];
    let cell = delegate?.downcast::<TableCell>().ok()?;
    let point = cell.convertPoint_fromView(click.locationInWindow(), None);
    cell.mouse_inRect(point, cell.visibleRect()).then_some(cell)
}

unsafe fn install_grid_context_filter() {
    thread_local! {
        static MONITOR: std::cell::OnceCell<Option<Retained<AnyObject>>> = const { std::cell::OnceCell::new() };
    }
    MONITOR.with(|monitor| {
        monitor.get_or_init(|| {
            let handler = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| {
                let event_ref = event.as_ref();
                let handled = if event_ref.r#type() == NSEventType::KeyDown {
                    grid_format_shortcut(event_ref)
                } else {
                    editing_grid_cell_at_click(event_ref).is_some()
                };
                if handled {
                    std::ptr::null_mut()
                } else {
                    event.as_ptr()
                }
            });
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(
                NSEventMask::RightMouseDown | NSEventMask::LeftMouseDown | NSEventMask::KeyDown,
                &handler,
            )
        });
    });
}

unsafe fn grid_menu(cell: &TableCell, event: &NSEvent) {
    if cell.currentEditor().is_some() {
        return;
    }
    let Some(view) = cell.grid() else { return };
    if !view.ivars().text.load().is_some_and(|t| t.isEditable()) {
        return;
    }
    if let Some(w) = view.window() {
        w.makeFirstResponder(Some(&view));
    }
    if !cell.ivars().selected.get() {
        view.select(cell.position(), cell.position());
    }
    let menu = NSMenu::new(view.mtm());
    menu.setAllowsContextMenuPlugIns(false);
    for (tag, (title, symbol)) in [
        ("왼쪽 정렬", "text.alignleft"),
        ("가운데 정렬", "text.aligncenter"),
        ("오른쪽 정렬", "text.alignright"),
    ]
    .into_iter()
    .enumerate()
    {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(view.mtm()),
            &NSString::from_str(title),
            Some(sel!(gridAlign:)),
            &NSString::from_str(""),
        );
        let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(symbol),
            None,
        );
        item.setImage(icon.as_deref());
        item.setTarget(Some(cell));
        item.setTag(tag as isize);
        menu.addItem(&item);
    }
    // Where the whole table sits in its row.
    let source = view.ivars().text.load().map(|t| t.string().to_string()).unwrap_or_default();
    let current = table_previews(&source)
        .into_iter()
        .find(|t| utf16_range(&source, t.bytes.clone()) == view.ivars().range.get())
        .map(|t| t.placement());
    menu.addItem(&NSMenuItem::separatorItem(view.mtm()));
    for (tag, (title, placement)) in [
        ("표를 한 줄 전체로", Placement::Full),
        ("표를 왼쪽에 배치", Placement::Left),
        ("표를 오른쪽에 배치", Placement::Right),
    ]
    .into_iter()
    .enumerate()
    {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(view.mtm()),
            &NSString::from_str(title),
            Some(sel!(gridPlace:)),
            &NSString::from_str(""),
        );
        item.setTarget(Some(cell));
        item.setTag(tag as isize);
        item.setState(if current == Some(placement) { 1 } else { 0 });
        menu.addItem(&item);
    }
    let point = cell.convertPoint_fromView(event.locationInWindow(), None);
    menu.popUpMenuPositioningItem_atLocation_inView(None, point, Some(cell));
}
unsafe fn apply_grid_alignment(cell: &TableCell, tag: isize) {
    let Some(view) = cell.grid() else { return };
    save_grid_layout(&view, |layout| {
        for c in view.cells().iter().filter(|c| c.ivars().selected.get()) {
            let (r, c) = c.position();
            layout[format!("a{r},{c}")] = tag.into();
        }
    });
}

unsafe fn apply_grid_placement(cell: &TableCell, tag: isize) {
    let Some(view) = cell.grid() else { return };
    let placement = [Placement::Full, Placement::Left, Placement::Right][tag.clamp(0, 2) as usize];
    save_grid_layout(&view, |layout| set_table_placement(layout, placement));
}

unsafe fn save_grid_layout(view: &InlineTable, edit: impl FnOnce(&mut serde_json::Value)) {
    let Some(text) = view.ivars().text.load() else {
        return;
    };
    let source = text.string().to_string();
    let Some(mut spec) = table_previews(&source)
        .into_iter()
        .find(|t| utf16_range(&source, t.bytes.clone()) == view.ivars().range.get())
    else {
        return;
    };
    edit(&mut spec.layout);
    let start = spec.cell_ranges[0][0].start;
    let range = utf16_range(&source, spec.marker.unwrap_or(start..start));
    let marker = format!("<!--stology-grid {}-->", spec.layout);
    let selection: NSRange = msg_send![&*text, selectedRange];
    let selected = view
        .cells()
        .into_iter()
        .filter(|c| c.ivars().selected.get())
        .map(|c| c.position())
        .collect::<Vec<_>>();
    let delta = marker.encode_utf16().count() as isize - range.length as isize;
    text.breakUndoCoalescing();
    let _: () = msg_send![&*text, insertText:&*NSString::from_str(&marker), replacementRange:range];
    let at = if selection.location >= range.location + range.length {
        selection.location.saturating_add_signed(delta)
    } else {
        selection.location
    };
    text.setSelectedRange(NSRange::new(at, 0));
    text.breakUndoCoalescing();
    style_with_base(&text, view.ivars().base_directory.as_deref());
    if let (Some(a), Some(b)) = (selected.first(), selected.last()) {
        if let Some(grid) = text
            .subviews()
            .into_iter()
            .filter_map(|v| v.downcast::<InlineTable>().ok())
            .find(|v| v.ivars().range.get().location == view.ivars().range.get().location)
        {
            grid.select(*a, *b);
            if let Some(w) = grid.window() {
                w.makeFirstResponder(Some(&grid));
            }
        }
    }
}

struct DividerState {
    columns: bool,
    index: usize,
}
define_class!(
    #[unsafe(super = NSView)]
    #[name = "StologyGridDivider"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DividerState]
    struct GridDivider;
    impl GridDivider {
        #[unsafe(method(resetCursorRects))]
        fn cursor(&self) {
            let cursor = if self.ivars().columns { NSCursor::resizeLeftRightCursor() } else { NSCursor::resizeUpDownCursor() };
            self.addCursorRect_cursor(self.bounds(), &cursor);
        }
        #[unsafe(method(mouseDown:))]
        fn down(&self,event:&NSEvent) {unsafe { resize_grid_pair(self,event); }}
    }
);
const DIVIDER_HIT_WIDTH: f64 = 10.;

unsafe fn add_grid_dividers(view: &InlineTable, widths: &[f64], spec: &InlineTableSpec) {
    for columns in [true, false] {
        let sizes = if columns {
            widths.to_vec()
        } else {
            (0..spec.rows.len()).map(|r| spec.row_height(r)).collect()
        };
        let mut offset = 0.;
        for (index, size) in sizes.iter().take(sizes.len().saturating_sub(1)).enumerate() {
            offset += size;
            let frame = if columns {
                NSRect::new(
                    NSPoint::new(offset - DIVIDER_HIT_WIDTH / 2., 0.),
                    NSSize::new(DIVIDER_HIT_WIDTH, spec.height()),
                )
            } else {
                NSRect::new(
                    NSPoint::new(0., offset - DIVIDER_HIT_WIDTH / 2.),
                    NSSize::new(widths.iter().sum(), DIVIDER_HIT_WIDTH),
                )
            };
            let alloc = GridDivider::alloc(view.mtm()).set_ivars(DividerState { columns, index });
            let divider: Retained<GridDivider> = msg_send![super(alloc),initWithFrame:frame];
            view.addSubview(&divider);
        }
    }
}
unsafe fn resize_grid_pair(divider: &GridDivider, event: &NSEvent) {
    let Some(grid) = divider
        .superview()
        .and_then(|v| v.downcast::<InlineTable>().ok())
    else {
        return;
    };
    let Some(window) = grid.window() else { return };
    if !window.makeFirstResponder(Some(&grid)) {
        return;
    }
    let columns = divider.ivars().columns;
    let index = divider.ivars().index;
    let Some(text) = grid.ivars().text.load() else {
        return;
    };
    let source = text.string().to_string();
    let Some(spec) = table_previews(&source)
        .into_iter()
        .find(|t| utf16_range(&source, t.bytes.clone()) == grid.ivars().range.get())
    else {
        return;
    };
    let sizes: Vec<f64> = grid
        .cells()
        .into_iter()
        .filter(|c| {
            if columns {
                c.ivars().row == 0
            } else {
                c.ivars().column == 0
            }
        })
        .map(|c| {
            if columns {
                c.frame().size.width + 1.
            } else {
                c.frame().size.height + 1.
            }
        })
        .collect();
    let a = sizes[index];
    let b = sizes[index + 1];
    let origin = event.locationInWindow();
    let base = grid.ivars().base_directory.as_deref();
    let prefix = if columns { "w" } else { "h" };
    window.disableCursorRects();
    let _guard = TableDragGuard(window.clone());
    let cursor = if columns {
        NSCursor::resizeLeftRightCursor()
    } else {
        NSCursor::resizeUpDownCursor()
    };
    cursor.set();
    let mut previous = 0.;
    while let Some(next) =
        window.nextEventMatchingMask(NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp)
    {
        let point = next.locationInWindow();
        let delta = if columns {
            (point.x - origin.x).clamp(24_f64.min(a) - a, b - 24_f64.min(b))
        } else {
            let min_height = spec.rows[index]
                .iter()
                .map(|s| s.lines().count())
                .max()
                .unwrap_or(1) as f64
                * 17.
                + 8.;
            (origin.y - point.y).clamp(25_f64.max(min_height).min(a) - a, (4000. - a).max(0.))
        };
        let mut layout = spec.layout.clone();
        if columns {
            for (i, width) in sizes.iter().enumerate() {
                layout[format!("w{i}")] = (*width).into();
            }
            layout[format!("w{}", index + 1)] = (b - delta).into();
        }
        layout[format!("{prefix}{index}")] = (a + delta).into();
        if next.r#type() == NSEventType::LeftMouseUp {
            if delta != 0. && text.string().to_string() == source {
                save_grid_layout(&grid, |v| *v = layout);
            } else if previous != 0. {
                style_with_base(&text, base);
            }
            break;
        }
        if delta != previous {
            layout_markdown(
                &text,
                base,
                Some(TableSizePreview {
                    start: spec.bytes.start,
                    rows: spec.rows.len(),
                    columns: spec.rows[0].len(),
                    layout: Some(layout),
                }),
            );
            window.displayIfNeeded();
            previous = delta;
        }
    }
}

#[test]
fn grid_formatting_survives_markdown_round_trip() {
    let source = "| <!--stology-grid {\"w0\":170,\"h0\":80,\"a1,1\":1}-->**제목** | 값 |\n| --- | --- |\n| A<br>B | C |\n";
    let spec = table_previews(source).remove(0);
    assert_eq!(spec.rows[0][0], "제목");
    assert_eq!(&source[spec.cell_ranges[0][0].clone()], "**제목**");
    assert_eq!(spec.rows[1][0], "A\nB");
    assert_eq!(spec.column_width(0, 140.), 170.);
    assert_eq!(spec.row_height(0), 80.);
    assert_eq!(spec.alignment(1, 1), NSTextAlignment::Center);
    assert!(after_grid_line(source, source.len()));
    assert!(!after_grid_line("plain | text\n", 13));
}

unsafe fn render_grid_cell(cell: &TableCell, table: &InlineTableSpec) {
    let (row, column) = cell.position();
    let Some(contents) = table.rows.get(row).and_then(|r| r.get(column)) else {
        return;
    };
    *cell.ivars().original_display.borrow_mut() = contents.clone();
    cell.setStringValue(&NSString::from_str(contents));
    let Some(runs) = table.styles.get(&(row, column)) else {
        return;
    };
    let styled = objc2_foundation::NSMutableAttributedString::initWithString(
        objc2_foundation::NSMutableAttributedString::alloc(),
        &NSString::from_str(contents),
    );
    let all = NSRange::new(0, styled.length());
    let font = if row == 0 {
        crate::font::bold(13.)
    } else {
        crate::font::regular(13.)
    };
    styled.addAttribute_value_range(NSFontAttributeName, &font, all);
    let para = NSMutableParagraphStyle::new();
    para.setAlignment(table.alignment(row, column));
    styled.addAttribute_value_range(NSParagraphStyleAttributeName, &para, all);
    if let Some(color) = cell.textColor() {
        styled.addAttribute_value_range(NSForegroundColorAttributeName, &color, all);
    }
    for run in runs {
        style_grid_run(
            &styled,
            utf16_range(contents, run.bytes.clone()),
            &run.treatment,
        );
    }
    cell.setAttributedStringValue(&styled);
}

fn finish_grid_run(
    open: &mut Vec<(Treatment, usize)>,
    runs: &mut Vec<Span>,
    kind: Treatment,
    end: usize,
) {
    if let Some(i) = open.iter().rposition(|(k, _)| *k == kind) {
        let (_, start) = open.remove(i);
        if start < end {
            runs.push(Span {
                bytes: start..end,
                treatment: kind,
            });
        }
    }
}

unsafe fn style_grid_run(
    storage: &objc2_foundation::NSMutableAttributedString,
    range: NSRange,
    kind: &Treatment,
) {
    match kind {
        Treatment::Bold => apply_font_trait(storage, range, NSFontTraitMask::BoldFontMask),
        Treatment::Italic => apply_font_trait(storage, range, NSFontTraitMask::ItalicFontMask),
        Treatment::Underline | Treatment::Strike => {
            let key = if *kind == Treatment::Underline {
                NSUnderlineStyleAttributeName
            } else {
                NSStrikethroughStyleAttributeName
            };
            storage.addAttribute_value_range(key, &objc2_foundation::NSNumber::new_isize(1), range);
        }
        Treatment::Code => {
            storage.addAttribute_value_range(
                NSFontAttributeName,
                &NSFont::monospacedSystemFontOfSize_weight(13., 0.),
                range,
            );
            storage.addAttribute_value_range(
                NSBackgroundColorAttributeName,
                &NSColor::quaternaryLabelColor(),
                range,
            );
        }
        _ => {}
    }
}

fn grid_editable_source(raw: &str) -> String {
    let mut source = raw.to_owned();
    if raw.contains('<') {
        let breaks: Vec<_> = Parser::new(raw)
            .into_offset_iter()
            .filter_map(|(event, range)| match event {
                Event::InlineHtml(tag)
                    if matches!(
                        tag.to_ascii_lowercase().as_str(),
                        "<br>" | "<br/>" | "<br />"
                    ) =>
                {
                    Some(range)
                }
                _ => None,
            })
            .collect();
        for range in breaks.into_iter().rev() {
            source.replace_range(range, "\n");
        }
    }
    source
}

fn grid_encode_source(value: &str) -> String {
    let mut result = String::new();
    let mut escaped = false;
    for ch in value.replace("\r\n", "\n").chars() {
        if ch == '|' && !escaped {
            result.push('\\');
        }
        if matches!(ch, '\r' | '\n') {
            result.push_str("<br>");
        } else {
            result.push(ch);
        }
        escaped = ch == '\\' && !escaped;
    }
    result
}

unsafe fn grid_format_shortcut(event: &NSEvent) -> bool {
    let Some((open, close)) = inline_marks(event) else {
        return false;
    };
    let Some(editor) = event
        .window(MainThreadMarker::new().unwrap())
        .and_then(|w| w.firstResponder())
        .and_then(|r| r.downcast::<NSTextView>().ok())
    else {
        return false;
    };
    if !editor.isFieldEditor() {
        return false;
    }
    let delegate: Option<Retained<AnyObject>> = msg_send![&*editor, delegate];
    let Some(cell) = delegate.and_then(|d| d.downcast::<TableCell>().ok()) else {
        return false;
    };
    let marked: bool = msg_send![&*editor, hasMarkedText];
    if marked {
        return false;
    }
    let selection: NSRange = msg_send![&*editor, selectedRange];
    let source = editor.string().to_string();
    let bytes = byte_for_utf16(&source, selection.location)
        ..byte_for_utf16(&source, selection.location + selection.length);
    if bytes.is_empty() {
        let (tag, end_tag) = match open {
            "**" => ("<b>", "</b>"),
            "*" => ("<i>", "</i>"),
            "<u>" => ("<u>", "</u>"),
            "~~" => ("<s>", "</s>"),
            "`" => ("<code>", "</code>"),
            _ => return false,
        };
        let at = bytes.start;
        if source[at..].starts_with(end_tag) {
            if source[..at].ends_with(tag) {
                apply_source_edit(
                    &editor,
                    markdown_shortcuts::Edit {
                        range: at - tag.len()..at + end_tag.len(),
                        replacement: String::new(),
                        selection: at - tag.len()..at - tag.len(),
                    },
                );
            } else {
                editor
                    .setSelectedRange(utf16_range(&source, at + end_tag.len()..at + end_tag.len()));
            }
        } else {
            let caret = at + tag.len();
            apply_source_edit(
                &editor,
                markdown_shortcuts::Edit {
                    range: bytes,
                    replacement: format!("{tag}{end_tag}"),
                    selection: caret..caret,
                },
            );
        }
        restyle_grid_editor(&cell, &editor);
        return true;
    }
    let html_wrapper = grid_html_runs(&source).into_iter().find(|(kind, span)| {
        span.content == bytes
            && matches!(
                (open, kind),
                ("**", Treatment::Bold)
                    | ("*", Treatment::Italic)
                    | ("~~", Treatment::Strike)
                    | ("<u>", Treatment::Underline)
                    | ("`", Treatment::Code)
            )
    });
    if let Some((_, span)) = html_wrapper {
        let start = span.bytes.start;
        let len = span.content.len();
        apply_source_edit(
            &editor,
            markdown_shortcuts::Edit {
                range: span.bytes,
                replacement: source[span.content].to_owned(),
                selection: start..start + len,
            },
        );
    } else if let Some(edit) = toggle_format(&source, bytes, open, close) {
        apply_source_edit(&editor, edit);
    }
    true
}

unsafe fn restyle_grid_editor(cell: &TableCell, editor: &NSTextView) {
    let marked: bool = msg_send![editor, hasMarkedText];
    if marked {
        return;
    }
    let Some(storage) = editor.textStorage() else {
        return;
    };
    let raw = editor.string().to_string();
    let all = NSRange::new(0, storage.length());
    let font = if cell.ivars().row == 0 {
        crate::font::bold(13.)
    } else {
        crate::font::regular(13.)
    };
    let ink = cell.textColor().unwrap_or_else(NSColor::labelColor);
    storage.beginEditing();
    storage.addAttribute_value_range(NSForegroundColorAttributeName, &ink, all);
    storage.addAttribute_value_range(NSFontAttributeName, &font, all);
    for key in [
        NSStrikethroughStyleAttributeName,
        NSUnderlineStyleAttributeName,
        NSBackgroundColorAttributeName,
    ] {
        storage.removeAttribute_range(key, all);
    }
    for span in spans(&raw) {
        style_grid_run(&storage, utf16_range(&raw, span.bytes), &span.treatment);
    }
    for (kind, span) in grid_html_runs(&raw) {
        style_grid_run(&storage, utf16_range(&raw, span.content), &kind);
    }
    for marker in inline_marker_ranges(&grid_inline_syntax(&raw)) {
        conceal(&storage, utf16_range(&raw, marker), false);
    }
    storage.endEditing();
    // The insertion point can border a concealed delimiter. Keep newly composed
    // text visible until its own formatting pass, including marked Korean text.
    use objc2_foundation::NSMutableCopying;
    let typing = editor.typingAttributes().mutableCopy();
    typing.setObject_forKey(
        &*font,
        objc2::runtime::ProtocolObject::from_ref(NSFontAttributeName),
    );
    typing.setObject_forKey(
        &*ink,
        objc2::runtime::ProtocolObject::from_ref(NSForegroundColorAttributeName),
    );
    editor.setTypingAttributes(&typing);
}

fn grid_html_runs(source: &str) -> Vec<(Treatment, InlineSyntax)> {
    let mut runs = Vec::new();
    let mut active = Vec::new();
    for (event, range) in Parser::new_ext(source, Options::ENABLE_STRIKETHROUGH).into_offset_iter()
    {
        let Event::InlineHtml(tag) = event else {
            continue;
        };
        let Some((kind, closing)) = grid_html_style(&tag) else {
            continue;
        };
        if !closing {
            active.push((kind, range));
        } else if let Some(index) = active.iter().rposition(|(k, _)| *k == kind) {
            let (_, open) = active.remove(index);
            runs.push((
                kind,
                InlineSyntax {
                    bytes: open.start..range.end,
                    content: open.end..range.start,
                },
            ));
        }
    }
    for (kind, open) in active {
        runs.push((
            kind,
            InlineSyntax {
                bytes: open.start..source.len(),
                content: open.end..source.len(),
            },
        ));
    }
    runs
}
fn grid_inline_syntax(source: &str) -> Vec<InlineSyntax> {
    let mut syntax = inline_syntax(source);
    syntax.extend(grid_html_runs(source).into_iter().map(|(_, span)| span));
    syntax
}

fn grid_html_style(tag: &str) -> Option<(Treatment, bool)> {
    let name = tag.strip_prefix('<')?.strip_suffix('>')?;
    let closing = name.starts_with('/');
    let kind = match name.trim_start_matches('/').to_ascii_lowercase().as_str() {
        "i" | "em" => Treatment::Italic,
        "b" | "strong" => Treatment::Bold,
        "u" => Treatment::Underline,
        "s" | "del" => Treatment::Strike,
        "code" => Treatment::Code,
        _ => return None,
    };
    Some((kind, closing))
}

#[test]
fn grid_inline_styles_use_display_offsets() {
    let source = "| 이름 | 설명 |\n| --- | --- |\n| 😀 **굵게** *강조* <u>밑줄</u> ~~삭제~~ `코드` | 일반 |\n";
    let table = table_previews(source).remove(0);
    let display = &table.rows[1][0];
    assert_eq!(display, "😀 굵게 강조 밑줄 삭제 코드");
    let runs = &table.styles[&(1, 0)];
    for (kind, text) in [
        (Treatment::Bold, "굵게"),
        (Treatment::Italic, "강조"),
        (Treatment::Underline, "밑줄"),
        (Treatment::Strike, "삭제"),
        (Treatment::Code, "코드"),
    ] {
        assert!(runs
            .iter()
            .any(|s| s.treatment == kind && &display[s.bytes.clone()] == text));
    }
    let raw = r"**한글** a\|b<br>*둘*";
    assert_eq!(grid_encode_source(&grid_editable_source(raw)), raw);
}

#[test]
fn grid_editing_conceals_markers_but_preserves_literal_code() {
    let raw = "**굵게** <i>기울임</i> <u>선</u> `<i>`";
    let mut visible = raw.to_owned();
    for marker in inline_marker_ranges(&grid_inline_syntax(raw))
        .into_iter()
        .rev()
    {
        visible.replace_range(marker, "");
    }
    assert_eq!(visible, "굵게 기울임 선 <i>");
}
