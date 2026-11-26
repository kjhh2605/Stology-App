// Block handle, block selection and block moves. A block is one line of the
// note, or a whole code block, table or image; see note_blocks.rs.

const BLOCK_HANDLE_SIZE: NSSize = NSSize::new(12.0, 22.0);
/// Space between the handle and its block. The margin left of the handle stays free for block marquee drags.
const BLOCK_HANDLE_GAP: f64 = 1.0;

struct BlockHandleState {
    text: Weak<NSTextView>,
    /// Start byte of the block this handle belongs to.
    block: std::cell::Cell<usize>,
    drag: RefCell<Option<BlockDrag>>,
}

/// A handle drag in progress. The note cannot change until the button is released.
struct BlockDrag {
    origin: NSPoint,
    source: String,
    note: Vec<note_blocks::Block>,
    rows: Vec<note_blocks::Row>,
    boxes: Vec<Option<BlockBox>>,
    moving: Vec<usize>,
    dragged: bool,
    drop: Option<BlockDrop>,
}

/// A selection drag that started in the page margin.
struct BlockMarquee {
    down: Retained<NSEvent>,
    start: NSPoint,
    source: String,
    note: Vec<note_blocks::Block>,
    boxes: Vec<Option<BlockBox>>,
    dragged: bool,
    /// Blocks already selected when a Shift drag began; the drag adds to them.
    base: Vec<Range<usize>>,
}
define_class!(
    #[unsafe(super = NSView)]
    #[name = "StologyBlockHandle"]
    #[thread_kind = MainThreadOnly]
    #[ivars = BlockHandleState]
    struct BlockHandle;
    impl BlockHandle {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool { true }
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::openHandCursor());
        }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.40, 0.42, 0.50, 0.75).setFill();
            for row in 0..3 {
                for column in 0..2 {
                    let dot = NSRect::new(
                        NSPoint::new(2.0 + column as f64 * 5.0, 4.5 + row as f64 * 5.0),
                        NSSize::new(3.0, 3.0),
                    );
                    NSBezierPath::bezierPathWithOvalInRect(dot).fill();
                }
            }
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe { self.begin_drag(event); } }
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) { unsafe { self.continue_drag(event); } }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) { unsafe { self.finish_drag(); } }
    }
);

/// Where a dragged block would land.
#[derive(Clone, Copy, PartialEq)]
enum BlockDrop {
    /// In front of this block (`blocks.len()` for the end of the note).
    Before(usize),
    /// Beside this attachment, which then takes the given placement.
    Beside(usize, Placement),
}

/// A block's outline and the height of its first line.
#[derive(Clone, Copy)]
struct BlockBox {
    rect: NSRect,
    line: f64,
}

impl MarkdownEditor {
    /// Outline of every block in view coordinates. Attachments use their
    /// overlay; text uses its lines without the spacing around them.
    unsafe fn block_boxes(&self, source: &str, note: &[note_blocks::Block]) -> Vec<Option<BlockBox>> {
        let mut overlays = Vec::new();
        for child in self.subviews() {
            if let Some(image) = child.downcast_ref::<InlineImage>() {
                overlays.push((image.ivars().link.bytes.start, image.content_frame()));
            } else if let Some(table) = child.downcast_ref::<InlineTable>() {
                overlays.push((byte_for_utf16(source, table.ivars().range.get().location), child.frame()));
            }
        }
        let sides = self.ivars().sides.borrow();
        let full = (self.bounds().size.width - 2.0 * EDITOR_INSET).max(80.0);
        let origin = self.textContainerOrigin();
        let mut units = 0;
        let mut previous = 0;
        note.iter().enumerate().map(|(index, block)| {
            units += source[previous..block.bytes.start].encode_utf16().count();
            previous = block.bytes.start;
            if is_attachment(block) {
                if let Some((_, rect)) = overlays.iter().find(|(byte, _)| block.bytes.contains(byte)) {
                    return Some(BlockBox { rect: *rect, line: rect.size.height.min(NOTE_LINE_HEIGHT) });
                }
            }
            let content = source[block.bytes.clone()].trim_end_matches(['\r', '\n']);
            let length = content.encode_utf16().count();
            let first = layout_fragment(self, units)?;
            let line = first.textLineFragments().firstObject()?.typographicBounds();
            let top = origin.y + first.layoutFragmentFrame().origin.y + line.origin.y;
            let extra_line = index + 1 == note.len() && source.ends_with('\n');
            let bottom = paragraph_text_bottom(self, units + length.saturating_sub(1), extra_line)?;
            let (x, width) = match sides.iter().find(|s| s.bytes.contains(&block.bytes.start)) {
                Some(s) => (EDITOR_INSET + s.left, s.right.map_or(full - s.left, |right| right + LINE_PADDING)),
                None => (EDITOR_INSET, full),
            };
            Some(BlockBox {
                rect: NSRect::new(NSPoint::new(x, top), NSSize::new(width, (bottom - top).max(1.0))),
                line: line.size.height,
            })
        }).collect()
    }

    /// The block under the pointer. The margin left of a block belongs to it,
    /// so its handle can be reached.
    fn block_at_point(boxes: &[Option<BlockBox>], point: NSPoint) -> Option<usize> {
        let reach = BLOCK_HANDLE_SIZE.width + 8.0;
        let inside = |index: &usize| boxes[*index].is_some_and(|b| {
            point.y >= b.rect.origin.y - 5.0 && point.y < b.rect.origin.y + b.rect.size.height + 5.0
                && point.x >= b.rect.origin.x - reach && point.x < b.rect.origin.x + b.rect.size.width
        });
        // Prefer the narrowest match: an attachment or side block over the row.
        (0..boxes.len()).filter(inside).min_by(|a, b| {
            boxes[*a].unwrap().rect.size.width.total_cmp(&boxes[*b].unwrap().rect.size.width)
        })
    }

    /// Blocks and their outlines, measured once per layout.
    unsafe fn measured_blocks(&self) -> (Vec<note_blocks::Block>, Vec<Option<BlockBox>>) {
        let source = self.string().to_string();
        let print = fingerprint(&source);
        if let Some((_, note, boxes)) = self.ivars().block_cache.borrow().as_ref().filter(|cached| cached.0 == print) {
            return (note.clone(), boxes.clone());
        }
        let note = note_structure(&source);
        let boxes = self.block_boxes(&source, &note);
        *self.ivars().block_cache.borrow_mut() = Some((print, note.clone(), boxes.clone()));
        (note, boxes)
    }

    unsafe fn block_handle(&self) -> Retained<BlockHandle> {
        if let Some(handle) = self.subviews().into_iter().find_map(|v| v.downcast::<BlockHandle>().ok()) {
            return handle;
        }
        let allocated = BlockHandle::alloc(self.mtm()).set_ivars(BlockHandleState {
            text: Weak::new(self),
            block: Default::default(),
            drag: Default::default(),
        });
        let handle: Retained<BlockHandle> = msg_send![super(allocated), initWithFrame:NSRect::new(NSPoint::ZERO, BLOCK_HANDLE_SIZE)];
        handle.setHidden(true);
        handle.setAccessibilityLabel(Some(&NSString::from_str("블록 손잡이 · 끌어서 이동, 클릭해서 선택")));
        handle.setToolTip(Some(&NSString::from_str("끌어서 이동 · 클릭해서 선택")));
        self.addSubview(&handle);
        handle
    }

    /// Show the handle beside the block under the pointer.
    unsafe fn hover_block(&self, point: Option<NSPoint>) {
        let handle = self.block_handle();
        let marked: bool = msg_send![self, hasMarkedText];
        let Some(point) = point.filter(|_| self.isEditable() && !marked && !image_edit::interacting() && !table_cell_is_editing(self)) else {
            handle.setHidden(true);
            return;
        };
        let frame = handle.frame();
        if !handle.isHidden() && point.x >= frame.origin.x - 4.0 && point.x < frame.origin.x + frame.size.width + 4.0
            && point.y >= frame.origin.y && point.y < frame.origin.y + frame.size.height {
            return;
        }
        let (note, boxes) = self.measured_blocks();
        match Self::block_at_point(&boxes, point) {
            Some(index) => self.place_block_handle(&handle, note[index].bytes.start, boxes[index].unwrap()),
            None => handle.setHidden(true),
        }
    }

    unsafe fn place_block_handle(&self, handle: &BlockHandle, block: usize, outline: BlockBox) {
        handle.ivars().block.set(block);
        handle.setFrameOrigin(NSPoint::new(
            (outline.rect.origin.x - BLOCK_HANDLE_SIZE.width - BLOCK_HANDLE_GAP).max(2.0),
            outline.rect.origin.y + (outline.line - BLOCK_HANDLE_SIZE.height) / 2.0,
        ));
        handle.setHidden(false);
    }

    /// Indices of the selected blocks, when the note has not changed since.
    fn selected_blocks(&self, source: &str, note: &[note_blocks::Block]) -> Vec<usize> {
        let (print, ranges) = &*self.ivars().selected_blocks.borrow();
        if *print != fingerprint(source) {
            return Vec::new();
        }
        (0..note.len()).filter(|i| ranges.contains(&note[*i].bytes)).collect()
    }

    unsafe fn select_blocks(&self, source: &str, ranges: Vec<Range<usize>>) {
        *self.ivars().selected_blocks.borrow_mut() = (fingerprint(source), ranges);
        self.refresh_blocks();
    }

    /// After any layout: drop a stale selection, redraw the marks and keep
    /// the handle on its block.
    unsafe fn refresh_blocks(&self) {
        let source = self.string().to_string();
        let stale = self.ivars().selected_blocks.borrow().0 != fingerprint(&source);
        let handle = self.subviews().into_iter().find_map(|v| v.downcast::<BlockHandle>().ok());
        let empty = self.ivars().selected_blocks.borrow().1.is_empty();
        if empty && handle.as_ref().is_none_or(|h| h.isHidden()) {
            return;
        }
        let (note, boxes) = self.measured_blocks();
        if stale {
            self.ivars().selected_blocks.borrow_mut().1.clear();
        }
        let marks = self.selected_blocks(&source, &note).into_iter().filter_map(|i| boxes[i].map(|b| b.rect)).collect();
        *self.ivars().block_marks.borrow_mut() = marks;
        if let Some(handle) = handle.filter(|h| !h.isHidden()) {
            let block = handle.ivars().block.get();
            match note.iter().position(|b| b.bytes.start == block).and_then(|i| boxes[i]) {
                Some(outline) => self.place_block_handle(&handle, block, outline),
                None => handle.setHidden(true),
            }
        }
        self.setNeedsDisplay(true);
    }

    unsafe fn clear_block_selection(&self) -> bool {
        if self.ivars().selected_blocks.borrow().1.is_empty() {
            return false;
        }
        self.ivars().selected_blocks.borrow_mut().1.clear();
        self.ivars().block_marks.borrow_mut().clear();
        self.setNeedsDisplay(true);
        true
    }

    /// The selected blocks as Markdown, for the pasteboard.
    unsafe fn copy_selected_blocks(&self) -> bool {
        let source = self.string().to_string();
        let note = note_structure(&source);
        let selected = self.selected_blocks(&source, &note);
        if selected.is_empty() {
            return false;
        }
        let text: String = selected.iter().map(|i| &source[note[*i].bytes.clone()]).collect();
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(&text), NSPasteboardTypeString);
        true
    }

    unsafe fn delete_selected_blocks(&self) -> bool {
        let source = self.string().to_string();
        let note = note_structure(&source);
        let selected = self.selected_blocks(&source, &note);
        let Some((result, _)) = note_blocks::reordered(&source, &note, &selected, None) else {
            return false;
        };
        let caret = note[selected[0]].bytes.start.min(result.len());
        self.clear_block_selection();
        commit_source(self, &result, Some(caret))
    }

    fn draw_block_marks(&self) {
        let tint = NSColor::colorWithSRGBRed_green_blue_alpha(0.31, 0.275, 0.898, 0.13);
        tint.setFill();
        for mark in self.ivars().block_marks.borrow().iter() {
            let outline = NSRect::new(
                NSPoint::new(mark.origin.x - 3.0, mark.origin.y - 3.0),
                NSSize::new(mark.size.width + 6.0, mark.size.height + 6.0),
            );
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(outline, 5.0, 5.0).fill();
        }
        if let Some(area) = self.ivars().block_marquee.get() {
            tint.setFill();
            NSBezierPath::bezierPathWithRect(area).fill();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.31, 0.275, 0.898, 0.55).setStroke();
            NSBezierPath::bezierPathWithRect(area).stroke();
        }
        if let Some(line) = self.ivars().block_drop.get() {
            NSColor::controlAccentColor().setFill();
            NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(line, 1.0, 1.0).fill();
        }
    }

    /// A drag that starts in the page margin selects every block it passes.
    /// Until the pointer moves it may still be a plain click.
    unsafe fn begin_block_marquee(&self, event: &NSEvent) -> bool {
        let marked: bool = msg_send![self, hasMarkedText];
        let start = self.convertPoint_fromView(event.locationInWindow(), None);
        let margin = start.x < EDITOR_INSET - 2.0 || start.x > self.bounds().size.width - EDITOR_INSET + 2.0;
        let flags = event.modifierFlags();
        if !margin || !self.isEditable() || marked || event.clickCount() != 1
            || flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option)
        {
            return false;
        }
        let source = self.string().to_string();
        // With Shift the drag adds to the blocks already selected.
        let base = match flags.contains(NSEventModifierFlags::Shift) {
            true => {
                let (print, ranges) = &*self.ivars().selected_blocks.borrow();
                if *print == fingerprint(&source) { ranges.clone() } else { Vec::new() }
            }
            false => Vec::new(),
        };
        let (note, boxes) = self.measured_blocks();
        *self.ivars().marquee.borrow_mut() = Some(BlockMarquee {
            down: Retained::retain(event as *const NSEvent as *mut NSEvent).expect("live event"),
            start, source, note, boxes, dragged: false, base,
        });
        true
    }

    unsafe fn continue_block_marquee(&self, event: &NSEvent) -> bool {
        let mut active = self.ivars().marquee.borrow_mut();
        let Some(marquee) = active.as_mut() else {
            return false;
        };
        let point = self.convertPoint_fromView(event.locationInWindow(), None);
        marquee.dragged |= (point.x - marquee.start.x).hypot(point.y - marquee.start.y) >= 4.0;
        if !marquee.dragged {
            return true;
        }
        let (top, bottom) = (marquee.start.y.min(point.y), marquee.start.y.max(point.y));
        self.ivars().block_marquee.set(Some(NSRect::new(
            NSPoint::new(marquee.start.x.min(point.x), top),
            NSSize::new((marquee.start.x - point.x).abs(), bottom - top),
        )));
        let hit = |index: &usize| marquee.boxes[*index].is_some_and(|b| b.rect.origin.y < bottom && top < b.rect.origin.y + b.rect.size.height);
        let kept = |index: &usize| marquee.base.contains(&marquee.note[*index].bytes);
        let selected: Vec<usize> = (0..marquee.note.len()).filter(|i| hit(i) || kept(i)).collect();
        *self.ivars().selected_blocks.borrow_mut() = (fingerprint(&marquee.source), selected.iter().map(|i| marquee.note[*i].bytes.clone()).collect());
        *self.ivars().block_marks.borrow_mut() = selected.iter().filter_map(|i| marquee.boxes[*i].map(|b| b.rect)).collect();
        drop(active);
        self.autoscroll(event);
        self.setNeedsDisplay(true);
        true
    }

    /// Returns the press event when the pointer never moved: an ordinary click.
    unsafe fn finish_block_marquee(&self) -> Option<Option<Retained<NSEvent>>> {
        let marquee = self.ivars().marquee.borrow_mut().take()?;
        self.ivars().block_marquee.set(None);
        self.setNeedsDisplay(true);
        if !marquee.dragged {
            // A Shift click that never moved is an ordinary click: the kept
            // selection goes, as it does for any other click.
            self.clear_block_selection();
            return Some(Some(marquee.down));
        }
        if let Some(window) = self.window() {
            window.makeFirstResponder(Some(self));
        }
        Some(None)
    }
}

/// Choose the drop for a pointer position. Attachments take a block beside
/// them at their left and right edges; everywhere else blocks go between rows.
fn block_drop(
    note: &[note_blocks::Block],
    rows: &[note_blocks::Row],
    boxes: &[Option<BlockBox>],
    moving: &[usize],
    point: NSPoint,
) -> Option<BlockDrop> {
    for row in rows {
        // Two attachments already fill their row.
        let Some(outline) = boxes[row.media].filter(|_| !moving.contains(&row.media) && !row.side.is_some_and(|side| is_attachment(&note[side]))) else {
            continue;
        };
        let rect = outline.rect;
        // The top and bottom quarters stay ordinary boundaries above and below.
        let band = rect.size.height / 4.0;
        if point.y < rect.origin.y + band || point.y >= rect.origin.y + rect.size.height - band {
            continue;
        }
        let edge = 32.0_f64.min(rect.size.width / 4.0);
        let free = row.side.is_none_or(|side| moving.contains(&side));
        let right = rect.origin.x + rect.size.width;
        if point.x >= right - edge && (free || point.x < right) {
            return Some(BlockDrop::Beside(row.media, Placement::Left));
        }
        // The page margin is where handles are dragged, so it is not a side.
        if point.x < rect.origin.x + edge && (point.x >= rect.origin.x || (free && rect.origin.x > EDITOR_INSET + MIN_COLUMN_WIDTH / 2.0)) {
            return Some(BlockDrop::Beside(row.media, Placement::Right));
        }
    }
    // A side block shares its row, so only the row's first block is a boundary.
    let side = |index: &usize| rows.iter().any(|row| row.side == Some(*index));
    let target = (0..note.len())
        .filter(|index| !side(index))
        .find(|index| boxes[*index].is_some_and(|b| point.y < b.rect.origin.y + b.rect.size.height / 2.0))
        .unwrap_or(note.len());
    let first = *moving.first()?;
    let last = *moving.last()?;
    let contiguous = last + 1 - first == moving.len();
    (!(contiguous && (first..=last + 1).contains(&target)) && !moving.contains(&target)).then_some(BlockDrop::Before(target))
}

/// The note after the drop and the byte range the moved blocks land in.
fn dropped(source: &str, note: &[note_blocks::Block], moving: &[usize], drop: BlockDrop, width: f64) -> Option<(String, Range<usize>)> {
    match drop {
        BlockDrop::Before(target) => note_blocks::reordered(source, note, moving, Some(target)),
        BlockDrop::Beside(media, placement) => {
            // Only the attachment's markup changes length, so indices hold.
            let placed = with_placement(source, &note[media], placement, width)?;
            let blocks = note_structure(&placed);
            if blocks.len() != note.len() {
                return None;
            }
            match note_blocks::reordered(&placed, &blocks, moving, Some(media + 1)) {
                Some(result) => Some(result),
                // Already right after the attachment: only the placement changes.
                None => (placed != source).then(|| {
                    let range = blocks[moving[0]].bytes.start..blocks[*moving.last().unwrap()].bytes.end;
                    (placed, range)
                }),
            }
        }
    }
}

/// Selected blocks that hold text, without measuring the layout.
pub unsafe fn selected_block_count(text: &NSTextView) -> usize {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else { return 0 };
    let (print, ranges) = &*editor.ivars().selected_blocks.borrow();
    if ranges.is_empty() { return 0; }
    let source = text.string().to_string();
    if *print != fingerprint(&source) { return 0; }
    ranges.iter().filter(|r| source.get((*r).clone()).is_some_and(|t| !t.trim().is_empty())).count()
}

/// What attaching the selected blocks now would send.
pub unsafe fn note_excerpt(text: &NSTextView, title: &str) -> Result<note_context::Excerpt, String> {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else { return Err("노트를 열어 주세요".into()) };
    let source = editor.string().to_string();
    let note = note_structure(&source);
    let selected = editor.selected_blocks(&source, &note);
    note_context::excerpt(title, &source, &note, &selected)
}

/// Select the blocks a question was about and scroll them near the top.
pub unsafe fn reveal_blocks(text: &NSTextView, blocks: &[Range<usize>], heads: &[String]) -> bool {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else { return false };
    let source = editor.string().to_string();
    let note = note_structure(&source);
    let found = note_context::locate(&source, &note, blocks, heads);
    let Some(first) = found.first().copied() else { return false };
    editor.scrollRangeToVisible(utf16_range(&source, note[first].bytes.start..note[first].bytes.start));
    *editor.ivars().block_cache.borrow_mut() = None;
    editor.select_blocks(&source, found.iter().map(|i| note[*i].bytes.clone()).collect());
    let (_, boxes) = editor.measured_blocks();
    if let Some(outline) = boxes.get(first).copied().flatten() {
        editor.scrollPoint(NSPoint::new(0.0, (outline.rect.origin.y - 48.0).max(0.0)));
    }
    true
}

impl BlockHandle {
    unsafe fn begin_drag(&self, event: &NSEvent) {
        let Some(text) = self.ivars().text.load() else {
            return;
        };
        let (Some(editor), Some(window)) = (text.downcast_ref::<MarkdownEditor>(), self.window()) else {
            return;
        };
        let marked: bool = msg_send![&*text, hasMarkedText];
        if !text.isEditable() || marked || !window.makeFirstResponder(Some(&text)) {
            return;
        }
        let source = text.string().to_string();
        let (note, rows) = note_rows(&source);
        let Some(own) = note.iter().position(|b| b.bytes.start == self.ivars().block.get()) else {
            return;
        };
        let selected = editor.selected_blocks(&source, &note);
        // With Shift the block joins the blocks already selected, as a Shift
        // marquee drag does; a click then selects them all and a drag moves them all.
        let moving: Vec<usize> = if selected.contains(&own) {
            selected
        } else if event.modifierFlags().contains(NSEventModifierFlags::Shift) {
            let mut joined: Vec<usize> = selected.into_iter().chain(note_blocks::subtree(&note, own)).collect();
            joined.sort_unstable();
            joined.dedup();
            joined
        } else {
            note_blocks::subtree(&note, own).collect()
        };
        let boxes = editor.block_boxes(&source, &note);
        *self.ivars().drag.borrow_mut() = Some(BlockDrag {
            origin: event.locationInWindow(), source, note, rows, boxes, moving, dragged: false, drop: None,
        });
    }

    unsafe fn continue_drag(&self, event: &NSEvent) {
        let Some(text) = self.ivars().text.load() else {
            return;
        };
        let Some(editor) = text.downcast_ref::<MarkdownEditor>() else {
            return;
        };
        let mut active = self.ivars().drag.borrow_mut();
        let Some(drag) = active.as_mut() else {
            return;
        };
        let location = event.locationInWindow();
        drag.dragged |= (location.x - drag.origin.x).hypot(location.y - drag.origin.y) >= 4.0;
        // An unclosed fence would swallow whatever is moved below it.
        let unclosed = code_blocks(&drag.source).iter().any(|code| code.close.is_empty() && drag.moving.iter().any(|i| drag.note[*i].bytes.contains(&code.bytes.start)));
        if !drag.dragged || unclosed {
            return;
        }
        NSCursor::closedHandCursor().set();
        text.autoscroll(event);
        let point = text.convertPoint_fromView(location, None);
        drag.drop = block_drop(&drag.note, &drag.rows, &drag.boxes, &drag.moving, point);
        let boxes = &drag.boxes;
        let line = drag.drop.and_then(|drop| match drop {
            BlockDrop::Before(target) => {
                let y = match boxes.get(target).copied().flatten() {
                    Some(outline) => outline.rect.origin.y - 4.0,
                    None => boxes.iter().flatten().map(|b| b.rect.origin.y + b.rect.size.height).fold(0.0, f64::max) + 4.0,
                };
                Some(NSRect::new(NSPoint::new(EDITOR_INSET, y), NSSize::new(text.bounds().size.width - 2.0 * EDITOR_INSET, 2.0)))
            }
            BlockDrop::Beside(media, placement) => boxes[media].map(|outline| {
                let x = if placement == Placement::Left { outline.rect.origin.x + outline.rect.size.width + COLUMN_GAP / 2.0 } else { outline.rect.origin.x - 5.0 };
                NSRect::new(NSPoint::new(x - 1.0, outline.rect.origin.y), NSSize::new(2.0, outline.rect.size.height))
            }),
        });
        if editor.ivars().block_drop.replace(line) != line {
            text.setNeedsDisplay(true);
        }
    }

    /// A click selects the block; a drag moves it, as one undo step.
    unsafe fn finish_drag(&self) {
        let _keep = Retained::retain(self as *const BlockHandle as *mut BlockHandle);
        let Some(drag) = self.ivars().drag.borrow_mut().take() else {
            return;
        };
        let Some(text) = self.ivars().text.load() else {
            return;
        };
        let Some(editor) = text.downcast_ref::<MarkdownEditor>() else {
            return;
        };
        editor.ivars().block_drop.set(None);
        text.setNeedsDisplay(true);
        if text.string().to_string() != drag.source {
            return;
        }
        if !drag.dragged {
            let ranges = drag.moving.iter().map(|i| drag.note[*i].bytes.clone()).collect();
            editor.select_blocks(&drag.source, ranges);
            return;
        }
        let width = match drag.drop {
            Some(BlockDrop::Beside(media, _)) => drag.boxes[media].map_or(280.0, |b| b.rect.size.width),
            _ => 280.0,
        };
        let Some((result, landed)) = drag.drop.and_then(|drop| dropped(&drag.source, &drag.note, &drag.moving, drop, width)) else {
            return;
        };
        editor.clear_block_selection();
        if commit_source(&text, &result, Some(landed.end.min(result.len()))) {
            let moved = note_structure(&result).into_iter()
                .filter(|b| landed.start <= b.bytes.start && b.bytes.end <= landed.end)
                .map(|b| b.bytes)
                .collect();
            self.setHidden(true);
            editor.select_blocks(&result, moved);
        }
    }
}

#[cfg(test)]
mod block_tests {
    use super::*;

    const IMAGE: &str = "![그림](<a.png> \"stology:width=200;x=0.000;wrap=0\")";

    fn outline(x: f64, y: f64, width: f64, height: f64) -> Option<BlockBox> {
        Some(BlockBox { rect: NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)), line: height.min(24.0) })
    }

    #[test]
    fn note_structure_reads_the_blocks_the_editor_draws() {
        use note_blocks::Kind;
        let source = format!("# 제목\n{IMAGE}\n\n문장 안의 ![그림](a.png) 링크\n- 항목\n\n---\n| 가 |\n| --- |\n| 1 |\n\n```rs\n![코드](a.png)\n```\n");
        let note = note_structure(&source);
        assert_eq!(
            note.iter().map(|b| b.kind).collect::<Vec<_>>(),
            [Kind::Text, Kind::Image, Kind::Text, Kind::Item, Kind::Blank, Kind::Rule, Kind::Table, Kind::Code],
            "an image inside a sentence or a code block is not an attachment block"
        );
        assert_eq!(note.iter().map(|b| b.bytes.len()).sum::<usize>(), source.len());
        assert!(source[note[1].bytes.clone()].ends_with(")\n\n") && source[note[6].bytes.clone()].ends_with("| 1 |\n\n"));
    }

    #[test]
    fn placement_is_stored_in_the_existing_image_and_table_metadata() {
        let source = format!("{IMAGE}\n\n| 가 | 나 |\n| --- | --- |\n| 1 | 2 |\n");
        let (note, rows) = note_rows(&source);
        assert!(rows.iter().all(|row| row.placement == Placement::Full && row.side.is_none()));
        let right = with_placement(&source, &note[0], Placement::Right, 200.0).unwrap();
        assert!(right.starts_with("![그림](<a.png> \"stology:width=200;x=1.000;wrap=1\")\n\n| 가 |"), "{right}");
        let (note, rows) = note_rows(&right);
        // The right-hand image takes the table as its side block.
        assert_eq!(rows, [note_blocks::Row { media: 0, side: Some(1), placement: Placement::Right }]);
        let table = with_placement(&right, &note[1], Placement::Right, 0.0).unwrap();
        assert!(table.contains("| <!--stology-grid {\"side\":\"right\",\"wrap\":true}-->가 | 나 |"), "{table}");
        let note = note_structure(&table);
        let left = with_placement(&table, &note[1], Placement::Left, 0.0).unwrap();
        assert!(left.contains("{\"wrap\":true}") && !left.contains("side"));
        let full = with_placement(&left, &note_structure(&left)[1], Placement::Full, 0.0).unwrap();
        assert_eq!(full, right, "the grid comment disappears with its last setting");
        // Older notes stored a free position: it only picks a side now.
        let legacy = image_edit::markdown::images("![a](a.png \"stology:width=90;x=0.640;wrap=1;y=30\")").remove(0);
        assert_eq!(image_edit::placement(&legacy), Placement::Right);
        assert_eq!(image_edit::placed(&legacy, 90.0, Placement::Right), "![a](<a.png> \"stology:width=90;x=1.000;wrap=1\")");
    }

    #[test]
    fn drops_land_between_rows_or_beside_an_attachment() {
        let source = format!("가\n{IMAGE}\n\n나\n다\n");
        let (note, rows) = note_rows(&source);
        // 가 / whole-row image / 나 / 다, stacked from y = 0.
        let boxes = [outline(25., 0., 700., 24.), outline(25., 40., 200., 120.), outline(25., 180., 700., 24.), outline(25., 220., 700., 24.)];
        let at = |x, y| block_drop(&note, &rows, &boxes, &[3], NSPoint::new(x, y));
        assert!(at(60., 5.) == Some(BlockDrop::Before(0)));
        assert!(at(60., 185.) == Some(BlockDrop::Before(2)));
        assert!(at(60., 235.).is_none() && at(60., 400.).is_none(), "dropping a block where it is does nothing");
        // The image's right edge and the empty space beyond it take a side block.
        assert!(at(215., 100.) == Some(BlockDrop::Beside(1, Placement::Left)));
        assert!(at(600., 100.) == Some(BlockDrop::Beside(1, Placement::Left)));
        assert!(at(30., 100.) == Some(BlockDrop::Beside(1, Placement::Right)));
        // The page margin and the image's top and bottom quarters stay boundaries.
        assert!(at(10., 100.) == Some(BlockDrop::Before(2)));
        assert!(at(215., 45.) == Some(BlockDrop::Before(1)));
        let (result, landed) = dropped(&source, &note, &[3], BlockDrop::Beside(1, Placement::Left), 200.0).unwrap();
        assert_eq!(result, format!("가\n{}\n\n다\n나\n", IMAGE.replace("wrap=0", "wrap=1")));
        assert_eq!(&result[landed], "다\n");
        // A block already after the attachment only needs the placement.
        let (result, landed) = dropped(&source, &note, &[2], BlockDrop::Beside(1, Placement::Right), 200.0).unwrap();
        assert_eq!(result, format!("가\n{}\n\n나\n다\n", IMAGE.replace("x=0.000;wrap=0", "x=1.000;wrap=1")));
        assert_eq!(&result[landed], "나\n");
        // A filled row keeps its side block unless the drop is on the image's own edge.
        let (note, rows) = note_rows(&result);
        let boxes = [outline(25., 0., 700., 24.), outline(400., 40., 200., 120.), outline(25., 40., 350., 24.), outline(25., 180., 700., 24.)];
        let at = |x, y| block_drop(&note, &rows, &boxes, &[3], NSPoint::new(x, y));
        assert!(at(700., 100.) == Some(BlockDrop::Before(3)) || at(700., 100.).is_none());
        assert!(at(590., 100.) == Some(BlockDrop::Beside(1, Placement::Left)));
    }
}
