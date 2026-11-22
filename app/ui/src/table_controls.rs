const EMPTY_NOTE_TABLE: &str = "|  |  |\n| --- | --- |\n|  |  |\n\n";

unsafe fn table_shortcut(text: &NSTextView, event: &NSEvent) -> bool {
    use NSEventModifierFlags as Flags;
    let modifiers = Flags::Command | Flags::Option | Flags::Shift | Flags::Control;
    if event.keyCode() != 0 || event.modifierFlags() & modifiers != Flags::Command | Flags::Shift
    {
        return false;
    }
    let focused = text
        .window()
        .and_then(|w| w.firstResponder())
        .is_some_and(|r| std::ptr::eq(Retained::as_ptr(&r).cast::<NSTextView>(), text));
    let marked: bool = msg_send![text, hasMarkedText];
    if !focused || !text.isEditable() || marked {
        return false;
    }
    let selection: NSRange = msg_send![text, selectedRange];
    let source = text.string().to_string();
    let start = byte_for_utf16(&source, selection.location);
    let end = byte_for_utf16(&source, selection.location + selection.length);
    let mut block = String::new();
    if start > 0 && !source[..start].ends_with("\n\n") {
        if !source[..start].ends_with('\n') {
            block.push('\n');
        }
        block.push('\n');
    }
    block.push_str(EMPTY_NOTE_TABLE);
    text.breakUndoCoalescing();
    let _: () = msg_send![text, insertText:&*NSString::from_str(&block), replacementRange:utf16_range(&source,start..end)];
    text.breakUndoCoalescing();
    style_with_base(text, None);
    true
}

thread_local! {
    /// 끌고 있는 손잡이(열 여부, 표 시작 위치). 미리보기로 뷰를 다시 만들어도 그 손잡이는 계속 보인다.
    static TABLE_DRAG: std::cell::Cell<Option<(bool, usize)>> = const { std::cell::Cell::new(None) };
}
struct TableDragGuard(Retained<NSWindow>);
impl Drop for TableDragGuard {
    fn drop(&mut self) {
        TABLE_DRAG.set(None);
        self.0.enableCursorRects();
        self.0.resetCursorRects();
    }
}

struct TableHandleState {
    columns: bool,
    wash: Option<Retained<AnyObject>>,
}
define_class!(
    #[unsafe(super = NSButton)]
    #[name = "StologyTableHandle"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableHandleState]
    struct TableHandle;
    impl TableHandle {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            // The cursor surface is exactly the area that reveals the handle.
            self.addCursorRect_cursor(self.visibleRect(), &self.resize_cursor());
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe { track_table_size(self, event); } }
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) { self.reveal(true); }
        #[unsafe(method(cursorUpdate:))]
        fn cursor_update(&self, _event: &NSEvent) { self.resize_cursor().set(); }
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) { self.reveal(false); }
        #[unsafe(method(viewDidMoveToWindow))]
        fn moved_to_window(&self) { unsafe {
            // 새로 만든 뷰는 mouseEntered를 받지 못하므로 포인터가 이미 위에 있는지 직접 본다.
            let Some(window) = self.window() else { return };
            let point = self.convertPoint_fromView(window.mouseLocationOutsideOfEventStream(), None);
            let size = self.bounds().size;
            let inside = point.x >= 0. && point.y >= 0. && point.x < size.width && point.y < size.height;
            let dragging = TABLE_DRAG.get().is_some_and(|(columns, at)| {
                columns == self.ivars().columns && self.superview().is_some_and(|parent| {
                    (*(Retained::as_ptr(&parent) as *const InlineTable)).ivars().range.get().location == at
                })
            });
            if inside || dragging { self.reveal(true); }
        }}
    }
);
impl TableHandle {
    fn resize_cursor(&self) -> Retained<NSCursor> {
        if self.ivars().columns {
            NSCursor::resizeLeftRightCursor()
        } else {
            NSCursor::resizeUpDownCursor()
        }
    }

    fn reveal(&self, visible: bool) {
        if let Some(wash) = &self.ivars().wash {
            let _: () = unsafe { msg_send![&**wash, setOpacity: if visible { 1f32 } else { 0f32 }] };
        }
    }
}

unsafe fn table_handle_gradient(frame: NSRect, colors: &[Retained<NSColor>], stops: &[f64], horizontal: bool) -> Option<Retained<AnyObject>> {
    let g: Retained<AnyObject> = msg_send![objc2::runtime::AnyClass::get(c"CAGradientLayer")?, layer];
    let cg: Vec<_> = colors.iter().map(|c| c.CGColor()).collect();
    let refs: Vec<&AnyObject> = cg.iter().map(|c| &*(Retained::as_ptr(c) as *const AnyObject)).collect();
    let locations: Vec<_> = stops.iter().map(|&s| objc2_foundation::NSNumber::new_f64(s)).collect();
    let _: () = msg_send![&g, setColors: &*objc2_foundation::NSArray::from_slice(&refs)];
    let _: () = msg_send![&g, setLocations: &*objc2_foundation::NSArray::from_retained_slice(&locations)];
    let (start, end) = if horizontal { ((0., 0.5), (1., 0.5)) } else { ((0.5, 0.), (0.5, 1.)) };
    let _: () = msg_send![&g, setStartPoint: NSPoint::new(start.0, start.1)];
    let _: () = msg_send![&g, setEndPoint: NSPoint::new(end.0, end.1)];
    let _: () = msg_send![&g, setFrame: frame];
    Some(g)
}

/// 손잡이의 라벤더 워시. 길이 방향으로는 페리윙클에서 라벤더로 고르게 흐르고,
/// 표와 맞닿는 쪽의 가는 선에서 시작해 표에서 멀어질수록 옅어진다.
unsafe fn table_handle_wash(size: NSSize, columns: bool) -> Option<Retained<AnyObject>> {
    use objc2::runtime::AnyClass;
    let bounds = NSRect::new(NSPoint::new(0., 0.), size);
    let tint = |rgb: (f64, f64, f64), a: f64| NSColor::colorWithSRGBRed_green_blue_alpha(rgb.0 / 255., rgb.1 / 255., rgb.2 / 255., a);
    let ink = |a: f64| NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., a);
    let (first, mid, last) = ((126., 136., 240.), (150., 140., 238.), (176., 148., 240.));
    let fill = table_handle_gradient(bounds, &[tint(first, 0.36), tint(mid, 0.32), tint(last, 0.36)], &[0., 0.5, 1.], !columns)?;
    let ends = table_handle_gradient(bounds, &[ink(0.), ink(1.), ink(1.), ink(0.)], &[0., 0.08, 0.92, 1.], !columns)?;
    let away = table_handle_gradient(bounds, &[ink(1.), ink(0.55), ink(0.18), ink(0.)], &[0., 0.35, 0.7, 1.], columns)?;
    let line_frame = NSRect::new(NSPoint::new(0., 0.), if columns { NSSize::new(1.5, size.height) } else { NSSize::new(size.width, 1.5) });
    let line = table_handle_gradient(line_frame, &[tint(first, 0.), tint(first, 1.), tint(mid, 1.), tint(last, 1.), tint(last, 0.)],
        &[0., 0.12, 0.5, 0.88, 1.], !columns)?;
    let _: () = msg_send![&fill, setMask: &*ends];
    let plus: Retained<AnyObject> = msg_send![AnyClass::get(c"CATextLayer")?, layer];
    let font = NSFont::systemFontOfSize(15.);
    let _: () = msg_send![&plus, setString: &*NSString::from_str("+")];
    let _: () = msg_send![&plus, setFont: Retained::as_ptr(&font) as *const std::ffi::c_void];
    let _: () = msg_send![&plus, setFontSize: 15f64];
    let _: () = msg_send![&plus, setAlignmentMode: &*NSString::from_str("center")];
    let _: () = msg_send![&plus, setContentsScale: 2f64];
    let _: () = msg_send![&plus, setForegroundColor: &*tint((90., 84., 198.), 1.).CGColor()];
    // 옅어지는 바깥쪽에 묻히지 않도록 `+`를 표 쪽으로 당긴다.
    let pull = if columns { (-4., 0.) } else { (0., -4.) };
    let _: () = msg_send![&plus, setFrame: NSRect::new(
        NSPoint::new((size.width - 18.) / 2. + pull.0, (size.height - 18.) / 2. + pull.1), NSSize::new(18., 18.))];
    let faded: Retained<AnyObject> = msg_send![AnyClass::get(c"CALayer")?, layer];
    let _: () = msg_send![&faded, setFrame: bounds];
    let _: () = msg_send![&faded, addSublayer: &*fill];
    let _: () = msg_send![&faded, addSublayer: &*line];
    let _: () = msg_send![&faded, setMask: &*away];
    let wash: Retained<AnyObject> = msg_send![AnyClass::get(c"CALayer")?, layer];
    let _: () = msg_send![&wash, setFrame: bounds];
    let _: () = msg_send![&wash, addSublayer: &*faded];
    let _: () = msg_send![&wash, addSublayer: &*plus];
    let _: () = msg_send![&wash, setOpacity: 0f32];
    Some(wash)
}

unsafe fn add_table_handle(view: &InlineTable, columns: bool, frame: NSRect) {
    let wash = table_handle_wash(frame.size, columns);
    let allocated = TableHandle::alloc(view.mtm()).set_ivars(TableHandleState { columns, wash: wash.clone() });
    let handle: Retained<TableHandle> = msg_send![super(allocated), initWithFrame:frame];
    handle.setTitle(&NSString::from_str(""));
    handle.setBordered(false);
    let label = if columns { "열 조절" } else { "행 조절" };
    let _: () = msg_send![&handle, setAccessibilityLabel: &*NSString::from_str(label)];
    handle.setWantsLayer(true);
    if let Some(wash) = wash {
        let layer: Retained<AnyObject> = msg_send![&handle, layer];
        let _: () = msg_send![&layer, addSublayer: &*wash];
    }
    // 손잡이는 표 가장자리에 포인터를 올렸을 때만 나타난다.
    let area = NSTrackingArea::initWithRect_options_owner_userInfo(
        NSTrackingArea::alloc(), handle.bounds(),
        NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::CursorUpdate | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect,
        Some(&handle), None);
    handle.addTrackingArea(&area);
    view.addSubview(&handle);
}

struct TableSizePreview {
    layout: Option<serde_json::Value>,
    start: usize,
    rows: usize,
    columns: usize,
}

impl TableSizePreview {
    fn apply(&self, table: &mut InlineTableSpec) {
        if let Some(layout) = &self.layout { table.layout.clone_from(layout); }
        // Preview cells are transient. Their source slots become real only on commit.
        table.rows.resize_with(self.rows, Vec::new);
        table.cell_ranges.resize_with(self.rows, Vec::new);
        for (row, ranges) in table.rows.iter_mut().zip(&mut table.cell_ranges) {
            row.resize_with(self.columns, String::new);
            ranges.resize(self.columns, self.start..self.start);
        }
    }
}

unsafe fn track_table_size(handle: &TableHandle, event: &NSEvent) {
    let Some(parent) = handle.superview() else {
        return;
    };
    let table = &*(Retained::as_ptr(&parent) as *const InlineTable);
    let Some(text) = table.ivars().text.load() else {
        return;
    };
    let Some(window) = handle.window() else {
        return;
    };
    if !text.isEditable() || !window.makeFirstResponder(Some(&text)) {
        return;
    }
    let source = text.string().to_string();
    let range = table.ivars().range.get();
    let Some(spec) = table_previews(&source)
        .into_iter()
        .find(|t| utf16_range(&source, t.bytes.clone()) == range)
    else {
        return;
    };
    let columns = handle.ivars().columns;
    TABLE_DRAG.set(Some((columns, range.location)));
    // The preview rebuilds the text view's children during tracking. Suspend
    // cursor rectangles so NSTextView cannot restore its I-beam mid-drag.
    window.disableCursorRects();
    handle.resize_cursor().set();
    let _dragging = TableDragGuard(window.clone());
    let initial = if columns {
        spec.rows[0].len()
    } else {
        spec.rows.len()
    };
    let limit = if columns { 16 } else { 100 };
    // 한 칸만큼 끌면 한 칸이 늘도록, 열은 현재 열 너비를 행은 행 높이를 단위로 쓴다.
    let step = if columns { (handle.frame().origin.x / initial as f64).max(30.0) } else { 30.0 };
    let origin = event.locationInWindow();
    let mask = NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp;
    let mut dragged = false;
    let mut displayed = initial;
    let base = table.ivars().base_directory.as_deref();
    while let Some(next) = window.nextEventMatchingMask(mask) {
        let point = next.locationInWindow();
        let distance = if columns {
            point.x - origin.x
        } else {
            origin.y - point.y
        };
        dragged |= distance.abs() >= 4.0;
        let delta = if dragged {
            (distance / step).round() as isize
        } else {
            1
        };
        let count = initial.saturating_add_signed(delta).clamp(1, limit);
        if next.r#type() != NSEventType::LeftMouseUp {
            if dragged && count != displayed {
                let size = TableSizePreview {
                    layout: None,
                    start: spec.bytes.start,
                    rows: if columns { spec.rows.len() } else { count },
                    columns: if columns { count } else { spec.rows[0].len() },
                };
                layout_markdown(&text, base, Some(size));
                window.displayIfNeeded();
                // The tracking loop can wait for another mouse event before
                // AppKit reaches its ordinary end-of-run-loop layer commit.
                if let Some(transaction) = objc2::runtime::AnyClass::get(c"CATransaction") {
                    let _: () = msg_send![transaction, flush];
                }
                displayed = count;
            }
            continue;
        }
        if initial == count || text.string().to_string() != source {
            break;
        }
        let Some(result) = resize_table_source(&source, &spec, columns, count) else {
            break;
        };
        let selection: NSRange = msg_send![&*text, selectedRange];
        text.breakUndoCoalescing();
        let _: () =
            msg_send![&*text, insertText:&*NSString::from_str(&result), replacementRange:range];
        text.breakUndoCoalescing();
        let offset = if selection.location >= range.location + range.length {
            selection.location - range.length + result.encode_utf16().count()
        } else {
            selection.location.min(range.location)
        };
        text.setSelectedRange(NSRange::new(offset, 0));
        style_with_base(&text, base);
        return;
    }
    if displayed != initial {
        style_with_base(&text, base);
    }
}

fn resize_table_source(
    source: &str,
    spec: &InlineTableSpec,
    columns: bool,
    count: usize,
) -> Option<String> {
    let raw = &source[spec.bytes.clone()];
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = raw.lines().map(str::to_owned).collect();
    if lines.len() != spec.rows.len() + 1 {
        return None;
    }
    if columns {
        for (index, line) in lines.iter_mut().enumerate() {
            let mut parts = Vec::new();
            let mut start = 0;
            let mut escaped = false;
            for (at, ch) in line.char_indices() {
                if ch == '|' && !escaped {
                    parts.push(&line[start..at]);
                    start = at + 1;
                }
                escaped = ch == '\\' && !escaped;
            }
            parts.push(&line[start..]);
            if parts.first().is_some_and(|s| s.trim().is_empty()) {
                parts.remove(0);
            }
            if parts.last().is_some_and(|s| s.trim().is_empty()) {
                parts.pop();
            }
            parts.resize(count, if index == 1 { " --- " } else { "  " });
            *line = format!("|{}|", parts.join("|"));
        }
    } else {
        let blank = format!("|{}", "  |".repeat(spec.rows[0].len()));
        lines.resize(count + 1, blank);
    }
    let mut result = lines.join(newline);
    if raw.ends_with('\n') {
        result.push_str(newline);
    }
    Some(result)
}

#[cfg(test)]
#[test]
fn resize_preserves_markdown_and_edits_empty_grid() {
    let raw = "| **제목** | B |\r\n| :--- | ---: |\r\n| a\\|b | 😀 |\r\n";
    let spec = &table_previews(raw)[0];
    let larger = resize_table_source(raw, spec, true, 4).unwrap();
    let expanded = &table_previews(&larger)[0];
    assert_eq!(expanded.rows[1], ["a|b", "😀", "", ""]);
    assert_eq!(
        resize_table_source(&larger, expanded, true, 2).unwrap(),
        raw
    );
    let empty = "|  |  |\n| --- | --- |\n|  |  |\n";
    let spec = &table_previews(empty)[0];
    assert_eq!(spec.rows.len(), 2);
    let one_row = resize_table_source(empty, spec, false, 1).unwrap();
    assert_eq!(
        resize_table_source(&one_row, &table_previews(&one_row)[0], false, 2).unwrap(),
        empty
    );
}
