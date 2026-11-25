//! Clipboard attachments and native image controls; all document edits use NSTextView undo.
use super::*;
use objc2_foundation::{NSData, NSInsetRect};
use std::cell::Cell;
#[path = "../../shared/image_markdown.rs"]
#[allow(dead_code)]
pub mod markdown;

type DiagramOpener = fn(&str);
pub fn set_diagram_opener(open: DiagramOpener) { DIAGRAM_OPEN.with(|v|v.set(Some(open))); }
type Importer = fn(&[u8], &str) -> Result<(String, PathBuf), String>;
thread_local! {
    static DIAGRAM_OPEN: Cell<Option<DiagramOpener>> = const {Cell::new(None)};
    static IMPORTER: Cell<Option<Importer>> = const { Cell::new(None) };
    static IMAGE_CACHE: RefCell<Vec<(PathBuf, u64, Option<std::time::SystemTime>, Retained<NSImage>)>> = const { RefCell::new(Vec::new()) };
    static INTERACTING: Cell<bool> = const { Cell::new(false) };
}
pub fn set_image_importer(importer: Importer) {
    IMPORTER.with(|v| v.set(Some(importer)));
}
pub fn interacting() -> bool {
    INTERACTING.with(Cell::get)
}

/// Cmd+A selects disjoint text runs, leaving image syntax and its block separators intact.
pub fn text_selection_ranges(source: &str) -> Option<Vec<NSRange>> {
    let images = markdown::images(source);
    if images.is_empty() {
        return None;
    }
    let mut protected: Vec<Range<usize>> = images.iter().map(|image| image.bytes.clone()).collect();
    let parser = Parser::new_ext(source, Options::all());
    for (_, definition) in parser.reference_definitions().iter() {
        if images
            .iter()
            .any(|image| image.destination == definition.dest.as_ref())
        {
            protected.push(definition.span.clone());
        }
    }
    for range in &mut protected {
        let line_start = source[..range.start].rfind('\n').map_or(0, |n| n + 1);
        let line_end = source[range.end..]
            .find('\n')
            .map_or(source.len(), |n| range.end + n);
        if source[line_start..range.start].trim().is_empty()
            && source[range.end..line_end].trim().is_empty()
        {
            // Preserve standalone paragraphs so deleting surrounding text cannot merge image anchors.
            range.start = line_start.saturating_sub(usize::from(line_start > 0));
            range.end = line_end;
            while range.end < source.len() && matches!(source.as_bytes()[range.end], b'\r' | b'\n')
            {
                range.end += 1;
            }
        }
    }
    protected.sort_by_key(|r| r.start);
    let mut ranges = Vec::new();
    let mut end = 0;
    for image in protected {
        if image.start > end {
            ranges.push(utf16_range(source, end..image.start));
        }
        end = end.max(image.end);
    }
    if end < source.len() {
        ranges.push(utf16_range(source, end..source.len()));
    }
    Some(ranges)
}
pub unsafe fn select_text(text: &NSTextView) -> bool {
    let source = text.string().to_string();
    let Some(ranges) = text_selection_ranges(&source) else {
        return false;
    };
    if ranges.is_empty() {
        text.setSelectedRange(NSRange::new(0, 0));
    } else {
        let values: Vec<_> = ranges
            .into_iter()
            .map(|range| objc2_foundation::NSValue::valueWithRange(range))
            .collect();
        text.setSelectedRanges(&objc2_foundation::NSArray::from_retained_slice(&values));
    }
    true
}

/// AppKit deletes disjoint ranges correctly, but typing normally replaces only its first range.
/// Replace a Cmd+A text-only selection as one native undoable edit instead.
pub unsafe fn replacing_selected_text(
    text: &NSTextView,
    replacement: &str,
    requested: NSRange,
) -> Option<(String, usize)> {
    let source = text.string().to_string();
    let selected = text.selectedRanges();
    if selected.len() < 2 {
        return None;
    }
    let first = selected.objectAtIndex(0).rangeValue();
    if requested.location <= source.encode_utf16().count() && requested != first {
        return None;
    }
    let expected = text_selection_ranges(&source)?;
    if expected.len() != selected.len()
        || expected
            .iter()
            .zip(selected.iter())
            .any(|(range, value)| *range != value.rangeValue())
    {
        return None;
    }
    let mut result = source.clone();
    for (index, range) in expected.iter().enumerate().rev() {
        result.replace_range(
            byte_for_utf16(&source, range.location)
                ..byte_for_utf16(&source, range.location + range.length),
            if index == 0 { replacement } else { "" },
        );
    }
    Some((result, first.location + replacement.encode_utf16().count()))
}

const RESIZE_CORNER_SIZE: f64 = 48.0;
const RESIZE_EDGE_SIZE: f64 = 32.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Bottom,
    Top,
    Left,
    Right,
}
impl ResizeHandle {
    const ALL: [Self; 8] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::Bottom,
        Self::Top,
        Self::Left,
        Self::Right,
    ];
    fn left(self) -> bool {
        matches!(self, Self::TopLeft | Self::BottomLeft | Self::Left)
    }
    fn top(self) -> bool {
        matches!(self, Self::TopLeft | Self::TopRight | Self::Top)
    }
    fn position(self) -> NSCursorFrameResizePosition {
        match self {
            Self::TopLeft => NSCursorFrameResizePosition::TopLeft,
            Self::TopRight => NSCursorFrameResizePosition::TopRight,
            Self::BottomLeft => NSCursorFrameResizePosition::BottomLeft,
            Self::BottomRight => NSCursorFrameResizePosition::BottomRight,
            Self::Bottom => NSCursorFrameResizePosition::Bottom,
            Self::Top => NSCursorFrameResizePosition::Top,
            Self::Left => NSCursorFrameResizePosition::Left,
            Self::Right => NSCursorFrameResizePosition::Right,
        }
    }
    fn rect(self, size: NSSize) -> NSRect {
        // Leave a move target in the middle even for very small images.
        let w = RESIZE_CORNER_SIZE.min(size.width / 3.0);
        let h = RESIZE_CORNER_SIZE.min(size.height / 3.0);
        if matches!(self, Self::Top | Self::Bottom) {
            let edge = RESIZE_EDGE_SIZE.min(size.height / 3.0);
            NSRect::new(
                NSPoint::new(w, if self.top() { 0.0 } else { size.height - edge }),
                NSSize::new(size.width - 2.0 * w, edge),
            )
        } else if matches!(self, Self::Left | Self::Right) {
            let edge = RESIZE_EDGE_SIZE.min(size.width / 3.0);
            NSRect::new(
                NSPoint::new(if self.left() { 0.0 } else { size.width - edge }, h),
                NSSize::new(edge, size.height - 2.0 * h),
            )
        } else {
            NSRect::new(
                NSPoint::new(
                    if self.left() { 0.0 } else { size.width - w },
                    if self.top() { 0.0 } else { size.height - h },
                ),
                NSSize::new(w, h),
            )
        }
    }
}
fn resize_handle(point: NSPoint, size: NSSize) -> Option<ResizeHandle> {
    ResizeHandle::ALL.into_iter().find(|handle| {
        let r = handle.rect(size);
        point.x >= r.origin.x
            && point.x <= r.origin.x + r.size.width
            && point.y >= r.origin.y
            && point.y <= r.origin.y + r.size.height
    })
}
fn resized_frame(
    original: NSRect,
    handle: ResizeHandle,
    dx: f64,
    dy: f64,
    max_width: f64,
) -> NSRect {
    let ratio = original.size.height / original.size.width;
    let change = if matches!(handle, ResizeHandle::Top | ResizeHandle::Bottom) {
        (if handle.top() { -dy } else { dy }) / ratio
    } else if matches!(handle, ResizeHandle::Left | ResizeHandle::Right) {
        if handle.left() { -dx } else { dx }
    } else {
        // Project pointer motion onto the image's aspect ratio: vertical drags work too.
        ((if handle.left() { -dx } else { dx }) + (if handle.top() { -dy } else { dy }) * ratio)
            / (1.0 + ratio * ratio)
    };
    let limit = if handle.left() {
        original.origin.x + original.size.width - EDITOR_INSET
    } else {
        max_width - (original.origin.x - EDITOR_INSET)
    };
    let width = (original.size.width + change).clamp(32.0, limit.max(32.0));
    NSRect::new(
        NSPoint::new(
            if handle.left() {
                original.origin.x + original.size.width - width
            } else {
                original.origin.x
            },
            original.origin.y,
        ),
        NSSize::new(width, width * ratio),
    )
}

#[allow(deprecated)]
unsafe fn resize_cursor(handle: ResizeHandle) -> Retained<NSCursor> {
    let available: bool = msg_send![NSCursor::class(),respondsToSelector:sel!(frameResizeCursorFromPosition:inDirections:)];
    if available {
        NSCursor::frameResizeCursorFromPosition_inDirections(
            handle.position(),
            NSCursorFrameResizeDirections::All,
        )
    } else {
        // Public AppKit fallback for macOS 14, which predates frame resize cursors.
        if matches!(handle, ResizeHandle::Top | ResizeHandle::Bottom) {
            return NSCursor::resizeUpDownCursor();
        }
        if matches!(handle, ResizeHandle::Left | ResizeHandle::Right) {
            return NSCursor::resizeLeftRightCursor();
        }
        let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(24.0, 24.0));
        image.lockFocus();
        let path = NSBezierPath::bezierPath();
        for (a, b) in [
            ((5., 19.), (19., 5.)),
            ((5., 12.), (5., 19.)),
            ((5., 19.), (12., 19.)),
            ((12., 5.), (19., 5.)),
            ((19., 5.), (19., 12.)),
        ] {
            let flip = matches!(handle, ResizeHandle::TopRight | ResizeHandle::BottomLeft);
            path.moveToPoint(NSPoint::new(a.0, if flip { 24.0 - a.1 } else { a.1 }));
            path.lineToPoint(NSPoint::new(b.0, if flip { 24.0 - b.1 } else { b.1 }));
        }
        NSColor::whiteColor().setStroke();
        path.setLineWidth(4.0);
        path.stroke();
        NSColor::blackColor().setStroke();
        path.setLineWidth(2.0);
        path.stroke();
        image.unlockFocus();
        NSCursor::initWithImage_hotSpot(NSCursor::alloc(), &image, NSPoint::new(12.0, 12.0))
    }
}

pub unsafe fn load_image(path: &Path, metadata: &std::fs::Metadata) -> Option<Retained<NSImage>> {
    IMAGE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let modified = metadata.modified().ok();
        if let Some((_, _, _, image)) = cache
            .iter()
            .find(|(p, size, time, _)| p == path && *size == metadata.len() && *time == modified)
        {
            return Some(image.clone());
        }
        cache.retain(|(p, _, _, _)| p != path);
        let owned = path.parent().and_then(Path::file_name).is_some_and(|name|name==".diagram")
            && path.file_name().and_then(|n|n.to_str()).is_some_and(markdown::diagram_filename);
        let image = if owned && metadata.len() <= stology_core::diagram_svg::LIMIT as u64 {
            std::fs::read_to_string(path).ok()
                .and_then(|svg| stology_core::diagram_svg::Diagram::read(&svg).ok())
                .map(|(diagram, title)| {
                    let image = diagram_paint::image(diagram);
                    image.setAccessibilityDescription(Some(&NSString::from_str(&title)));
                    image
                })
        } else {None}.or_else(|| NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path.to_str()?)))?;
        if cache.len() >= 32 {
            cache.remove(0);
        }
        cache.push((path.to_owned(), metadata.len(), modified, image.clone()));
        Some(image)
    })
}

pub unsafe fn has_clipboard_image() -> bool {
    let board = NSPasteboard::generalPasteboard();
    let types = objc2_foundation::NSArray::from_slice(&[
        NSPasteboardTypePNG,
        NSPasteboardTypeTIFF,
        &NSString::from_str("public.file-url"),
    ]);
    board.availableTypeFromArray(&types).is_some()
}

pub unsafe fn paste(text: &NSTextView) -> bool {
    if !text.isEditable() {
        return false;
    }
    let marked: bool = msg_send![text, hasMarkedText];
    if marked {
        return false;
    }
    let board = NSPasteboard::generalPasteboard();
    // Prefer the actual copied file over Finder's icon representation.
    let file_url_type = NSString::from_str("public.file-url");
    let mut encoded: Option<(Vec<u8>, &str)> = None;
    if let Some(url) = board
        .stringForType(&file_url_type)
        .and_then(|s| NSURL::URLWithString(&s))
    {
        if url.isFileURL() {
            if let Some(path) = url.path() {
                let path = PathBuf::from(path.to_string());
                if std::fs::metadata(&path)
                    .is_ok_and(|m| m.is_file() && m.len() <= 20 * 1024 * 1024)
                {
                    if let Ok(bytes) = std::fs::read(&path) {
                        if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                            encoded = Some((bytes, "png"));
                        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
                            encoded = Some((bytes, "jpg"));
                        }
                    }
                }
            }
        }
        if encoded.is_none() {
            return false;
        }
    }
    if encoded.is_none() {
        if let Some(data) = board.dataForType(NSPasteboardTypePNG) {
            encoded = Some((data.to_vec(), "png"));
        } else if let Some(data) = board.dataForType(NSPasteboardTypeTIFF) {
            if data.length() > 80 * 1024 * 1024 {
                show_error("이미지가 너무 큽니다");
                return true;
            }
            if let Some(bitmap) = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data) {
                if bitmap.pixelsWide().saturating_mul(bitmap.pixelsHigh()) > 40_000_000 {
                    show_error("이미지는 4천만 픽셀 이하로 붙여넣어 주세요");
                    return true;
                }
                if let Some(png) = bitmap.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                ) {
                    encoded = Some((png.to_vec(), "png"));
                }
            }
        }
    }
    let Some((bytes, extension)) = encoded else {
        return false;
    };
    if bytes.len() > 20 * 1024 * 1024 {
        show_error("이미지는 20 MiB 이하로 붙여넣어 주세요");
        return true;
    }
    let data = NSData::with_bytes(&bytes);
    let Some(bitmap) = NSBitmapImageRep::initWithData(NSBitmapImageRep::alloc(), &data) else {
        show_error("이미지를 읽을 수 없습니다");
        return true;
    };
    if bitmap.pixelsWide() <= 0
        || bitmap.pixelsHigh() <= 0
        || bitmap.pixelsWide().saturating_mul(bitmap.pixelsHigh()) > 40_000_000
    {
        show_error("이미지 크기가 지원 범위를 벗어났습니다");
        return true;
    }
    let result = IMPORTER.with(|v| {
        v.get()
            .ok_or_else(|| "이미지 저장소가 연결되지 않았습니다".to_owned())
            .and_then(|f| f(&bytes, extension))
    });
    match result {
        Ok((destination, base)) => {
            let range: NSRange = msg_send![text, selectedRange];
            insert_attachment(text,range,"이미지",&destination,&base,280.);
        }
        Err(message) => show_error(&message),
    }
    true
}
pub(super) fn is_diagram_link(destination: &str) -> bool {
    destination.trim_start_matches("../").strip_prefix(".diagram/").is_some_and(markdown::diagram_filename)
}

/// New attachments take a whole row; `wrap=1` opens the column beside them.
pub(super) fn attachment_markup(alt: &str, destination: &str, width: f64) -> String {
    format!("![{alt}](<{destination}> \"stology:width={width:.0};x=0.000;wrap=0\")\n\n")
}

/// Older notes stored a free horizontal position. It now only picks a side.
pub(super) fn placement(link: &markdown::ImageLink) -> Placement {
    if !link.wrap {
        Placement::Full
    } else if link.x < 0.5 {
        Placement::Left
    } else {
        Placement::Right
    }
}

pub(super) fn placed(link: &markdown::ImageLink, width: f64, placement: Placement) -> String {
    let x = if placement == Placement::Right { 1.0 } else { 0.0 };
    link.markdown_at(&link.destination, width, x, placement != Placement::Full, 0.0)
}

pub(super) unsafe fn insert_attachment(text:&NSTextView, range:NSRange, alt:&str, destination:&str, base:&Path, width:f64) {
    if let Some(editor)=text.downcast_ref::<MarkdownEditor>() {
        *editor.ivars().base_directory.borrow_mut()=Some(base.to_owned());
    }
    let source=text.string().to_string();
    let start=byte_for_utf16(&source,range.location);
    let prefix=if start>0 && !source[..start].ends_with('\n'){"\n\n"}else{""};
    let markup = format!("{prefix}{}", attachment_markup(alt, destination, width));
    text.breakUndoCoalescing();
    let _:()=msg_send![text,insertText:&*NSString::from_str(&markup),replacementRange:range];
    text.breakUndoCoalescing();
    style_with_base(text,Some(base));
}

unsafe fn show_error(message: &str) {
    let alert = NSAlert::new(MainThreadMarker::new().unwrap());
    alert.setMessageText(&NSString::from_str("이미지를 붙여넣지 못했습니다"));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

// A layout pass recreates InlineImage views. Anchor selection to this occurrence
// in the editor so a refresh or a second link to the same SVG cannot move it.
pub(super) fn select_image(text: &NSTextView, anchor: Option<usize>) {
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else { return };
    if editor.ivars().selected_image.replace(anchor) != anchor {
        for child in editor.subviews() {
            if let Some(image) = child.downcast_ref::<InlineImage>() {
                image.sync_selection();
            }
        }
    }
}

pub(super) unsafe fn delete_selected_image(text: &MarkdownEditor) -> bool {
    let Some(anchor) = text.ivars().selected_image.get() else { return false };
    let marked: bool = msg_send![text, hasMarkedText];
    if !text.isEditable() || marked { return false; }
    for child in text.subviews() {
        if let Some(image) = child.downcast_ref::<InlineImage>() {
            if image.ivars().link.bytes.start == anchor {
                image.replace("");
                return true;
            }
        }
    }
    false
}

// The editable view grows temporarily. Markdown and TextKit always use the
// content frame, so selecting an attachment never changes its line wrapping.
const SELECTION_MARGIN:f64=11.;
fn draw_resize_handles(size: NSSize) {
    let side = 7.0_f64.min(size.width / 4.).min(size.height / 4.);
    if side < 1. { return; }
    let inset = side / 2. + 0.5;
    let left = inset;
    let right = size.width - inset;
    let top = inset;
    let bottom = size.height - inset;
    let center_x = size.width / 2.;
    let center_y = size.height / 2.;
    let accent = diagram_paint::color("#6556D9");
    accent.setStroke();
    let outline = NSBezierPath::bezierPathWithRect(NSRect::new(
        NSPoint::new(left, top), NSSize::new(right-left, bottom-top)));
    outline.setLineWidth(1.25);
    outline.stroke();
    NSColor::whiteColor().setFill();
    for (x,y) in [(left,top),(center_x,top),(right,top),(right,center_y),
                  (right,bottom),(center_x,bottom),(left,bottom),(left,center_y)] {
        let handle=NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
            NSRect::new(NSPoint::new(x-side/2., y-side/2.),NSSize::new(side,side)),1.5,1.5);
        handle.fill();
        handle.setLineWidth(1.25);
        handle.stroke();
    }
}

/// A place an attachment can be dropped: in front of block `index`.
#[derive(Clone, Copy)]
pub(super) struct DropAnchor {
    index: usize,
    top: f64,
}

// Attachments move between whole blocks. An image written inside a sentence
// stays where its Markdown puts it.
unsafe fn drop_anchors(text: &NSTextView, link: &markdown::ImageLink) -> (usize, Vec<DropAnchor>) {
    let source = text.string().to_string();
    if !source.is_char_boundary(link.bytes.end) || !standalone(&source, &link.bytes) { return (usize::MAX, Vec::new()); }
    let note = note_structure(&source);
    let own = note_blocks::block_at(&note, link.bytes.start).unwrap_or(usize::MAX);
    let Some(editor) = text.downcast_ref::<MarkdownEditor>() else { return (usize::MAX, Vec::new()) };
    let boxes = editor.block_boxes(&source, &note);
    let mut anchors: Vec<_> = boxes.iter().enumerate()
        .filter_map(|(index, outline)| outline.map(|b| DropAnchor { index, top: b.rect.origin.y }))
        .collect();
    let end = boxes.iter().flatten().map(|b| b.rect.origin.y + b.rect.size.height).fold(0.0, f64::max);
    anchors.push(DropAnchor { index: note.len(), top: end });
    (own, anchors)
}

pub(super) struct ImageState {
    pub selected: Cell<bool>,
    pub text: Weak<NSTextView>,
    pub link: markdown::ImageLink,
    pub fingerprint: u64,
    pub base: Option<PathBuf>,
    pub range: NSRange,
    /// The widest this image may be drawn in its column.
    pub limit: f64,
    /// This image's block and the boundaries it can move to, set when a drag starts.
    pub(super) targets: RefCell<(usize, Vec<DropAnchor>)>,
    pub(super) preview: RefCell<Option<Retained<NSTextView>>>,
    pub drag: Cell<Option<(NSPoint, NSRect, Option<ResizeHandle>)>>,
}
define_class!(
    #[unsafe(super(NSImageView, NSControl, NSView, NSResponder))]
    #[name = "StologyInlineMarkdownImage"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ImageState]
    pub(super) struct InlineImage;
    impl InlineImage {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool {true}
        #[unsafe(method(drawRect:))]
        fn draw(&self, dirty: NSRect) { unsafe {
            if self.ivars().selected.get() {
                if let Some(image) = self.image() {
                    image.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                        NSInsetRect(self.bounds(),SELECTION_MARGIN,SELECTION_MARGIN), NSRect::ZERO,
                        NSCompositingOperation::SourceOver,1.,true,None);
                }
                draw_resize_handles(self.bounds().size);
            } else {
                let _: () = msg_send![super(self),drawRect:dirty];
            }
        }}
        #[unsafe(method(viewDidMoveToWindow))]
        fn moved_to_window(&self) { unsafe {
            let _: () = msg_send![super(self), viewDidMoveToWindow];
            if self.window().is_none() && self.ivars().drag.take().is_some() {
                self.clear_preview();
                self.show_anchor(None);
                INTERACTING.with(|v| v.set(false));
            }
        }}
        #[unsafe(method(resetCursorRects))]
        fn reset_cursors(&self) {unsafe {
            if self.ivars().text.load().is_some_and(|t|t.isEditable()) {
                let bounds=self.bounds();
                self.addCursorRect_cursor(bounds,&NSCursor::openHandCursor());
                for handle in ResizeHandle::ALL {
                    self.addCursorRect_cursor(handle.rect(bounds.size),&resize_cursor(handle));
                }
            }
        }}
        #[unsafe(method(mouseDown:))]
        fn down(&self,event:&NSEvent) {unsafe {
            let Some(text)=self.ivars().text.load() else {return;};
            if !text.isEditable() {return;}
            if event.clickCount()==2 && self.open_diagram(){return;}
            select_image(&text, Some(self.ivars().link.bytes.start));
            let p=self.convertPoint_fromView(event.locationInWindow(),None);
            let resize=resize_handle(p,self.bounds().size);
            if resize.is_none() { *self.ivars().targets.borrow_mut() = drop_anchors(&text, &self.ivars().link); }
            self.ivars().drag.set(Some((text.convertPoint_fromView(event.locationInWindow(),None),self.content_frame(),resize)));
            INTERACTING.with(|v|v.set(true));
            if let Some(handle)=resize {resize_cursor(handle).set();} else {NSCursor::closedHandCursor().set();}
        }}
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self,event:&NSEvent) {unsafe {
            let Some((start,original,resize))=self.ivars().drag.get() else {return;};
            let Some(text)=self.ivars().text.load() else {return;};
            if resize.is_none() { text.autoscroll(event); }
            let delta=text.convertPoint_fromView(event.locationInWindow(),None);
            let max_width=(text.bounds().size.width-2.0*EDITOR_INSET-8.0).max(80.0);
            let mut frame=original;
            if let Some(handle)=resize {
                // The Markdown paragraph stays vertically anchored while its size changes.
                frame=resized_frame(original,handle,delta.x-start.x,delta.y-start.y,max_width);
                let limit=self.ivars().limit.max(32.0);
                if frame.size.width>limit {
                    let height=frame.size.height*limit/frame.size.width;
                    if handle.left() { frame.origin.x+=frame.size.width-limit; }
                    frame.size=NSSize::new(limit,height);
                }
            } else {
                if (delta.x-start.x).hypot(delta.y-start.y) < 3. { self.clear_preview(); self.set_content_frame(original); self.show_anchor(None); return; }
                frame.origin.y=(original.origin.y+delta.y-start.y).clamp(text.textContainerOrigin().y,text.bounds().size.height.max(text.textContainerOrigin().y));
                self.show_anchor(self.target(original,frame));
                frame.origin.x=(original.origin.x+delta.x-start.x).clamp(EDITOR_INSET,EDITOR_INSET+(max_width-frame.size.width).max(0.0));
            }
            let changed = self.content_frame() != frame;
            self.set_content_frame(frame);
            if resize.is_none() && changed { self.preview_move(&text, original, frame); }
            if let Some(handle)=resize {resize_cursor(handle).set();} else {NSCursor::closedHandCursor().set();}
            let _: ()=msg_send![self,setNeedsDisplay:true];
        }}
        #[unsafe(method(mouseUp:))]
        fn up(&self,_event:&NSEvent) {unsafe {
            let _keep=Retained::retain(self as *const Self as *mut Self);
            let drag=self.ivars().drag.replace(None);
            INTERACTING.with(|v|v.set(false));
            let Some((_,original,resize))=drag else {return;};
            let frame=self.content_frame();
            self.clear_preview();
            self.show_anchor(None);
            if frame!=original {
                if !self.commit_layout(original, frame, resize.is_none()) { self.set_content_frame(original); }
            }
            else if let Some(text)=self.ivars().text.load() {
                // Put the caret in the block after the image, not its concealed syntax.
                let source=text.string().to_string();
                let note=note_structure(&source);
                let after=note_blocks::block_at(&note,self.ivars().link.bytes.start).map_or(source.len(),|i|note[i].bytes.end);
                text.setSelectedRange(NSRange::new(source[..after].encode_utf16().count(),0));
                if let Some(window)=text.window(){window.makeFirstResponder(Some(&text));}
            }
        }}
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self,event:&NSEvent) {unsafe {
            let _keep=Retained::retain(self as *const Self as *mut Self);
            INTERACTING.with(|v|v.set(true));
            if let Some(menu)=self.attachment_menu() {
                let point=self.convertPoint_fromView(event.locationInWindow(),None);
                menu.popUpMenuPositioningItem_atLocation_inView(None,point,Some(self));
            }
            INTERACTING.with(|v|v.set(false));
        }}
        #[unsafe(method(menuForEvent:))]
        fn menu(&self,_event:&NSEvent)->*mut NSMenu {unsafe {
            self.attachment_menu().map_or(std::ptr::null_mut(),Retained::autorelease_return)
        }}
        #[unsafe(method(editDiagram:))]
        fn edit_diagram(&self,_sender:Option<&AnyObject>) {self.open_diagram();}
        #[unsafe(method(deleteImage:))]
        fn delete(&self,_sender:Option<&AnyObject>) {unsafe {self.replace("");}}
        #[unsafe(method(placeImage:))]
        fn place(&self,item:&NSMenuItem) {unsafe {
            let placement=[Placement::Full,Placement::Left,Placement::Right][item.tag().clamp(0,2) as usize];
            let link=&self.ivars().link;
            self.replace(&placed(link,link.width.unwrap_or(self.content_frame().size.width),placement));
        }}

    }
);
impl InlineImage {
    fn attachment_menu(&self)->Option<Retained<NSMenu>> {
        if !self.ivars().text.load().is_some_and(|t|t.isEditable()){return None}
        let mtm=MainThreadMarker::new().unwrap();
        let menu=NSMenu::new(mtm);
        menu.setAllowsContextMenuPlugIns(false);
        let add=|title:&str,action| unsafe {
            let item=NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm),&NSString::from_str(title),Some(action),&NSString::new());
            item.setTarget(Some(self));menu.addItem(&item);
        };
        let current=placement(&self.ivars().link);
        for (tag,(title,value)) in [("한 줄 전체",Placement::Full),("왼쪽에 배치",Placement::Left),("오른쪽에 배치",Placement::Right)].into_iter().enumerate() {
            add(title,sel!(placeImage:));
            if let Some(item)=menu.itemAtIndex(tag as isize) {
                item.setTag(tag as isize);
                item.setState(if current==value {1} else {0});
            }
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        if self.diagram_link().is_some(){add("편집",sel!(editDiagram:));}
        add("삭제",sel!(deleteImage:));Some(menu)
    }
    pub(super) fn content_frame(&self)->NSRect{
        if self.ivars().selected.get(){NSInsetRect(self.frame(),SELECTION_MARGIN,SELECTION_MARGIN)}else{self.frame()}
    }
    fn set_content_frame(&self,frame:NSRect){
        self.setFrame(if self.ivars().selected.get(){NSInsetRect(frame,-SELECTION_MARGIN,-SELECTION_MARGIN)}else{frame});
    }
    pub(super) fn sync_selection(&self) {
        let selected=self.ivars().text.load().is_some_and(|text|text.isEditable() &&
            text.downcast_ref::<MarkdownEditor>().is_some_and(|editor|editor.ivars().selected_image.get()==Some(self.ivars().link.bytes.start)));
        if selected!=self.ivars().selected.get() {
            let content=self.content_frame();
            self.ivars().selected.set(selected);
            self.set_content_frame(content);
            NSView::setNeedsDisplay(self,true);
            if let Some(w)=self.window(){w.invalidateCursorRectsForView(self);}
        }
    }
    pub(super) fn diagram_link(&self)->Option<&str>{
        let dest=&self.ivars().link.destination;
        is_diagram_link(dest).then_some(dest.as_str())
    }
    fn open_diagram(&self)->bool{
        let Some(id)=self.diagram_link() else{return false};
        if let Some(open)=DIAGRAM_OPEN.with(Cell::get){open(id);return true}
        false
    }
    fn target(&self, original: NSRect, frame: NSRect) -> Option<DropAnchor> {
        if (frame.origin.y - original.origin.y).abs() < 3. { return None; }
        let targets = self.ivars().targets.borrow();
        let (own, anchors) = (targets.0, &targets.1);
        let target = anchors.iter().rev().find(|a| a.top <= frame.origin.y + 0.5)
            .or_else(|| anchors.first()).copied()?;
        // In front of itself or of the block after it, the image does not move.
        (target.index != own && target.index != own + 1).then_some(target)
    }
    fn show_anchor(&self, target: Option<DropAnchor>) {
        if let Some(text) = self.ivars().text.load() {
            if let Some(editor) = text.downcast_ref::<MarkdownEditor>() {
                let y = target.map(|a| a.top);
                if editor.ivars().image_drop_line.replace(y) != y { text.setNeedsDisplay(true); }
            }
        }
    }
    fn clear_preview(&self) {
        if let Some(view) = self.ivars().preview.borrow_mut().take() { view.removeFromSuperview(); }
    }
    unsafe fn preview_move(&self, text: &NSTextView, original: NSRect, frame: NSRect) {
        let source = text.string().to_string();
        let Some((source, selected)) = self.layout_edit(&source, original, frame, true) else {
            self.clear_preview();
            return;
        };
        let mut preview = self.ivars().preview.borrow_mut();
        let view = preview.get_or_insert_with(|| {
            let (scroll, view) = make_editor(text.bounds());
            scroll.setDocumentView(None);
            view.setAllowsUndo(false);
            view.setFrame(text.bounds());
            text.addSubview(&view);
            view
        });
        view.setString(&NSString::from_str(&source));
        if let Some(editor) = view.downcast_ref::<MarkdownEditor>() {
            editor.ivars().selected_image.set(Some(selected));
        }
        render_markdown(view, self.ivars().base.as_deref(), None);
        if let Some(editor) = view.downcast_ref::<MarkdownEditor>() {
            let line = self.target(original, frame).and_then(|_|
                layout_fragment_rect(view, source[..selected].encode_utf16().count()));
            editor.ivars().image_drop_line.set(line.map(|r| r.origin.y));
            view.setNeedsDisplay(true);
        }
    }
    /// The note after this drag and where the image's link starts in it.
    /// Moving picks a side from the image's centre and a block boundary from
    /// its top; resizing only stores the new width.
    unsafe fn layout_edit(&self, source: &str, original: NSRect, frame: NSRect, moving: bool)
        -> Option<(String, usize)> {
        if fingerprint(source) != self.ivars().fingerprint { return None; }
        let text = self.ivars().text.load()?;
        let link = &self.ivars().link;
        let current = placement(link);
        if !moving {
            let mut result = source.to_owned();
            result.replace_range(link.bytes.clone(), &placed(link, frame.size.width, current));
            return (result != source).then_some((result, link.bytes.start));
        }
        let note = note_structure(source);
        let own = note_blocks::block_at(&note, link.bytes.start).filter(|i| note[*i].kind == note_blocks::Kind::Image)?;
        let body = (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0).max(80.0);
        let right = frame.origin.x + frame.size.width / 2.0 - EDITOR_INSET > body / 2.0;
        let side = if right { Placement::Right } else if current == Placement::Right { Placement::Left } else { current };
        // Rewriting the link also normalizes it, so only touch it for a new side.
        let mut result = if side == current { source.to_owned() } else { with_placement(source, &note[own], side, original.size.width)? };
        let mut selected = link.bytes.start;
        if let Some(target) = self.target(original, frame) {
            // Only the link changed length, so the same block indices apply.
            let (moved, landed) = note_blocks::reordered(&result, &note_structure(&result), &[own], Some(target.index))?;
            selected = landed.start + moved[landed].find("![").unwrap_or(0);
            result = moved;
        }
        (result != source).then_some((result, selected))
    }
    unsafe fn commit_layout(&self, original: NSRect, frame: NSRect, moving: bool) -> bool {
        let Some(text) = self.ivars().text.load() else { return false };
        let source = text.string().to_string();
        let Some((result, selected)) = self.layout_edit(&source, original, frame, moving) else { return false };
        let (span, replacement) = note_blocks::edit_span(&source, &result);
        self.replace_range(replacement, utf16_range(&source, span));
        select_image(&text, Some(selected));
        true
    }
    unsafe fn replace(&self, replacement: &str) {
        self.replace_range(replacement, self.ivars().range);
    }
    unsafe fn replace_range(&self, replacement: &str, old: NSRange) {
        let _keep = Retained::retain(self as *const Self as *mut Self);
        let Some(text) = self.ivars().text.load() else {
            return;
        };
        if !text.isEditable() {
            return;
        }
        let marked: bool = msg_send![&*text, hasMarkedText];
        if marked || fingerprint(&text.string().to_string()) != self.ivars().fingerprint {
            return;
        }
        let selection: NSRange = msg_send![&*text, selectedRange];
        let scroll = text.visibleRect().origin;

        let delta = replacement.encode_utf16().count() as isize - old.length as isize;
        INTERACTING.with(|v| v.set(false));
        text.breakUndoCoalescing();
        let _: () = msg_send![&*text,insertText:&*NSString::from_str(replacement),replacementRange:old];
        text.breakUndoCoalescing();
        let restored = if selection.location >= old.location + old.length {
            NSRange::new(
                selection.location.saturating_add_signed(delta),
                selection.length,
            )
        } else if selection.location >= old.location {
            NSRange::new(old.location + replacement.encode_utf16().count(), 0)
        } else {
            selection
        };
        text.setSelectedRange(restored);
        style_with_base(&text, self.ivars().base.as_deref());
        if let Some(window) = text.window() {
            window.makeFirstResponder(Some(&text));
        }
        text.scrollPoint(scroll);
    }
}

#[cfg(test)]
mod resize_tests {
    use super::*;
    #[test]
    fn expanded_targets_cover_eight_directions_without_stealing_move_area() {
        let size = NSSize::new(300.0, 180.0);
        for (point, handle) in [
            (NSPoint::new(44., 44.), ResizeHandle::TopLeft),
            (NSPoint::new(256., 44.), ResizeHandle::TopRight),
            (NSPoint::new(44., 136.), ResizeHandle::BottomLeft),
            (NSPoint::new(256., 136.), ResizeHandle::BottomRight),
            (NSPoint::new(150., 150.), ResizeHandle::Bottom),
            (NSPoint::new(150., 30.), ResizeHandle::Top),
            (NSPoint::new(30., 90.), ResizeHandle::Left),
            (NSPoint::new(270., 90.), ResizeHandle::Right),
        ] {
            assert_eq!(resize_handle(point, size), Some(handle));
        }
        assert_eq!(resize_handle(NSPoint::new(150., 90.), size), None);
        assert_eq!(
            resize_handle(NSPoint::new(16., 10.), NSSize::new(32., 20.)),
            None
        );
        assert_eq!(resize_handle(NSPoint::new(-1., -1.), size), None);
    }
    #[test]
    fn all_handles_resize_in_their_own_direction_and_preserve_ratio() {
        let original = NSRect::new(
            NSPoint::new(EDITOR_INSET + 150., 80.),
            NSSize::new(240., 144.),
        );
        for handle in ResizeHandle::ALL {
            let dx = if handle.left() { -40. } else { 40. };
            let dy = if handle.top() { -24. } else { 24. };
            let resized = resized_frame(original, handle, dx, dy, 800.);
            assert!((resized.size.width - 280.).abs() < 0.0001, "{handle:?}");
            assert!((resized.size.height / resized.size.width - 0.6).abs() < 0.0001);
            assert_eq!(
                resized.origin.y, original.origin.y,
                "Markdown paragraph remains anchored"
            );
            if handle.left() {
                assert!(
                    (resized.origin.x + resized.size.width
                        - original.origin.x
                        - original.size.width)
                        .abs()
                        < 0.0001
                );
            } else {
                assert_eq!(resized.origin.x, original.origin.x);
            }
            let vertical = resized_frame(original, handle, 0., dy, 800.);
            if matches!(handle, ResizeHandle::Left | ResizeHandle::Right) {
                assert_eq!(vertical, original, "side handles ignore perpendicular motion");
            } else {
                assert!(vertical.size.width > original.size.width, "vertical-only {handle:?}");
            }
            if matches!(handle, ResizeHandle::Top | ResizeHandle::Bottom) {
                assert_eq!(resized_frame(original, handle, dx, 0., 800.), original);
            }
        }
    }
    #[test]
    fn resize_stays_inside_editor_and_never_flips() {
        let original = NSRect::new(
            NSPoint::new(EDITOR_INSET + 40., 80.),
            NSSize::new(240., 144.),
        );
        for handle in ResizeHandle::ALL {
            for motion in [-10000., 10000.] {
                let resized = resized_frame(original, handle, motion, motion, 600.);
                assert!(resized.size.width >= 32.);
                assert!(resized.origin.x >= EDITOR_INSET);
                assert!(resized.origin.x + resized.size.width <= EDITOR_INSET + 600.);
            }
        }
    }
}
