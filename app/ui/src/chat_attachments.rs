//! 채팅 이미지 첨부: 붙여넣기·끌어놓기·파일 선택으로 받은 이미지를 정규화하고,
//! 입력창 바로 위의 가로 스크롤 줄에 "Image #N" 썸네일로 보여 준다.
use super::*;
use objc2::DefinedClass;
use objc2::runtime::AnyClass;

pub const MAX_IMAGES: usize = 10;
/// 공급자 한도(base64 5MB)를 넘지 않도록 원본 바이트를 이 크기 이하로 맞춘다.
const MAX_BYTES: usize = 3_600_000;
const MAX_SIDE: isize = 2048;
const IMAGE_EXTENSIONS: [&str; 9] = ["png", "jpg", "jpeg", "gif", "webp", "heic", "tif", "tiff", "bmp"];
pub const TILE: f64 = 64.;
pub const GAP: f64 = 8.;
/// 썸네일 줄 높이. 위아래로 호버 그림자와 닫기 버튼이 잘리지 않을 여백을 둔다.
pub const STRIP_HEIGHT: f64 = TILE + 8.;
pub const CLOSE_TAG: isize = 3000;

pub struct Attachment {
    pub mime: String,
    pub data: String,
    image: Retained<NSImage>,
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for b in bytes.chunks(3) {
        let n = ((b[0] as u32) << 16) | ((b.get(1).copied().unwrap_or(0) as u32) << 8) | (b.get(2).copied().unwrap_or(0) as u32);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if b.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if b.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}

fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(&[0xff, 0xd8, 0xff]) { Some("image/jpeg") }
    else if bytes.starts_with(b"GIF8") { Some("image/gif") }
    else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("image/webp") }
    else { None }
}

/// 원본이 작고 공급자가 읽는 형식이면 그대로 쓰고, 아니면 긴 변 2048px 이하로 줄여
/// PNG(크면 JPEG)로 다시 인코딩한다.
unsafe fn normalize(bytes: &[u8]) -> Result<Attachment, String> {
    if bytes.len() > 80 * 1024 * 1024 { return Err("이미지가 너무 큽니다".into()); }
    let data = NSData::with_bytes(bytes);
    let image = NSImage::initWithData(NSImage::alloc(), &data).ok_or("이미지를 읽을 수 없습니다")?;
    let (mut pw, mut ph) = (0isize, 0isize);
    for rep in image.representations().iter() {
        pw = pw.max(rep.pixelsWide());
        ph = ph.max(rep.pixelsHigh());
    }
    if pw <= 0 || ph <= 0 {
        let size = image.size();
        pw = size.width.round() as isize;
        ph = size.height.round() as isize;
    }
    if pw <= 0 || ph <= 0 || pw.saturating_mul(ph) > 100_000_000 {
        return Err("이미지 크기가 지원 범위를 벗어났습니다".into());
    }
    if let Some(mime) = sniff(bytes) {
        if bytes.len() <= MAX_BYTES && pw.max(ph) <= MAX_SIDE {
            return Ok(Attachment { mime: mime.into(), data: base64(bytes), image });
        }
    }
    let scale = (MAX_SIDE as f64 / pw.max(ph) as f64).min(1.);
    let (w, h) = (((pw as f64) * scale).round().max(1.) as isize, ((ph as f64) * scale).round().max(1.) as isize);
    let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
        NSBitmapImageRep::alloc(), std::ptr::null_mut(), w, h, 8, 4, true, false, NSDeviceRGBColorSpace, 0, 0,
    ).ok_or("이미지를 변환하지 못했습니다")?;
    rep.setSize(NSSize::new(w as f64, h as f64));
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).ok_or("이미지를 변환하지 못했습니다")?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    context.setImageInterpolation(NSImageInterpolation::High);
    image.drawInRect_fromRect_operation_fraction(rect(0., 0., w as f64, h as f64), NSRect::ZERO, NSCompositingOperation::Copy, 1.);
    context.flushGraphics();
    NSGraphicsContext::restoreGraphicsState_class();
    let png = rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()).map(|d| d.to_vec());
    let (mime, encoded) = match png {
        Some(png) if png.len() <= MAX_BYTES => ("image/png", png),
        _ => {
            let mut found = None;
            for quality in [0.85, 0.7, 0.5] {
                let key: &NSString = NSImageCompressionFactor;
                let value = NSNumber::new_f64(quality);
                let props = NSDictionary::from_slices(&[key], &[&*value as &AnyObject]);
                let jpeg = rep.representationUsingType_properties(NSBitmapImageFileType::JPEG, &props).map(|d| d.to_vec());
                if let Some(jpeg) = jpeg.filter(|j| j.len() <= MAX_BYTES) { found = Some(jpeg); break; }
            }
            ("image/jpeg", found.ok_or("이미지가 너무 커서 첨부할 수 없어요")?)
        }
    };
    Ok(Attachment { mime: mime.into(), data: base64(&encoded), image })
}

fn is_image_path(path: &str) -> bool {
    std::path::Path::new(path).extension().and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

pub unsafe fn from_file(path: &str) -> Result<Attachment, String> {
    let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let meta = std::fs::metadata(path).map_err(|e| format!("{name} · {e}"))?;
    if meta.len() > 80 * 1024 * 1024 { return Err(format!("{name} · 이미지가 너무 큽니다")); }
    let bytes = std::fs::read(path).map_err(|e| format!("{name} · {e}"))?;
    normalize(&bytes).map_err(|e| format!("{name} · {e}"))
}

unsafe fn file_paths(board: &NSPasteboard) -> Vec<String> {
    let classes = NSArray::from_slice(&[NSURL::class() as &AnyClass]);
    let key: &NSString = NSPasteboardURLReadingFileURLsOnlyKey;
    let yes = NSNumber::new_bool(true);
    let options = NSDictionary::from_slices(&[key], &[&*yes as &AnyObject]);
    let Some(items) = board.readObjectsForClasses_options(&classes, Some(&options)) else { return Vec::new() };
    items.iter().filter_map(|item| {
        let url: Option<Retained<NSURL>> = item.downcast::<NSURL>().ok();
        url.and_then(|u| u.path()).map(|p| p.to_string())
    }).collect()
}

/// 대지에 이미지가 있으면 첨부 후보를 돌려준다. 이미지가 아니면 None이라 평소처럼
/// 글자를 붙여넣는다. `text_first`면 글자가 함께 있을 때 글자를 우선한다(붙여넣기).
pub unsafe fn from_pasteboard(board: &NSPasteboard, text_first: bool) -> Option<Vec<Result<Attachment, String>>> {
    let paths = file_paths(board);
    if !paths.is_empty() {
        let images: Vec<_> = paths.iter().filter(|p| is_image_path(p)).collect();
        if images.is_empty() { return None; }
        return Some(images.into_iter().map(|p| from_file(p)).collect());
    }
    let types = board.types().map(|t| t.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap_or_default();
    if text_first && types.iter().any(|t| t == &NSPasteboardTypeString.to_string()) { return None; }
    for kind in [NSPasteboardTypePNG, NSPasteboardTypeTIFF] {
        if let Some(data) = board.dataForType(kind) {
            return Some(vec![normalize(&data.to_vec())]);
        }
    }
    None
}

pub unsafe fn pasteboard_has_images(board: &NSPasteboard) -> bool {
    let paths = file_paths(board);
    if !paths.is_empty() { return paths.iter().any(|p| is_image_path(p)); }
    board.availableTypeFromArray(&NSArray::from_slice(&[NSPasteboardTypePNG, NSPasteboardTypeTIFF])).is_some()
}

/// 파일 선택 창으로 이미지를 고른다. 취소하면 빈 목록.
pub unsafe fn choose() -> Vec<Result<Attachment, String>> {
    let panel = NSOpenPanel::openPanel(mtm());
    panel.setCanChooseFiles(true);
    panel.setCanChooseDirectories(false);
    panel.setAllowsMultipleSelection(true);
    panel.setTitle(Some(&NSString::from_str("첨부할 이미지 선택")));
    panel.setMessage(Some(&NSString::from_str("PNG, JPEG, GIF, WebP, HEIC · 한 번에 10장까지")));
    if panel.runModal() != NSModalResponseOK { return Vec::new(); }
    panel.URLs().iter().filter_map(|u| u.path()).map(|p| p.to_string())
        .map(|p| if is_image_path(&p) { from_file(&p) } else { Err(format!("{} · 이미지 파일이 아닙니다", std::path::Path::new(&p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())) })
        .collect()
}

/// 썸네일 한 칸. 마우스를 올리면 오른쪽 위 닫기 버튼이 보인다.
pub struct TileState { close: RefCell<Option<Retained<NSButton>>> }
define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyChatAttachmentTile"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TileState]
    struct Tile;
    impl Tile {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            for area in self.trackingAreas().iter() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveAlways
                    | NSTrackingAreaOptions::InVisibleRect, Some(self), None);
            self.addTrackingArea(&area);
        } }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _event: &NSEvent) { self.show_close(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { self.show_close(false); }
    }
);
impl Tile {
    fn show_close(&self, shown: bool) {
        if let Some(close) = &*self.ivars().close.borrow() { close.setHidden(!shown); }
    }
}

/// 첨부 줄의 스크롤 뷰. 스크롤 막대 대신 넘친 쪽 가장자리를 흐리게 하고,
/// 썸네일을 잡아 끌어서 좌우로 넘길 수 있다.
const EDGE_FADE: f64 = 28.;
thread_local! {static DRAGGING: Cell<bool> = const { Cell::new(false) };}
define_class!(
    #[unsafe(super(NSScrollView, NSView, NSResponder))]
    #[name = "StologyChatAttachmentStrip"]
    #[thread_kind = MainThreadOnly]
    struct StripScroll;
    impl StripScroll {
        #[unsafe(method(reflectScrolledClipView:))]
        fn reflect(&self, clip: &NSClipView) { unsafe {
            let _: () = msg_send![super(self), reflectScrolledClipView: clip];
            edge_fade(self);
        } }
        #[unsafe(method(mouseDown:))]
        fn down(&self, _event: &NSEvent) {
            if overflow(self) > 0. && !DRAGGING.replace(true) { NSCursor::closedHandCursor().push(); }
        }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, event: &NSEvent) { unsafe {
            let max = overflow(self);
            if max <= 0. { return; }
            let clip = self.contentView();
            let x = (clip.bounds().origin.x - event.deltaX()).clamp(0., max);
            clip.scrollToPoint(NSPoint::new(x, 0.));
            self.reflectScrolledClipView(&clip);
        } }
        #[unsafe(method(mouseUp:))]
        fn up(&self, _event: &NSEvent) { if DRAGGING.replace(false) { NSCursor::pop_class(); } }
    }
);
fn overflow(scroll: &NSScrollView) -> f64 {
    let document = scroll.documentView().map(|d| d.frame().size.width).unwrap_or(0.);
    (document - scroll.contentSize().width).max(0.)
}
/// 왼쪽·오른쪽에 더 볼 썸네일이 있을 때만 그쪽 가장자리를 투명하게 흐린다.
unsafe fn edge_fade(scroll: &NSScrollView) {
    scroll.setWantsLayer(true);
    let Some(layer) = scroll.layer() else { return };
    let Some(class) = AnyClass::get(c"CAGradientLayer") else { return };
    let size = scroll.frame().size;
    let max = overflow(scroll);
    let x = scroll.contentView().bounds().origin.x;
    if max <= 0. || size.width <= EDGE_FADE * 2. {
        let _: () = msg_send![&layer, setMask: std::ptr::null::<AnyObject>()];
        return;
    }
    let mask: Retained<AnyObject> = msg_send![class, layer];
    let _: () = msg_send![&mask, setFrame: rect(0., 0., size.width, size.height)];
    let alpha = |a: f64| NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., a).CGColor();
    let left = alpha(if x > 0.5 { 0. } else { 1. });
    let right = alpha(if x < max - 0.5 { 0. } else { 1. });
    let solid = alpha(1.);
    let as_object = |c: &Retained<_>| &*(Retained::as_ptr(c) as *const AnyObject);
    let colors = [as_object(&left), as_object(&solid), as_object(&solid), as_object(&right)];
    let _: () = msg_send![&mask, setColors: &*NSArray::from_slice(&colors)];
    let edge = EDGE_FADE / size.width;
    let stops: Vec<Retained<NSNumber>> = [0., edge, 1. - edge, 1.].iter().map(|v| NSNumber::new_f64(*v)).collect();
    let _: () = msg_send![&mask, setLocations: &*NSArray::from_retained_slice(&stops)];
    let _: () = msg_send![&mask, setStartPoint: NSPoint::new(0., 0.5)];
    let _: () = msg_send![&mask, setEndPoint: NSPoint::new(1., 0.5)];
    let _: () = msg_send![&layer, setMask: &*mask];
}

pub struct Strip {
    pub scroll: Retained<NSScrollView>,
    document: Retained<NSView>,
    tiles: Vec<Retained<NSView>>,
}
impl Strip {
    pub unsafe fn new() -> Self {
        let scroll: Retained<StripScroll> = msg_send![StripScroll::alloc(mtm()), initWithFrame: rect(0., 0., 300., STRIP_HEIGHT)];
        let scroll: Retained<NSScrollView> = scroll.into_super();
        // 스크롤 막대는 숨긴다. 트랙패드·휠 가로 스크롤과 끌기는 그대로 된다.
        scroll.setHasHorizontalScroller(false);
        scroll.setHasVerticalScroller(false);
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        scroll.setVerticalScrollElasticity(NSScrollElasticity::None);
        let document = view(rect(0., 0., 10., STRIP_HEIGHT));
        scroll.setDocumentView(Some(&document));
        scroll.setHidden(true);
        let _: () = msg_send![&scroll, setAccessibilityLabel:&*NSString::from_str("첨부한 이미지")];
        Self { scroll, document, tiles: Vec::new() }
    }
    pub unsafe fn rebuild(&mut self, items: &[Attachment], target: &AnyObject) {
        for tile in self.tiles.drain(..) { tile.removeFromSuperview(); }
        for (i, item) in items.iter().enumerate() {
            let allocated = Tile::alloc(mtm()).set_ivars(TileState { close: RefCell::new(None) });
            let tile: Retained<Tile> = msg_send![super(allocated), initWithFrame: rect(i as f64 * (TILE + GAP), 4., TILE, TILE)];
            let name = format!("Image #{}", i + 1);
            // 사진은 칸을 꽉 채우고(aspect fill) 둥근 모서리로 자른다.
            let picture = view(rect(0., 0., TILE, TILE));
            picture.setWantsLayer(true);
            if let Some(layer) = picture.layer() {
                let _: () = msg_send![&layer, setContents:&*item.image];
                let _: () = msg_send![&layer, setContentsGravity:&*NSString::from_str("resizeAspectFill")];
                let _: () = msg_send![&layer, setCornerRadius:10f64];
                let _: () = msg_send![&layer, setMasksToBounds:true];
                let _: () = msg_send![&layer, setBorderWidth:0.5f64];
                let _: () = msg_send![&layer, setBorderColor:&*NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.12).CGColor()];
                let _: () = msg_send![&layer, setBackgroundColor:&*NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.06).CGColor()];
            }
            let _: () = msg_send![&picture, setAccessibilityElement:true];
            let _: () = msg_send![&picture, setAccessibilityRole:&*NSString::from_str("AXImage")];
            let _: () = msg_send![&picture, setAccessibilityLabel:&*NSString::from_str(&format!("첨부 이미지 {name}"))];
            tile.addSubview(&picture);
            // 왼쪽 아래 "Image #N" 이름표.
            let label = text_label(&name, rect(0., 0., 60., 14.), 9.5, true);
            label.setTextColor(Some(&NSColor::whiteColor()));
            label.setAlignment(NSTextAlignment::Center);
            let width = label.intrinsicContentSize().width + 10.;
            let pill = view(rect(4., 4., width.min(TILE - 8.), 15.));
            pill.setWantsLayer(true);
            if let Some(layer) = pill.layer() {
                let _: () = msg_send![&layer, setBackgroundColor:&*NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.55).CGColor()];
                let _: () = msg_send![&layer, setCornerRadius:7.5f64];
            }
            label.setFrame(rect(0., 0.5, width.min(TILE - 8.), 14.));
            let _: () = msg_send![&label, setAccessibilityElement:false];
            pill.addSubview(&label);
            tile.addSubview(&pill);
            let close = NSButton::buttonWithTitle_target_action(&NSString::from_str(""), Some(target), Some(sel!(chatAction:)), mtm());
            close.setTag(CLOSE_TAG + i as isize);
            close.setBordered(false);
            close.setImagePosition(NSCellImagePosition::ImageOnly);
            if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("xmark"), None) {
                let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(8., 0.4);
                close.setImage(image.imageWithSymbolConfiguration(&config).as_deref());
            }
            close.setContentTintColor(Some(&NSColor::whiteColor()));
            close.setFrame(rect(TILE - 22., TILE - 22., 18., 18.));
            close.setWantsLayer(true);
            if let Some(layer) = close.layer() {
                let _: () = msg_send![&layer, setBackgroundColor:&*NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.62).CGColor()];
                let _: () = msg_send![&layer, setCornerRadius:9f64];
            }
            let _: () = msg_send![&close, setAccessibilityLabel:&*NSString::from_str(&format!("{name} 첨부 취소"))];
            close.setHidden(true);
            tile.addSubview(&close);
            *tile.ivars().close.borrow_mut() = Some(close);
            let tile: Retained<NSView> = tile.into_super();
            self.document.addSubview(&tile);
            self.tiles.push(tile);
        }
        self.layout();
        self.scroll.setHidden(items.is_empty());
    }
    pub unsafe fn frame(&self, f: NSRect) {
        self.scroll.setFrame(f);
        self.layout();
    }
    unsafe fn layout(&self) {
        let n = self.tiles.len() as f64;
        let width = (n * (TILE + GAP) - GAP).max(0.).max(self.scroll.contentSize().width);
        self.document.setFrame(rect(0., 0., width, STRIP_HEIGHT));
        edge_fade(&self.scroll);
    }
    /// 새로 붙인 이미지가 보이도록 오른쪽 끝으로 넘긴다.
    pub unsafe fn reveal_last(&self) {
        let x = (self.document.frame().size.width - self.scroll.contentSize().width).max(0.);
        self.document.scrollPoint(NSPoint::new(x, 0.));
        edge_fade(&self.scroll);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_matches_reference_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
    #[test]
    fn image_types_are_sniffed_and_paths_filtered() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff(b"II*\0"), None);
        assert!(is_image_path("/tmp/Screen Shot.PNG"));
        assert!(is_image_path("/tmp/a.heic"));
        assert!(!is_image_path("/tmp/a.pdf"));
    }
}
