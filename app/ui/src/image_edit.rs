//! Clipboard attachments and native image controls; all document edits use NSTextView undo.
use super::*;
use objc2_foundation::NSData;
use std::cell::Cell;
#[path = "../../shared/image_markdown.rs"]
#[allow(dead_code)]
pub mod markdown;

type Importer = fn(&[u8], &str) -> Result<(String, PathBuf), String>;
thread_local! {
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
        let image =
            NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path.to_str()?))?;
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
            let native = &*(text as *const NSTextView as *const MarkdownEditor);
            *native.ivars().base_directory.borrow_mut() = Some(base.clone());
            let source = text.string().to_string();
            let range: NSRange = msg_send![text, selectedRange];
            let start = byte_for_utf16(&source, range.location);
            let prefix = if start > 0 && !source[..start].ends_with('\n') {
                "\n\n"
            } else {
                ""
            };
            let replacement = format!(
                "{prefix}![이미지](<{destination}> \"stology:width=280;x=0.000;wrap=1\")\n\n"
            );
            text.breakUndoCoalescing();
            let _: () = msg_send![text,insertText:&*NSString::from_str(&replacement),replacementRange:range];
            text.breakUndoCoalescing();
            style_with_base(text, Some(&base));
        }
        Err(message) => show_error(&message),
    }
    true
}
unsafe fn show_error(message: &str) {
    let alert = NSAlert::new(MainThreadMarker::new().unwrap());
    alert.setMessageText(&NSString::from_str("이미지를 붙여넣지 못했습니다"));
    alert.setInformativeText(&NSString::from_str(message));
    alert.runModal();
}

pub(super) struct ImageState {
    pub text: Weak<NSTextView>,
    pub link: markdown::ImageLink,
    pub fingerprint: u64,
    pub base: Option<PathBuf>,
    pub range: NSRange,
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
            let p=self.convertPoint_fromView(event.locationInWindow(),None);
            let resize=resize_handle(p,self.bounds().size);
            self.ivars().drag.set(Some((event.locationInWindow(),self.frame(),resize)));
            INTERACTING.with(|v|v.set(true));
            if let Some(handle)=resize {resize_cursor(handle).set();} else {NSCursor::closedHandCursor().set();}
        }}
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self,event:&NSEvent) {unsafe {
            let Some((start,original,resize))=self.ivars().drag.get() else {return;};
            let Some(text)=self.ivars().text.load() else {return;};
            let delta=event.locationInWindow();
            let max_width=(text.bounds().size.width-2.0*EDITOR_INSET-8.0).max(80.0);
            let mut frame=original;
            if let Some(handle)=resize {
                // Window coordinates point up; image/text coordinates point down.
                // The Markdown paragraph stays vertically anchored while its size changes.
                frame=resized_frame(original,handle,delta.x-start.x,start.y-delta.y,max_width);
            } else {
                frame.origin.x=(original.origin.x+delta.x-start.x).clamp(EDITOR_INSET,EDITOR_INSET+(max_width-frame.size.width).max(0.0));
            }
            self.setFrame(frame);
            if let Some(handle)=resize {resize_cursor(handle).set();} else {NSCursor::closedHandCursor().set();}
            let _: ()=msg_send![self,setNeedsDisplay:true];
        }}
        #[unsafe(method(mouseUp:))]
        fn up(&self,_event:&NSEvent) {unsafe {
            let _keep=Retained::retain(self as *const Self as *mut Self);
            let drag=self.ivars().drag.replace(None);
            INTERACTING.with(|v|v.set(false));
            let Some((_,original,_))=drag else {return;};
            let frame=self.frame();
            if frame!=original { self.commit_layout(frame.size.width, self.position(), self.ivars().link.wrap); }
            else if let Some(text)=self.ivars().text.load() {
                // Put the caret in the paragraph beside the image, not its concealed syntax.
                let source=text.string().to_string();
                let end=self.ivars().link.bytes.end;
                let after=end+usize::from(source[end..].starts_with('\n'));
                text.setSelectedRange(NSRange::new(source[..after].encode_utf16().count(),0));
                if let Some(window)=text.window(){window.makeFirstResponder(Some(&text));}
            }
        }}
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self,event:&NSEvent) {unsafe {
            let _keep=Retained::retain(self as *const Self as *mut Self);
            INTERACTING.with(|v|v.set(true));
            let _: ()=msg_send![super(self),rightMouseDown:event];
            INTERACTING.with(|v|v.set(false));
        }}
        #[unsafe(method(menuForEvent:))]
        fn menu(&self,_event:&NSEvent)->*mut NSMenu {unsafe {
            if !self.ivars().text.load().is_some_and(|t|t.isEditable()) {return std::ptr::null_mut();}
            let menu=NSMenu::new(MainThreadMarker::new().unwrap());
            for (title,action) in [("이미지 삭제",sel!(deleteImage:)),("왼쪽에 배치 · 옆에 필기",sel!(alignLeft:)),("오른쪽에 배치 · 옆에 필기",sel!(alignRight:)),("가운데에 배치 · 옆에 필기",sel!(alignCenter:)),("한 줄 전체 사용",sel!(blockImage:))] {
                let item=NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(MainThreadMarker::new().unwrap()),&NSString::from_str(title),Some(action),&NSString::from_str(""));
                item.setTarget(Some(self)); menu.addItem(&item);
            }
            Retained::autorelease_return(menu)
        }}
        #[unsafe(method(deleteImage:))]
        fn delete(&self,_sender:Option<&AnyObject>) {unsafe {self.replace("");}}
        #[unsafe(method(alignLeft:))]
        fn left(&self,_sender:Option<&AnyObject>) {unsafe {self.commit_layout(self.frame().size.width,0.0,true);}}
        #[unsafe(method(alignRight:))]
        fn right(&self,_sender:Option<&AnyObject>) {unsafe {self.commit_layout(self.frame().size.width,1.0,true);}}
        #[unsafe(method(alignCenter:))]
        fn center(&self,_sender:Option<&AnyObject>) {unsafe {self.commit_layout(self.frame().size.width,0.5,true);}}
        #[unsafe(method(blockImage:))]
        fn block(&self,_sender:Option<&AnyObject>) {unsafe {self.commit_layout(self.frame().size.width,0.0,false);}}
    }
);
impl InlineImage {
    unsafe fn position(&self) -> f64 {
        let Some(text) = self.ivars().text.load() else {
            return 0.0;
        };
        let span = (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0 - self.frame().size.width)
            .max(1.0);
        ((self.frame().origin.x - EDITOR_INSET) / span).clamp(0.0, 1.0)
    }
    unsafe fn commit_layout(&self, width: f64, x: f64, wrap: bool) {
        let link = &self.ivars().link;
        self.replace(&link.markdown(&link.destination, width, x, wrap));
    }
    unsafe fn replace(&self, replacement: &str) {
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
        let old = self.ivars().range;
        let delta = replacement.encode_utf16().count() as isize - old.length as isize;
        INTERACTING.with(|v| v.set(false));
        text.breakUndoCoalescing();
        let _: () = msg_send![&*text,insertText:&*NSString::from_str(replacement),replacementRange:self.ivars().range];
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
