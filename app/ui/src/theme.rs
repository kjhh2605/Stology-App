//! Native macOS glass chrome; the document itself remains an opaque surface.
//!
//! macOS 26's NSGlassEffectView embeds controls through contentView. Older
//! systems fall back to NSVisualEffectView; no browser or shader runtime is used.
use objc2::{
    msg_send,
    rc::{Allocated, Retained},
    runtime::{AnyClass, AnyObject},
    AnyThread, ClassType, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{
    NSArray, NSData, NSMutableAttributedString, NSNumber, NSPoint, NSRange, NSRect, NSSize, NSString,
};

fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("theme requires main thread")
}
fn color(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}
unsafe fn layer(view: &NSView) -> Retained<AnyObject> {
    view.setWantsLayer(true);
    msg_send![view, layer]
}
unsafe fn shadow(view: &NSView, opacity: f32, radius: f64) {
    let l = layer(view);
    let ink = color(0.15, 0.18, 0.36, 1.);
    let _: () = msg_send![&l,setShadowColor:&*ink.CGColor()];
    let _: () = msg_send![&l,setShadowOpacity:opacity];
    let _: () = msg_send![&l,setShadowRadius:radius];
    let _: () = msg_send![&l,setShadowOffset:NSSize::new(0.,-5.)];
    let _: () = msg_send![&l,setMasksToBounds:false];
}

/// Return the native glass shell and the plain content view to populate.
/// Position/resize the shell in its parent. The content view is sized to bounds.
pub unsafe fn panel(frame: NSRect) -> (Retained<NSView>, Retained<NSView>) {
    let inner = NSView::initWithFrame(
        NSView::alloc(mtm()),
        NSRect::new(NSPoint::new(0., 0.), frame.size),
    );
    inner.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    // 뒤 배경과 무관하게 라벤더로 고정하기 위한 반투명 바탕(유리·구형 fallback 공통).
    let base = color(0.78, 0.73, 0.97, 0.05);
    let _: () = msg_send![&layer(&inner),setBackgroundColor:&*base.CGColor()];
    let outer: Retained<NSView> = if let Some(class) = AnyClass::get(c"NSGlassEffectView") {
        let allocated: Allocated<NSView> = msg_send![class, alloc];
        let shell: Retained<NSView> = msg_send![allocated,initWithFrame:frame];
        let _: () = msg_send![&shell,setContentView:&*inner];
        let _: () = msg_send![&shell,setCornerRadius:22.0f64];
        let _: () = msg_send![&shell,setStyle:0isize];
        let tint = color(0.70, 0.64, 0.95, 0.11);
        let _: () = msg_send![&shell,setTintColor:&*tint];
        shell
    } else {
        let shell = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm()), frame);
        shell.setMaterial(NSVisualEffectMaterial::Sidebar);
        shell.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        shell.setState(NSVisualEffectState::Active);
        shell.addSubview(&inner);
        shell.into_super()
    };
    glass_panel(&outer);
    (outer, inner)
}

/// Keep a manually laid-out body below the full-size native contentView.
/// NSGlassEffectView resets contentView to its bounds during deferred layout;
/// shortening that view directly makes resize deltas accumulate in its children.
pub unsafe fn panel_with_body(frame: NSRect, header_height: f64)
    -> (Retained<NSView>, Retained<NSView>, Retained<NSView>) {
    let (shell, content) = panel(frame);
    let body = NSView::initWithFrame(NSView::alloc(mtm()), NSRect::new(
        NSPoint::new(0., 0.), NSSize::new(frame.size.width, frame.size.height - header_height)));
    content.addSubview(&body);
    (shell, content, body)
}

/// A quiet, cool backdrop makes the native glass edges and refraction visible.
pub unsafe fn canvas(view: &NSView) {
    let l = layer(view);
    let background = color(0.91, 0.93, 0.98, 1.);
    let _: () = msg_send![&l,setBackgroundColor:&*background.CGColor()];
    if let Some(class) = AnyClass::get(c"CAGradientLayer") {
        let gradient: Retained<AnyObject> = msg_send![class, layer];
        let first = color(0.90, 0.92, 0.985, 1.);
        let middle = color(0.965, 0.972, 0.995, 1.);
        let last = color(0.88, 0.91, 0.97, 1.);
        let first_cg = first.CGColor();
        let middle_cg = middle.CGColor();
        let last_cg = last.CGColor();
        let colors = [
            Retained::as_ptr(&first_cg) as *const AnyObject,
            Retained::as_ptr(&middle_cg) as *const AnyObject,
            Retained::as_ptr(&last_cg) as *const AnyObject,
        ];
        let array: Retained<AnyObject> = msg_send![NSArray::<AnyObject>::class(),arrayWithObjects:colors.as_ptr(), count:colors.len()];
        let _: () = msg_send![&gradient,setColors:&*array];
        let _: () = msg_send![&gradient,setFrame:view.bounds()];
        let _: () = msg_send![&gradient,setAutoresizingMask:18u32];
        let _: () = msg_send![&gradient,setStartPoint:NSPoint::new(0.,1.)];
        let _: () = msg_send![&gradient,setEndPoint:NSPoint::new(1.,0.)];
        let _: () = msg_send![&gradient,setName:&*NSString::from_str("StologyCanvasGradient")];
        let _: () = msg_send![&l,insertSublayer:&*gradient, atIndex:0u32];
    }
}

pub unsafe fn glass_panel(view: &NSView) {
    let l = layer(view);
    let _: () = msg_send![&l,setCornerRadius:22.0f64];
    let _: () = msg_send![&l,setCornerCurve:&*NSString::from_str("continuous")];
    // NSGlassEffectView supplies its own refractive edge. The hairline and soft
    // external shadow also provide a clean fallback on pre-26 AppKit.
    // 흰 가장자리 선은 두지 않는다. 구형 AppKit fallback에서는 그림자만으로 경계를 구분한다.
    let _: () = msg_send![&l,setBorderWidth:0.0f64];
    shadow(view, 0.075, 18.);
}

pub unsafe fn document(view: &NSView) {
    let l = layer(view);
    let paper = NSColor::whiteColor();
    let border = color(0.75, 0.79, 0.89, 0.60);
    let _: () = msg_send![&l,setBackgroundColor:&*paper.CGColor()];
    let _: () = msg_send![&l,setCornerRadius:22.0f64];
    let _: () = msg_send![&l,setCornerCurve:&*NSString::from_str("continuous")];
    let _: () = msg_send![&l,setBorderColor:&*border.CGColor()];
    let _: () = msg_send![&l,setBorderWidth:0.65f64];
    shadow(view, 0.065, 20.);
}

/// A stable rounded shadow silhouette must not depend on transient rectangular
/// backing layers while AppKit resizes the glass/document during a sidebar slide.
pub unsafe fn sync_surface_shadow(view: &NSView) {
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(view.bounds(), 22., 22.).CGPath();
    let l = layer(view);
    // The sidebar already drives its own 60Hz geometry; don't animate shadowPath
    // independently or it can lag behind the rounded surface.
    if let Some(transaction) = AnyClass::get(c"CATransaction") {
        let _: () = msg_send![transaction, begin];
        let _: () = msg_send![transaction, setDisableActions:true];
        let _: () = msg_send![&l, setShadowPath:&*path];
        let _: () = msg_send![transaction, commit];
    } else {
        let _: () = msg_send![&l, setShadowPath:&*path];
    }
}

/// Native glass buttons keep platform hover/press/focus and accessibility.
/// Apply before a caller intentionally makes a list-row button borderless.
pub unsafe fn button(button: &NSButton, primary: bool) {
    button.setBezelStyle(if AnyClass::get(c"NSGlassEffectView").is_some() {
        NSBezelStyle::Glass
    } else {
        NSBezelStyle::Push
    });
    button.setControlSize(NSControlSize::Regular);
    button.setFont(Some(&crate::font::weighted(
        12.,
        if primary { 0.4 } else { 0.18 },
    )));
    button.setBordered(true);
    button.setShowsBorderOnlyWhileMouseInside(false);
    if primary {
        let indigo = color(79. / 255., 70. / 255., 229. / 255., 1.);
        button.setBezelColor(Some(&indigo));
        button.setContentTintColor(Some(&NSColor::whiteColor()));
        let title = NSMutableAttributedString::initWithString(
            NSMutableAttributedString::alloc(),
            &button.title(),
        );
        let range = NSRange::new(0, title.length());
        title.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::whiteColor(),
            range,
        );
        title.addAttribute_value_range(
            NSFontAttributeName,
            &crate::font::weighted(12., 0.4),
            range,
        );
        button.setAttributedTitle(&title);
    } else {
        button.setBezelColor(None);
        button.setContentTintColor(Some(&color(0.17, 0.19, 0.31, 1.)));
    }
}

/// 사이드바 아이콘 이름 → SF Symbols. 시스템 내장 기호라 별도 에셋이 없고,
/// Apple 플랫폼 앱 안에서 쓰는 용도로 허용된다. 앱 아이콘·로고에는 쓰지 않는다.
fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        "archive" => "archivebox",
        "note-plus" => "square.and.pencil",
        "folder" => "folder",
        "folder-open" => "folder.fill",
        "folder-plus" => "folder.badge.plus",
        "file-pdf" => "doc.richtext",
        "file-slides" => "rectangle.on.rectangle",
        "file-image" => "photo",
        "doc" => "doc.text",
        "file" => "doc.richtext",
        "pencil" => "pencil",
        "tag" => "tag",
        "trash" => "trash",
        "network" => "point.3.connected.trianglepath.dotted",
        "import" => "square.and.arrow.down",
        "review-document" => "doc.text.magnifyingglass",
        "markup" => "pencil.tip",
        "convert-document" => "doc.badge.arrow.up",
        "sparkle" => "sparkles",
        "fit" => "arrow.up.left.and.arrow.down.right",
        "focus" => "scope",
        "zoom-in" => "plus.magnifyingglass",
        "zoom-out" => "minus.magnifyingglass",
        "rotate" => "arrow.clockwise",
        "deselect" => "xmark.circle",
        "list" => "list.bullet",
        "stop" => "stop.circle",
        "link" => "link",
        "book" => "book.closed",
        "book-fill" => "book.closed.fill",
        "books-plus" => "plus",
        "key" => "person.crop.circle.badge.checkmark",
        "plus" => "plus.circle",
        _ => return None,
    })
}

pub fn indigo() -> Retained<NSColor> {
    color(79. / 255., 70. / 255., 229. / 255., 1.)
}

/// 사이드바 기호를 만든다. 색은 버튼 contentTint 또는 이미지 뷰 tint로 준다.
pub unsafe fn icon(name: &str, size: f64) -> Option<Retained<NSImage>> {
    let template: Option<&[u8]> = match name {
        "note-plus" => Some(include_bytes!("../../assets/note-plus.svg")),
        "broom" => Some(include_bytes!("../../assets/broom.svg")),
        "orb" => Some(include_bytes!("../../assets/graph-bullet.svg")),
        "quote" => Some(include_bytes!("../../assets/quote-opening.svg")),
        _ => None,
    };
    if let Some(bytes) = template {
        let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))?;
        if matches!(name, "quote" | "broom" | "note-plus") {
            image.setSize(NSSize::new(size, size));
        }
        image.setTemplate(true);
        return Some(image);
    }
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol(name)?),
        None,
    )?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, 0.0);
    image.imageWithSymbolConfiguration(&config)
}

/// 배지 달린 SF Symbol을 두 색으로 칠한다. 팔레트 순서는 [배지, 본체]다.
pub unsafe fn palette_icon(
    symbol: &str,
    size: f64,
    badge: &NSColor,
    body: &NSColor,
) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        None,
    )?;
    let sized = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, 0.0);
    let colors = NSArray::from_slice(&[badge, body]);
    let palette: Retained<NSImageSymbolConfiguration> = msg_send![
        NSImageSymbolConfiguration::class(),
        configurationWithPaletteColors: &*colors
    ];
    let config: Retained<NSImageSymbolConfiguration> =
        msg_send![&sized, configurationByApplyingConfiguration: &*palette];
    image.imageWithSymbolConfiguration(&config)
}

/// 섹션 제목 옆 장식 아이콘(인디고). 접근성 요소에서 제외한다.
pub unsafe fn icon_view(name: &str, frame: NSRect) -> Option<Retained<NSImageView>> {
    let image = icon(name, (frame.size.height * 0.62).round())?;
    let v = NSImageView::imageViewWithImage(&image, mtm());
    v.setFrame(frame);
    v.setContentTintColor(Some(&indigo()));
    let _: () = msg_send![&v, setAccessibilityElement: false];
    Some(v)
}

/// Filetype shape and a readable dark accent are shared by icon and extension badge.
pub fn file_type_symbol(extension: &str) -> &'static str {
    match extension.to_ascii_uppercase().as_str() {
        "MD" => "doc",
        "PDF" => "file-pdf",
        "PPT" | "PPTX" => "file-slides",
        "PNG" | "JPG" | "JPEG" | "HEIC" | "GIF" | "WEBP" | "TIFF" | "BMP" => "file-image",
        _ => "file",
    }
}
pub fn file_type_tint(extension: &str) -> Retained<NSColor> {
    match file_type_symbol(extension) {
        "file-pdf" => color(0.66, 0.20, 0.24, 1.),
        "file-slides" => color(0.62, 0.30, 0.09, 1.),
        "file-image" => color(0.10, 0.42, 0.42, 1.),
        _ => color(0.36, 0.38, 0.52, 1.),
    }
}
pub const FILE_TYPE_BADGE_WIDTH: f64 = 44.;
pub const FILE_TYPE_BADGE_HEIGHT: f64 = 18.;
pub unsafe fn file_type_badge(extension: &str, frame: NSRect) -> Retained<NSTextField> {
    let badge = NSTextField::labelWithString(&NSString::from_str(extension), mtm());
    badge.setFrame(frame);
    let font = crate::font::weighted(10., 0.3);
    let tint = file_type_tint(extension);
    let title = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &NSString::from_str(extension),
    );
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    title.addAttribute_value_range(NSForegroundColorAttributeName, &tint, range);
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setAlignment(NSTextAlignment::Center);
    title.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
    // NSTextField's label baseline sits high inside this 18pt badge.
    title.addAttribute_value_range(NSBaselineOffsetAttributeName, &NSNumber::numberWithDouble(-2.5), range);
    badge.setAttributedStringValue(&title);
    badge.setAlignment(NSTextAlignment::Center);
    badge.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    badge.setSelectable(false);
    let l = layer(&badge);
    let background = file_type_tint(extension).colorWithAlphaComponent(0.10);
    let _: () = msg_send![&l, setBackgroundColor: &*background.CGColor()];
    let _: () = msg_send![&l, setCornerRadius: 5.0f64];
    // Full extension is already part of the cell's accessibility name.
    let _: () = msg_send![&badge, setAccessibilityElement: false];
    badge
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Glass {
    /// 투명한 유리 캡슐
    Clear,
    /// 선택된 항목: 인디고가 옅게 비치는 유리
    Selected,
    /// 주요 동작: 인디고로 물든 유리(iOS glassProminent)
    Primary,
}

/// iOS/macOS 26 Liquid Glass 캡슐. NSGlassEffectView가 없으면 반투명 재질로 대체한다.
/// 돌려준 셸을 parent에 넣고, 두 번째 값(내용 뷰)에 버튼을 넣는다.
pub unsafe fn glass_shell(frame: NSRect, kind: Glass) -> (Retained<NSView>, Retained<NSView>) {
    let bounds = NSRect::new(NSPoint::new(0., 0.), frame.size);
    let inner = NSView::initWithFrame(NSView::alloc(mtm()), bounds);
    inner.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let radius = (frame.size.height / 2.).min(12.);
    let tint = match kind {
        // 패널 유리 위에서 회색으로 가라앉지 않도록 흰빛을 조금 섞는다.
        Glass::Clear => Some(color(1., 1., 1., 0.16)),
        Glass::Selected => Some(color(0.39, 0.40, 0.95, 0.16)),
        Glass::Primary => Some(color(0.33, 0.29, 0.92, 0.82)),
    };
    if let Some(class) = AnyClass::get(c"NSGlassEffectView") {
        let allocated: Allocated<NSView> = msg_send![class, alloc];
        let shell: Retained<NSView> = msg_send![allocated, initWithFrame: frame];
        let _: () = msg_send![&shell, setContentView: &*inner];
        let _: () = msg_send![&shell, setCornerRadius: radius];
        // 0 = regular: 뒤 배경이 은은하게 비치는 기본 유리
        let _: () = msg_send![&shell, setStyle: 0isize];
        if let Some(t) = &tint {
            let _: () = msg_send![&shell, setTintColor: &**t];
        }
        return (shell, inner);
    }
    let shell = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm()), frame);
    shell.setMaterial(NSVisualEffectMaterial::Popover);
    shell.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    shell.setState(NSVisualEffectState::Active);
    let l = layer(&shell);
    let _: () = msg_send![&l, setCornerRadius: radius];
    let _: () = msg_send![&l, setMasksToBounds: true];
    let edge = color(1., 1., 1., 0.7);
    let _: () = msg_send![&l, setBorderColor: &*edge.CGColor()];
    let _: () = msg_send![&l, setBorderWidth: 0.6f64];
    if let Some(t) = &tint {
        let il = layer(&inner);
        let _: () = msg_send![&il, setBackgroundColor: &*t.CGColor()];
    }
    shell.addSubview(&inner);
    (shell.into_super(), inner)
}

/// 유리 위에 올라가는 판 없는 버튼의 글꼴·색·아이콘을 맞춘다.
pub unsafe fn glass_button(button: &NSButton, kind: Glass, icon_name: Option<&str>, size: f64) {
    button.setBordered(false);
    let ink = match kind {
        Glass::Primary => NSColor::whiteColor(),
        Glass::Selected => color(0.26, 0.22, 0.78, 1.),
        Glass::Clear => color(0.13, 0.14, 0.24, 1.),
    };
    let weight = if kind == Glass::Clear { 0.0 } else { 0.3 };
    let font = crate::font::weighted(12.5, weight);
    button.setFont(Some(&font));
    let title = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &button.title(),
    );
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, &ink, range);
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    button.setAttributedTitle(&title);
    // 아이콘은 Clear일 때만 브랜드 인디고, 나머지는 글자색과 맞춘다.
    let tint = if kind == Glass::Clear { indigo() } else { ink };
    button.setContentTintColor(Some(&tint));
    set_icon(button, icon_name, size);
}

fn cg_array(colors: &[Retained<NSColor>]) -> Retained<AnyObject> {
    let cgs: Vec<_> = colors.iter().map(|c| c.CGColor()).collect();
    let ptrs: Vec<*const AnyObject> = cgs
        .iter()
        .map(|c| Retained::as_ptr(c) as *const AnyObject)
        .collect();
    unsafe {
        msg_send![NSArray::<AnyObject>::class(),arrayWithObjects:ptrs.as_ptr(), count:ptrs.len()]
    }
}

/// 판 없는 행 버튼의 제목 글꼴·색.
pub unsafe fn row_title(button: &NSButton, ink: &NSColor, weight: f64) {
    let font = crate::font::weighted(13., weight);
    button.setFont(Some(&font));
    let title = NSMutableAttributedString::initWithString(
        NSMutableAttributedString::alloc(),
        &button.title(),
    );
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, ink, range);
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    button.setAttributedTitle(&title);
}

/// 양끝이 투명하게 사라지고 가운데가 은은하게 빛나는 1pt 구분선.
pub unsafe fn divider(frame: NSRect) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&v);
    if let Some(class) = AnyClass::get(c"CAGradientLayer") {
        let g: Retained<AnyObject> = msg_send![class, layer];
        let edge = color(0.55, 0.57, 0.92, 0.);
        let mid = color(0.55, 0.57, 0.92, 0.42);
        let glow = color(1., 1., 1., 0.95);
        let _: () =
            msg_send![&g, setColors: &*cg_array(&[edge.clone(), mid.clone(), glow, mid, edge])];
        let _: () = msg_send![&g, setStartPoint: NSPoint::new(0., 0.5)];
        let _: () = msg_send![&g, setEndPoint: NSPoint::new(1., 0.5)];
        let _: () = msg_send![&g, setFrame: NSRect::new(NSPoint::new(0., 0.), frame.size)];
        let _: () = msg_send![&l, addSublayer: &*g];
    }
    let _: () = msg_send![&v, setAccessibilityElement: false];
    v
}

/// 사이드바용 차분한 1pt 구분선: 가운데 반짝임 없이 고른 인디고 선이 양끝 약 17%에서만 사라진다.
/// `strong`은 구역 경계(워크스페이스 추가 아래·하단 묶음 위), 아니면 워크스페이스 사이 선.
pub unsafe fn soft_divider(frame: NSRect, strong: bool) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&v);
    if let Some(class) = AnyClass::get(c"CAGradientLayer") {
        let g: Retained<AnyObject> = msg_send![class, layer];
        let alpha = if strong { 0.28 } else { 0.18 };
        let edge = color(0.357, 0.357, 0.839, 0.);
        let mid = color(0.357, 0.357, 0.839, alpha);
        // 같은 간격 7개 색: 0→1/6 구간에서 나타나고 5/6→1 구간에서 사라진다.
        let colors = [
            edge.clone(),
            mid.clone(),
            mid.clone(),
            mid.clone(),
            mid.clone(),
            mid,
            edge,
        ];
        let _: () = msg_send![&g, setColors: &*cg_array(&colors)];
        let _: () = msg_send![&g, setStartPoint: NSPoint::new(0., 0.5)];
        let _: () = msg_send![&g, setEndPoint: NSPoint::new(1., 0.5)];
        let _: () = msg_send![&g, setFrame: NSRect::new(NSPoint::new(0., 0.), frame.size)];
        // Follow sidebar width changes, including shrink-to-hide and restore.
        let _: () = msg_send![&g, setAutoresizingMask:18u32];
        let _: () = msg_send![&l, addSublayer: &*g];
    }
    let _: () = msg_send![&v, setAccessibilityElement: false];
    v
}

/// Resize the gradient explicitly: AppKit backing-layer resizing does not
/// reliably lay out custom sublayers during interactive sidebar resizing.
pub unsafe fn resize_soft_divider(view: &NSView, frame: NSRect) {
    let transaction = AnyClass::get(c"CATransaction");
    if let Some(t) = transaction {
        let _: () = msg_send![t, begin];
        let _: () = msg_send![t, setDisableActions: true];
    }
    view.setFrame(frame);
    let l = layer(view);
    let _: () = msg_send![&l, setMasksToBounds: true];
    let children: Option<Retained<NSArray<AnyObject>>> = msg_send![&l, sublayers];
    if let Some(children) = children {
        for child in children.iter() {
            let _: () = msg_send![&child, setFrame: view.bounds()];
        }
    }
    if let Some(t) = transaction { let _: () = msg_send![t, commit]; }
}

/// 선택된 행 바탕: 유리 질감. 위는 밝고 아래로 인디고가 비치는 반투명 그라데이션,
/// 윗면 반사광 선, 아랫면 옅은 인디고 반사, 밝은 테두리와 은은한 빛번짐.
pub unsafe fn selection_bg(frame: NSRect) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&v);
    let radius = 12.0f64;
    let border = color(0.50, 0.47, 0.99, 0.55);
    let glow = color(0.40, 0.36, 0.98, 1.);
    let _: () = msg_send![&l, setBorderColor: &*border.CGColor()];
    let _: () = msg_send![&l, setBorderWidth: 1.0f64];
    let _: () = msg_send![&l, setCornerRadius: radius];
    let _: () = msg_send![&l, setCornerCurve: &*NSString::from_str("continuous")];
    let _: () = msg_send![&l, setShadowColor: &*glow.CGColor()];
    let _: () = msg_send![&l, setShadowOpacity: 0.22f32];
    let _: () = msg_send![&l, setShadowRadius: 9.0f64];
    let _: () = msg_send![&l, setShadowOffset: NSSize::new(0., -1.)];
    let _: () = msg_send![&l, setMasksToBounds: false];
    if let Some(class) = AnyClass::get(c"CAGradientLayer") {
        let size = frame.size;
        let body: Retained<AnyObject> = msg_send![class, layer];
        let colors = [
            color(1., 1., 1., 0.62),
            color(0.86, 0.86, 1., 0.40),
            color(0.52, 0.49, 0.99, 0.26),
        ];
        let _: () = msg_send![&body, setColors: &*cg_array(&colors)];
        let _: () = msg_send![&body, setStartPoint: NSPoint::new(0.5, 1.)];
        let _: () = msg_send![&body, setEndPoint: NSPoint::new(0.5, 0.)];
        let _: () = msg_send![&body, setFrame: NSRect::new(NSPoint::new(0., 0.), size)];
        let _: () = msg_send![&body, setCornerRadius: radius];
        let _: () = msg_send![&body, setCornerCurve: &*NSString::from_str("continuous")];
        let _: () = msg_send![&l, addSublayer: &*body];
        // 비스듬한 유리 광택: 왼쪽 위에서 오른쪽 아래로 옅어지는 흰 막
        let sheen: Retained<AnyObject> = msg_send![class, layer];
        let sheen_colors = [color(1., 1., 1., 0.38), color(1., 1., 1., 0.)];
        let _: () = msg_send![&sheen, setColors: &*cg_array(&sheen_colors)];
        let _: () = msg_send![&sheen, setStartPoint: NSPoint::new(0., 1.)];
        let _: () = msg_send![&sheen, setEndPoint: NSPoint::new(0.55, 0.2)];
        let _: () = msg_send![&sheen, setFrame: NSRect::new(NSPoint::new(0., 0.), size)];
        let _: () = msg_send![&sheen, setCornerRadius: radius];
        let _: () = msg_send![&sheen, setCornerCurve: &*NSString::from_str("continuous")];
        let _: () = msg_send![&l, addSublayer: &*sheen];
        // 윗면 반사광 선: 양끝이 사라지는 흰 1pt
        let top: Retained<AnyObject> = msg_send![class, layer];
        let clear = color(1., 1., 1., 0.);
        let shine = color(1., 1., 1., 0.95);
        let _: () = msg_send![&top, setColors: &*cg_array(&[clear.clone(), shine, clear.clone()])];
        let _: () = msg_send![&top, setStartPoint: NSPoint::new(0., 0.5)];
        let _: () = msg_send![&top, setEndPoint: NSPoint::new(1., 0.5)];
        let _: () = msg_send![&top, setFrame: NSRect::new(NSPoint::new(radius, size.height - 1.5), NSSize::new(size.width - radius * 2., 1.))];
        let _: () = msg_send![&l, addSublayer: &*top];
        // 아랫면 옅은 인디고 반사
        let bottom: Retained<AnyObject> = msg_send![class, layer];
        let tint = color(0.45, 0.42, 0.99, 0.45);
        let clear2 = color(0.45, 0.42, 0.99, 0.);
        let _: () = msg_send![&bottom, setColors: &*cg_array(&[clear2.clone(), tint, clear2])];
        let _: () = msg_send![&bottom, setStartPoint: NSPoint::new(0., 0.5)];
        let _: () = msg_send![&bottom, setEndPoint: NSPoint::new(1., 0.5)];
        let _: () = msg_send![&bottom, setFrame: NSRect::new(NSPoint::new(radius, 1.5), NSSize::new(size.width - radius * 2., 1.))];
        let _: () = msg_send![&l, addSublayer: &*bottom];
    }
    let _: () = msg_send![&v, setAccessibilityElement: false];
    v
}

/// 사이드바 워크스페이스 선택 바탕: 채움·테두리 없이 행 바깥의 옅은 보랏빛
/// 그림자만 둔다(CSS `0 3px 10px rgba(110,90,220,.14)`). 행 안쪽은 패널 유리가
/// 그대로 보이도록 그림자를 행 모양 밖에서만 그린다.
pub const WORKSPACE_SELECTION_RADIUS: f64 = 12.;

pub unsafe fn soft_selection(frame: NSRect) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&v);
    let _: () = msg_send![&l, setMasksToBounds: false];
    let radius = WORKSPACE_SELECTION_RADIUS;
    let bounds = NSRect::new(NSPoint::new(0., 0.), frame.size);
    let shape = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, radius, radius);
    let (Some(plain), Some(shape_class)) = (AnyClass::get(c"CALayer"), AnyClass::get(c"CAShapeLayer")) else {
        let _: () = msg_send![&v, setAccessibilityElement: false];
        return v;
    };
    // 그림자를 그리는 레이어. 내용 없이 shadowPath만으로 행 모양 그림자를 만든다.
    let caster: Retained<AnyObject> = msg_send![plain, layer];
    let _: () = msg_send![&caster, setFrame: bounds];
    let glow = color(110. / 255., 90. / 255., 220. / 255., 1.);
    let _: () = msg_send![&caster, setShadowColor: &*glow.CGColor()];
    let _: () = msg_send![&caster, setShadowOpacity: 0.14f32];
    // CSS blur 10px는 Core Animation 반경 약 5에 해당한다.
    let _: () = msg_send![&caster, setShadowRadius: 5.0f64];
    let _: () = msg_send![&caster, setShadowOffset: NSSize::new(0., -3.)];
    let _: () = msg_send![&caster, setShadowPath: &*shape.CGPath()];
    // 행 안쪽에 그림자가 비치지 않도록 바깥만 남기는 컨테이너 마스크.
    let margin = 24.;
    let size = NSSize::new(frame.size.width + margin * 2., frame.size.height + margin * 2.);
    let container: Retained<AnyObject> = msg_send![plain, layer];
    let _: () = msg_send![&container, setFrame: NSRect::new(NSPoint::new(-margin, -margin), size)];
    let _: () = msg_send![&caster, setFrame: NSRect::new(NSPoint::new(margin, margin), frame.size)];
    let mask: Retained<AnyObject> = msg_send![shape_class, layer];
    let _: () = msg_send![&mask, setFrame: NSRect::new(NSPoint::new(0., 0.), size)];
    let path = NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::new(0., 0.), size));
    path.appendBezierPath(&NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        NSRect::new(NSPoint::new(margin, margin), frame.size), radius, radius,
    ));
    let _: () = msg_send![&mask, setPath: &*path.CGPath()];
    let _: () = msg_send![&mask, setFillRule: &*NSString::from_str("even-odd")];
    let _: () = msg_send![&mask, setFillColor: &*color(0., 0., 0., 1.).CGColor()];
    let _: () = msg_send![&container, setMask: &*mask];
    let _: () = msg_send![&container, addSublayer: &*caster];
    let _: () = msg_send![&l, addSublayer: &*container];
    let _: () = msg_send![&v, setAccessibilityElement: false];
    v
}

unsafe fn gradient_layer(bounds: NSRect, colors: &[Retained<NSColor>], stops: &[f64], horizontal: bool) -> Option<Retained<AnyObject>> {
    let g: Retained<AnyObject> = msg_send![AnyClass::get(c"CAGradientLayer")?, layer];
    let locations: Vec<_> = stops.iter().map(|&s| NSNumber::new_f64(s)).collect();
    let _: () = msg_send![&g, setColors: &*cg_array(colors)];
    let _: () = msg_send![&g, setLocations: &*NSArray::from_retained_slice(&locations)];
    let (start, end) = if horizontal { ((0., 0.5), (1., 0.5)) } else { ((0.5, 0.), (0.5, 1.)) };
    let _: () = msg_send![&g, setStartPoint: NSPoint::new(start.0, start.1)];
    let _: () = msg_send![&g, setEndPoint: NSPoint::new(end.0, end.1)];
    let _: () = msg_send![&g, setFrame: bounds];
    Some(g)
}

/// 대화 목록 시트의 선택된 대화 행 배경. 구분선 사이 칸을 모서리 없이 채우고,
/// 왼쪽 페리윙클에서 오른쪽 라벤더로 양 끝이 진하며 가운데는 맑다.
pub unsafe fn selection_wash(frame: NSRect) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let _: () = msg_send![&v, setAccessibilityElement: false];
    let l = layer(&v);
    let bounds = NSRect::new(NSPoint::new(0., 0.), frame.size);
    let Some(plain) = AnyClass::get(c"CALayer") else { return v; };
    let tint = |rgb: (f64, f64, f64), a: f64| color(rgb.0 / 255., rgb.1 / 255., rgb.2 / 255., a);
    let (left, mid, right) = ((126., 136., 240.), (150., 140., 238.), (176., 148., 240.));
    let across = [tint(left, 0.24), tint(left, 0.19), tint(left, 0.12), tint(mid, 0.07), tint(mid, 0.05),
        tint(mid, 0.07), tint(right, 0.12), tint(right, 0.18), tint(right, 0.22)];
    let across_stops = [0., 0.08, 0.2, 0.35, 0.5, 0.65, 0.8, 0.92, 1.];
    let along = [tint(mid, 0.07), tint(mid, 0.02), tint(mid, 0.02), tint(mid, 0.07)];
    // 좌우 끝은 구분선처럼 서서히, 위아래 끝은 구분선이 없는 첫 행에서도 딱 잘리지 않을 만큼만 흐린다.
    let ink = |a: f64| color(0., 0., 0., a);
    let fade_across = [ink(0.), ink(0.35), ink(0.75), ink(1.), ink(1.), ink(0.75), ink(0.35), ink(0.)];
    let edge = (5. / frame.size.height.max(10.)).min(0.5);
    let (Some(fill), Some(rim), Some(mask_across), Some(mask_along)) = (
        gradient_layer(bounds, &across, &across_stops, true),
        gradient_layer(bounds, &along, &[0., 0.35, 0.65, 1.], false),
        gradient_layer(bounds, &fade_across, &[0., 0.03, 0.07, 0.12, 0.88, 0.93, 0.97, 1.], true),
        gradient_layer(bounds, &[ink(0.4), ink(1.), ink(1.), ink(0.4)], &[0., edge, 1. - edge, 1.], false),
    ) else { return v; };
    let inner: Retained<AnyObject> = msg_send![plain, layer];
    let _: () = msg_send![&inner, setFrame: bounds];
    let _: () = msg_send![&inner, addSublayer: &*fill];
    let _: () = msg_send![&inner, addSublayer: &*rim];
    let _: () = msg_send![&inner, setMask: &*mask_along];
    let _: () = msg_send![&l, addSublayer: &*inner];
    let _: () = msg_send![&l, setMask: &*mask_across];
    v
}

/// A small, one-shot lift on explicit workspace selection. Only presentation
/// layers move: row geometry, hit targets and scroll layout stay stable.
pub unsafe fn float_selection(view: &NSView) {
    if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
        return;
    }
    let Some(class) = AnyClass::get(c"CAKeyframeAnimation") else { return; };
    let animation: Retained<AnyObject> = msg_send![class,
        animationWithKeyPath: &*NSString::from_str("transform.translation.y")];
    let values = NSArray::from_retained_slice(&[
        NSNumber::new_f64(0.), NSNumber::new_f64(1.5), NSNumber::new_f64(0.),
    ]);
    let times = NSArray::from_retained_slice(&[
        NSNumber::new_f64(0.), NSNumber::new_f64(0.45), NSNumber::new_f64(1.),
    ]);
    let _: () = msg_send![&animation, setValues: &*values];
    let _: () = msg_send![&animation, setKeyTimes: &*times];
    let _: () = msg_send![&animation, setCalculationMode: &*NSString::from_str("cubic")];
    if let Some(timing_class) = AnyClass::get(c"CAMediaTimingFunction") {
        let ease: Retained<AnyObject> = msg_send![timing_class,
            functionWithName: &*NSString::from_str("easeInEaseOut")];
        let timings = NSArray::from_retained_slice(&[ease.clone(), ease]);
        let _: () = msg_send![&animation, setTimingFunctions: &*timings];
    }
    let _: () = msg_send![&animation, setDuration: 0.46f64];
    let l = layer(view);
    let _: () = msg_send![&l, addAnimation: &*animation
        forKey: &*NSString::from_str("workspace-selection-float")];
}

/// 글래스 리프트 선택 행의 글자색(본문 잉크)과 아이콘색(인디고).
pub fn soft_selection_ink() -> Retained<NSColor> {
    color(0.137, 0.145, 0.239, 1.)
}
pub fn soft_selection_icon() -> Retained<NSColor> {
    indigo()
}

/// 선택된 행 왼쪽의 세로 강조 막대.
pub unsafe fn accent_bar(frame: NSRect) -> Retained<NSView> {
    let v = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&v);
    let ink = color(0.42, 0.38, 0.98, 1.);
    let _: () = msg_send![&l, setBackgroundColor: &*ink.CGColor()];
    let _: () = msg_send![&l, setCornerRadius: frame.size.width / 2.];
    let _: () = msg_send![&l, setShadowColor: &*ink.CGColor()];
    let _: () = msg_send![&l, setShadowOpacity: 0.55f32];
    let _: () = msg_send![&l, setShadowRadius: 4.0f64];
    let _: () = msg_send![&l, setShadowOffset: NSSize::new(0., 0.)];
    let _: () = msg_send![&l, setMasksToBounds: false];
    let _: () = msg_send![&v, setAccessibilityElement: false];
    v
}

/// 목록 행처럼 판 없이 쓰는 버튼에 아이콘만 붙인다.
pub unsafe fn set_icon(button: &NSButton, icon_name: Option<&str>, size: f64) {
    if let Some(image) = icon_name.and_then(|n| icon(n, size)) {
        button.setImage(Some(&image));
        button.setImageHugsTitle(true);
        button.setImagePosition(if button.title().length() == 0 {
            NSCellImagePosition::ImageOnly
        } else {
            NSCellImagePosition::ImageLeading
        });
        if button.contentTintColor().is_none() {
            button.setContentTintColor(Some(&indigo()));
        }
    }
}

pub unsafe fn apply_window(window: &NSWindow) {
    window.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
    window.setTitlebarAppearsTransparent(true);
    window.setTitleVisibility(NSWindowTitleVisibility::Hidden);
    window.setMovableByWindowBackground(false);
    window.setBackgroundColor(Some(&color(0.925, 0.94, 0.985, 1.)));
    window.setHasShadow(true);
}

/// 패널 유리 위에 얹는 반투명 흰 조각. 탭 트랙·선택 알약·입력창·알약 버튼이 함께 쓴다.
unsafe fn frosted(view: &NSView, alpha: f64, radius: f64) {
    let l = layer(view);
    let fill = color(1., 1., 1., alpha);
    let _: () = msg_send![&l, setBackgroundColor: &*fill.CGColor()];
    let _: () = msg_send![&l, setCornerRadius: radius];
    let _: () = msg_send![&l, setBorderWidth: 0.0f64];
    let _: () = msg_send![&l, setShadowOpacity: 0.0f32];
}
/// 조각 아래로 떨어지는 옅은 남색 그림자.
unsafe fn lift(view: &NSView, opacity: f32, radius: f64, drop: f64) {
    let l = layer(view);
    let ink = color(0.157, 0.173, 0.353, 1.);
    let _: () = msg_send![&l, setShadowColor: &*ink.CGColor()];
    let _: () = msg_send![&l, setShadowOpacity: opacity];
    let _: () = msg_send![&l, setShadowRadius: radius];
    let _: () = msg_send![&l, setShadowOffset: NSSize::new(0., -drop)];
    let _: () = msg_send![&l, setMasksToBounds: false];
}

/// 두 탭을 묶는 반투명 알약 트랙.
pub unsafe fn chat_tab_group(frame: NSRect) -> (Retained<NSView>,Retained<NSView>) {
    let group=NSView::initWithFrame(NSView::alloc(mtm()),frame);
    frosted(&group, 0.30, 20.);
    (group.clone(),group)
}
/// 선택된 탭 아래의 흰 유리 알약.
pub unsafe fn chat_tab_selection(frame:NSRect)->Retained<NSView>{
    let selection=NSView::initWithFrame(NSView::alloc(mtm()),frame);
    frosted(&selection, 0.74, 16.);
    lift(&selection, 0.12, 4., 2.);
    let _:()=msg_send![&selection,setAccessibilityElement:false];
    selection
}
pub unsafe fn chat_tab(button: &NSButton, selected: bool, index: usize) {
    let _: () = msg_send![button, setAccessibilityLabel:&*button.title()];
    glass_button(button,if selected {Glass::Selected}else{Glass::Clear},Some(if index==0 {"folder"}else{"sparkle"}),14.);
    let ink=if selected {color(0.114,0.114,0.169,1.)}else{color(0.235,0.235,0.322,0.64)};
    let font = crate::font::weighted(12.5, if selected {0.3} else {0.18});
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &button.title());
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, &ink, range);
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    button.setAttributedTitle(&title);
    // 선택된 탭은 아이콘만 인디고로 강조한다.
    let tint = if selected {indigo()} else {ink};
    button.setContentTintColor(Some(&tint));
    button.setState(if selected {NSControlStateValueOn}else{NSControlStateValueOff});
    let _:()=msg_send![button,setAccessibilityValue:&*NSString::from_str(if selected {"선택됨"}else{""})];
}

/// 유리 캡슐 입력창: 테두리 없는 반투명 흰 바탕, 아래로 옅은 그림자.
pub unsafe fn chat_composer(frame:NSRect)->Retained<NSView>{
    let shell = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    frosted(&shell, 0.50, 24.);
    let l = layer(&shell);
    let _: () = msg_send![&l, setCornerCurve: &*NSString::from_str("continuous")];
    lift(&shell, 0.22, 7., 6.);
    let _: () = msg_send![&shell, setAccessibilityElement: false];
    shell
}

/// Transparent selector host: a 28pt hit area with a compact 22pt hover fill.
#[derive(Default)]
pub struct ChatChipState { hover: std::cell::Cell<bool> }
objc2::define_class!(
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyChatSelectorHost"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ChatChipState]
    pub struct ChatChip;
    impl ChatChip {
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
        fn entered(&self, _event: &NSEvent) { self.ivars().hover.set(true); self.setNeedsDisplay(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { self.ivars().hover.set(false); self.setNeedsDisplay(true); }
        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { self.ivars().hover.set(false); self.setNeedsDisplay(true); }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) { unsafe {
            if self.ivars().hover.get() && self.subviews().iter().any(|view| {
                let popup: bool = msg_send![&*view, isKindOfClass: NSPopUpButton::class()];
                popup && { let enabled: bool = msg_send![&*view, isEnabled]; enabled }
            }) {
                color(0.431, 0.420, 0.616, 0.07).setFill();
                let b = self.bounds();
                let fill = NSRect::new(NSPoint::new(0., (b.size.height - CHAT_CONTROL_FACE_SIZE) / 2.), NSSize::new(b.size.width, CHAT_CONTROL_FACE_SIZE));
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(fill, 6., 6.).fill();
            }
        } }
        #[unsafe(method(mouseDown:))]
        fn down(&self, _event: &NSEvent) { unsafe {
            // The status dot and padding are part of the same clickable region.
            for view in self.subviews().iter() {
                let popup: bool = msg_send![&*view, isKindOfClass: NSPopUpButton::class()];
                if popup { let _: () = msg_send![&*view, performClick: self]; break; }
            }
        } }
    }
);
use objc2::DefinedClass;
pub unsafe fn chat_chip(frame: NSRect) -> Retained<NSView> {
    let allocated = ChatChip::alloc(mtm()).set_ivars(ChatChipState::default());
    let chip: Retained<ChatChip> = msg_send![super(allocated), initWithFrame: frame];
    let _: () = msg_send![&chip, setAccessibilityElement: false];
    chip.into_super()
}

pub const CHAT_CONTROL_FACE_SIZE: f64 = 22.;
pub const CHAT_CONTROL_ICON_SIZE: f64 = 8.;

/// 칩 오른쪽의 아래 화살표. 팝업 뒤에 깔려 눌림은 팝업이 받는다.
pub unsafe fn chat_chevron() -> Retained<NSView> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("chevron.down"), None)
        .and_then(|image| {
            let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(CHAT_CONTROL_ICON_SIZE, 0.3);
            image.imageWithSymbolConfiguration(&config)
        });
    let view = NSImageView::initWithFrame(NSImageView::alloc(mtm()), NSRect::new(NSPoint::new(0., 0.), NSSize::new(10., 10.)));
    view.setImage(image.as_deref());
    view.setContentTintColor(Some(&color(0.235, 0.235, 0.322, 0.6)));
    let _: () = msg_send![&view, setAccessibilityElement: false];
    view.into_super().into_super()
}

/// 모델 칩 앞의 상태 점. 초록은 연결과 모델 목록이 확인된 상태, 회색은 미확인.
pub unsafe fn chat_status_dot(frame: NSRect) -> Retained<NSView> {
    let dot = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let l = layer(&dot);
    let _: () = msg_send![&l, setCornerRadius: frame.size.width / 2.];
    let _: () = msg_send![&dot, setAccessibilityElement: false];
    chat_status_dot_state(&dot, false);
    dot
}
pub unsafe fn chat_status_dot_state(dot: &NSView, available: bool) {
    let l = layer(dot);
    let fill = if available { color(0.204, 0.78, 0.349, 1.) } else { color(0.64, 0.65, 0.72, 1.) };
    let _: () = msg_send![&l, setBackgroundColor: &*fill.CGColor()];
}

/// 전송·정지 원형 버튼. 보낼 수 없을 때는 회색으로 꺼진다.
pub unsafe fn chat_send(button: &NSButton, enabled: bool) {
    button.setBordered(false);
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    button.setContentTintColor(Some(&NSColor::whiteColor()));
    let l = layer(button);
    let fill = if enabled { color(0.373, 0.357, 0.933, 1.) } else { color(0.431, 0.439, 0.549, 0.24) };
    let glow = color(0.31, 0.275, 0.898, 1.);
    let _: () = msg_send![&l, setBackgroundColor: &*fill.CGColor()];
    let _: () = msg_send![&l, setCornerRadius: CHAT_CONTROL_FACE_SIZE / 2.];
    let _: () = msg_send![&l, setShadowColor: &*glow.CGColor()];
    let _: () = msg_send![&l, setShadowOpacity: if enabled { 0.45f32 } else { 0.0f32 }];
    let _: () = msg_send![&l, setShadowRadius: 5.0f64];
    let _: () = msg_send![&l, setShadowOffset: NSSize::new(0., -3.)];
    let _: () = msg_send![&l, setMasksToBounds: false];
}

/// 조작 줄 맨 왼쪽의 이미지 첨부(+) 버튼. 전송 버튼과 같은 22pt 원.
pub unsafe fn chat_attach_button(button: &NSButton) {
    button.setBordered(false);
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("plus"), None) {
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(CHAT_CONTROL_ICON_SIZE + 3., 0.3);
        button.setImage(image.imageWithSymbolConfiguration(&config).as_deref());
    }
    button.setContentTintColor(Some(&color(0.235, 0.235, 0.322, 0.75)));
    let l = layer(button);
    let _: () = msg_send![&l, setBackgroundColor: &*color(0.431, 0.420, 0.616, 0.09).CGColor()];
    let _: () = msg_send![&l, setCornerRadius: CHAT_CONTROL_FACE_SIZE / 2.];
    button.setToolTip(Some(&NSString::from_str("이미지 첨부 · 붙여넣기(⌘V)나 끌어놓기도 돼요")));
    let _: () = msg_send![button, setAccessibilityLabel: &*NSString::from_str("이미지 첨부")];
}

/// 아이콘이 붙은 반투명 알약 버튼(새 대화).
pub unsafe fn chat_pill_button(button: &NSButton, icon_name: &str) {
    button.setBordered(false);
    frosted(button, 0.42, 14.);
    lift(button, 0.08, 2., 1.);
    let ink = color(0.247, 0.259, 0.408, 1.);
    let font = crate::font::regular(11.5);
    button.setFont(Some(&font));
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &button.title());
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, &ink, range);
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    button.setAttributedTitle(&title);
    button.setContentTintColor(Some(&indigo()));
    set_icon(button, Some(icon_name), 12.);
}

/// 새 답변으로 이동하는 라벤더 알약 버튼.
pub unsafe fn chat_new_answer_button(button: &NSButton) {
    button.setBordered(false);
    let l = layer(button);
    let _: () = msg_send![&l, setBackgroundColor: &*color(0.89, 0.86, 0.98, 1.).CGColor()];
    let _: () = msg_send![&l, setCornerRadius: 13.0f64];
    let font = crate::font::regular(11.5);
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &button.title());
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, &color(0.30, 0.23, 0.59, 1.), range);
    title.addAttribute_value_range(NSFontAttributeName, &font, range);
    button.setAttributedTitle(&title);
}

/// Current document wash shared by saved documents and drafts.
pub fn document_selection_color() -> Retained<NSColor> {
    color(0.42, 0.38, 0.98, 0.09)
}
pub unsafe fn document_selection(frame: NSRect) -> Retained<NSView> {
    let view = NSView::initWithFrame(NSView::alloc(mtm()), frame);
    let layer = layer(&view);
    let _: () = msg_send![&layer, setBackgroundColor: &*document_selection_color().CGColor()];
    let _: () = msg_send![&layer, setCornerRadius: 10.0f64];
    let _: () = msg_send![&view, setAccessibilityElement: false];
    view
}


// Glass bezels ignore showsBorderOnlyWhileMouseInside on macOS 26.
// Explicitly toggle the native bezel so its shadow only exists during hover.
objc2::define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologyHoverGlassButton"]
    #[thread_kind = MainThreadOnly]
    pub struct HoverGlassButton;
    impl HoverGlassButton {
        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking(&self) { unsafe {
            let _: () = msg_send![super(self), updateTrackingAreas];
            for area in self.trackingAreas().iter() { self.removeTrackingArea(&area); }
            let area = NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(), self.bounds(),
                NSTrackingAreaOptions::MouseEnteredAndExited | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect, Some(self), None);
            self.addTrackingArea(&area);
        } }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _event: &NSEvent) { unsafe {
            self.setBordered(self.isEnabled());
        } }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _event: &NSEvent) { unsafe { self.setBordered(false); } }
        #[unsafe(method(viewDidHide))]
        fn hidden(&self) { unsafe { self.setBordered(false); } }
    }
);
pub unsafe fn hover_glass_button(frame: NSRect) -> Retained<NSButton> {
    let button: Retained<HoverGlassButton> = msg_send![HoverGlassButton::alloc(mtm()), initWithFrame: frame];
    button.setButtonType(NSButtonType::MomentaryPushIn);
    Retained::into_super(button)
}
