//! 학습도우미 입력창 위의 노트 칩과 미리보기.
//! 칩은 노트에서 블록을 골랐을 때(제안)와 ⌘L로 붙였을 때(첨부)만 보인다.
//! 칩을 누르면 높이가 고정된 미리보기가 입력창 밑에서 올라오고,
//! 칩은 제자리에서 그 위에 겹친다. 칩을 눌러도 붙지는 않는다.
//! 미리보기가 열려 있는 동안 입력창의 위 모서리를 각지게 해 둘이 한 판으로 이어진다.
use super::*;

pub const CHIP_TAG: i64 = 19;
pub const CHIP_CLOSE_TAG: i64 = 20;
pub const PREVIEW_CLOSE_TAG: i64 = 21;
/// 칩 줄 높이(칩 + 아래 여백).
pub const CHIP_ROW: f64 = 30.;
const CHIP_HEIGHT: f64 = 24.;
/// 미리보기 위쪽, 닫기 버튼이 앉는 줄.
const PREVIEW_HEADER: f64 = 30.;

/// 미리보기 높이. 자리가 모자라면 그만큼만 쓴다.
const PREVIEW_HEIGHT: f64 = 300.;
/// 입력창 밑에서 올라오고 들어가는 시간.
const SLIDE_SECONDS: f64 = 0.22;

/// 편집기에 열려 있는 노트. 메인 화면이 틱마다 넘긴다.
pub struct NoteSource {
    pub title: String,
    /// 다시 열 때 쓰는 문서 `{id, kind}`. 저장 전 새 노트는 Null이다.
    pub doc: Value,
    pub text: Retained<NSTextView>,
}

impl NoteSource {
    /// 칩에 쓰는 범위 이름. 고른 블록이 없으면 `None`. 레이아웃을 재지 않는다.
    pub unsafe fn label(&self) -> Option<String> {
        match editor::selected_block_count(&self.text) {
            0 => None,
            n => Some(format!("선택한 블록 {n}개")),
        }
    }
    /// 지금 붙이면 보낼 내용. 레이아웃을 재므로 누를 때만 부른다.
    pub unsafe fn context(&self) -> Result<Value, String> {
        let excerpt = editor::note_excerpt(&self.text, &self.title)?;
        Ok(json!({
            "title": self.title,
            "label": excerpt.label,
            "text": excerpt.text,
            "doc": self.doc,
            "blocks": excerpt.blocks.iter().map(|r| json!([r.start, r.end])).collect::<Vec<_>>(),
            "heads": excerpt.heads,
        }))
    }
}

pub struct NoteChip {
    pub chip: Retained<NSButton>,
    close: Retained<NSButton>,
    /// 입력창 윗선에서 잘리는 틀. 미리보기 판은 이 안에서 오르내린다.
    clip: Retained<NSView>,
    preview: Retained<NSView>,
    preview_close: Retained<NSButton>,
    preview_scroll: Retained<NSScrollView>,
    /// 스크롤 뷰 안에서 마크다운 본문을 싣는 뒤집힌 판.
    preview_page: Retained<NSView>,
    preview_rich: crate::chat_rich_text::RichText,
    /// 미리보기가 이어 붙는 입력창.
    composer: Retained<NSView>,
    /// 미리보기에 그리는 마크다운.
    preview_markdown: String,
    /// 미리보기에 보이는 내용의 열쇠. 바뀌면 다시 채운다.
    shown: Option<String>,
    /// 칩 모양을 마지막으로 그린 상태.
    drawn: Option<(String, bool)>,
    /// 미리보기가 자리 잡을 곳. 같은 자리면 움직이는 중인 판을 건드리지 않는다.
    target: std::cell::Cell<Option<NSRect>>,
    /// 다음 배치 때 아래에서 올라오게 한다.
    slide_in: std::cell::Cell<bool>,
    /// 내려가는 중이면 끝나는 때. 그 뒤에 숨긴다.
    closing: Option<Instant>,
}

/// SF Symbol 그대로. `theme::icon`은 앱 고유 이름만 받는다.
unsafe fn symbol(name: &str, size: f64) -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(name), None)?;
    let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(size, 0.0);
    image.imageWithSymbolConfiguration(&config)
}

fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// 높이 0으로 접혀 입력창 윗선에 숨은 틀.
fn folded(target: NSRect) -> NSRect {
    rect(target.origin.x, target.origin.y, target.size.width, 0.)
}

/// 미리보기 틀을 `frame`으로 부드럽게 펴거나 접는다.
unsafe fn animate(view: &NSView, frame: NSRect) {
    NSAnimationContext::beginGrouping();
    let context = NSAnimationContext::currentContext();
    context.setDuration(SLIDE_SECONDS);
    let proxy: Retained<NSView> = msg_send![view, animator];
    proxy.setFrame(frame);
    NSAnimationContext::endGrouping();
}

unsafe fn chip_title(button: &NSButton, text: &str, attached: bool) {
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &NSString::from_str(text));
    let range = NSRange::new(0, title.length());
    let ink = if attached { suggest_color(0.27, 0.24, 0.72, 1.) } else { suggest_color(0.33, 0.34, 0.45, 0.9) };
    title.addAttribute_value_range(NSForegroundColorAttributeName, &ink, range);
    title.addAttribute_value_range(NSFontAttributeName, &font::regular(11.), range);
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
    // 칩 왼쪽 여백은 들여쓰기로 준다. 오른쪽은 칩 폭에서 따로 남긴다.
    paragraph.setAlignment(NSTextAlignment::Left);
    paragraph.setFirstLineHeadIndent(10.);
    paragraph.setHeadIndent(10.);
    title.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
    button.setAttributedTitle(&title);
    button.setContentTintColor(Some(&ink));
}

unsafe fn chip_face(button: &NSButton, attached: bool) {
    button.setWantsLayer(true);
    let Some(l) = button.layer() else { return };
    let (fill, border) = if attached {
        (suggest_color(0.37, 0.36, 0.93, 0.14), suggest_color(0.37, 0.36, 0.93, 0.38))
    } else {
        (suggest_color(1., 1., 1., 0.55), suggest_color(0.43, 0.42, 0.62, 0.22))
    };
    let _: () = msg_send![&l, setBackgroundColor: &*fill.CGColor()];
    let _: () = msg_send![&l, setBorderColor: &*border.CGColor()];
    let _: () = msg_send![&l, setBorderWidth: 0.75f64];
    let _: () = msg_send![&l, setCornerRadius: CHIP_HEIGHT / 2.];
}

/// CACornerMask 위쪽 두 모서리. 레이어의 Y축 기준이다.
unsafe fn top_corners(l: &AnyObject) -> usize {
    let flipped: bool = msg_send![l, isGeometryFlipped];
    if flipped { 0b0011 } else { 0b1100 }
}

/// 미리보기가 열려 있으면 입력창의 위 모서리를 각지게 해 미리보기 판과 잇는다.
unsafe fn join_composer(composer: &NSView, joined: bool) {
    let Some(l) = composer.layer() else { return };
    let corners: usize = if joined { 0b1111 & !top_corners(&l) } else { 0b1111 };
    let _: () = msg_send![&l, setMaskedCorners: corners];
}

impl NoteChip {
    /// 미리보기는 `composer`(입력창) 뒤에 두어 입력창 뒤에서 올라오게 한다.
    pub unsafe fn new(parent: &NSView, target: &AnyObject, composer: &NSView) -> Self {
        let button = |tag: i64| {
            let b = NSButton::buttonWithTitle_target_action(&NSString::from_str(""), Some(target), Some(sel!(chatAction:)), mtm());
            b.setTag(tag as isize);
            b.setBordered(false);
            b
        };
        // 미리보기 틀만 뒤의 대화를 흐리게 비춘다. 틀 아래쪽 대화에 가우시안 흐림을 걸고
        // 거의 불투명한 연보라 면을 얹어, 흰 대화 화면과 다른 판으로 읽히게 한다.
        // 틀의 아래 끝은 입력창 윗선이라 위 모서리만 둥글게 한다. 아래 테두리는 입력창에 가린다.
        let clip = view(rect(0., 0., 300., 200.));
        clip.setWantsLayer(true);
        clip.setLayerUsesCoreImageFilters(true);
        if let Some(class) = objc2::runtime::AnyClass::get(c"CIFilter") {
            let blur: Option<Retained<AnyObject>> = msg_send![class, filterWithName: &*NSString::from_str("CIGaussianBlur")];
            if let Some(blur) = blur {
                let _: () = msg_send![&blur, setDefaults];
                let _: () = msg_send![&blur, setValue: &*NSNumber::new_f64(7.), forKey: &*NSString::from_str("inputRadius")];
                let filters = NSArray::from_slice(&[&*blur]);
                let _: () = msg_send![&clip, setBackgroundFilters: &*filters];
            }
        }
        if let Some(l) = clip.layer() {
            let _: () = msg_send![&l, setBackgroundColor: &*suggest_color(0.94, 0.94, 0.99, 0.94).CGColor()];
            let _: () = msg_send![&l, setBorderColor: &*suggest_color(0.43, 0.42, 0.62, 0.28).CGColor()];
            let _: () = msg_send![&l, setBorderWidth: 0.75f64];
            let _: () = msg_send![&l, setCornerRadius: 16.0f64];
            // 위쪽 두 모서리만 둥글게 한다. 아래는 입력창 윗선과 이어진다.
            let _: () = msg_send![&l, setMaskedCorners: top_corners(&l)];
            let _: () = msg_send![&l, setMasksToBounds: true];
        }
        clip.setHidden(true);
        // 내용은 틀의 위쪽에 붙어 있어, 틀이 자라면 입력창 밑에서 올라오는 것처럼 보인다.
        let preview = view(rect(0., 0., 300., 200.));
        preview.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
        clip.addSubview(&preview);
        let preview_close = button(PREVIEW_CLOSE_TAG);
        preview_close.setImage(symbol("xmark", 10.).as_deref());
        preview_close.setImagePosition(NSCellImagePosition::ImageOnly);
        preview_close.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        preview_close.setToolTip(Some(&NSString::from_str("미리보기 닫기")));
        preview_close.setAccessibilityLabel(Some(&NSString::from_str("미리보기 닫기")));
        preview.addSubview(&preview_close);
        let preview_scroll = crate::scrollbar::vertical(rect(0., 0., 300., 160.));
        preview_scroll.setDrawsBackground(false);
        preview_scroll.setBorderType(NSBorderType::NoBorder);
        let preview_page = crate::chat_messages::flipped();
        let preview_rich = crate::chat_rich_text::RichText::new();
        preview_page.addSubview(&preview_rich.view);
        preview_scroll.setDocumentView(Some(&preview_page));
        preview.addSubview(&preview_scroll);
        let _: () = msg_send![&preview, setAccessibilityLabel: &*NSString::from_str("AI에게 보낼 노트 미리보기")];
        parent.addSubview_positioned_relativeTo(&clip, NSWindowOrderingMode::Below, Some(composer));
        // 칩은 미리보기보다 나중에 넣어 미리보기 위에 겹쳐 보이게 한다.
        let chip = button(CHIP_TAG);
        chip.setToolTip(Some(&NSString::from_str("원문 보기")));
        chip.setHidden(true);
        parent.addSubview(&chip);
        let close = button(CHIP_CLOSE_TAG);
        close.setImage(symbol("xmark", 8.).as_deref());
        close.setImagePosition(NSCellImagePosition::ImageOnly);
        close.setContentTintColor(Some(&suggest_color(0.27, 0.24, 0.72, 0.9)));
        close.setToolTip(Some(&NSString::from_str("붙인 노트 빼기")));
        close.setAccessibilityLabel(Some(&NSString::from_str("붙인 노트 빼기")));
        close.setHidden(true);
        parent.addSubview(&close);

        Self { chip, close, clip, preview, preview_close, preview_scroll, preview_page, preview_rich, composer: Retained::retain(composer as *const NSView as *mut NSView).expect("live composer"), preview_markdown: String::new(), shown: None, drawn: None, target: std::cell::Cell::new(None), slide_in: std::cell::Cell::new(false), closing: None }
    }

    pub fn preview_open(&self) -> bool { self.shown.is_some() }
    pub fn shown(&self) -> Option<&str> { self.shown.as_deref() }

    /// 칩 글자와 모양. `None`이면 칩을 숨긴다. 바뀌었으면 `true`.
    pub unsafe fn show(&mut self, chip: Option<(String, bool)>) -> bool {
        if self.drawn == chip { return false; }
        match &chip {
            Some((text, attached)) => {
                let shown = if *attached { text.clone() } else { format!("{text}  ⌘L") };
                chip_title(&self.chip, &shown, *attached);
                chip_face(&self.chip, *attached);
                self.chip.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                    "{} {text} · 눌러서 원문 보기", if *attached { "붙인 노트" } else { "보고 있는 노트" }))));
                // 칩은 그 자체로만 떠 있고, 대화·썸네일·미리보기보다 늘 앞에 놓인다.
                if let Some(parent) = self.chip.superview() {
                    parent.addSubview_positioned_relativeTo(&self.chip, NSWindowOrderingMode::Above, None);
                    parent.addSubview_positioned_relativeTo(&self.close, NSWindowOrderingMode::Above, None);
                }
                self.chip.setHidden(false);
                self.close.setHidden(!attached);
            }
            None => {
                self.chip.setHidden(true);
                self.close.setHidden(true);
            }
        }
        self.drawn = chip;
        true
    }

    /// 미리보기를 `key`의 내용으로 연다. 이미 같은 내용이면 그대로 둔다.
    pub unsafe fn open(&mut self, key: &str, body: &str) {
        if self.shown.as_deref() == Some(key) { return; }
        // 보낼 원문을 마크다운 서식으로 보여 준다. 실제로 가는 글은 그대로다.
        self.preview_markdown = editor::note_context::display_markdown(body);
        self.preview_rich.update(&self.preview_markdown, true);
        self.preview_page.scrollPoint(NSPoint::new(0., 0.));
        if self.shown.is_none() {
            // 닫혀 있었거나 내려가는 중이면 아래에서 다시 올라온다.
            self.closing = None;
            join_composer(&self.composer, true);
            self.slide_in.set(true);
            self.target.set(None);
            self.clip.setHidden(false);
        }
        self.shown = Some(key.to_owned());
    }

    /// 웹 본문은 늦게 그려지고 높이를 나중에 알려 준다. 높이가 바뀌면 `true`.
    pub unsafe fn tick(&mut self) -> bool {
        if self.closing.is_some_and(|at| Instant::now() >= at) {
            self.closing = None;
            if self.shown.is_none() { self.clip.setHidden(true); join_composer(&self.composer, false); }
        }
        self.shown.is_some() && self.preview_rich.update(&self.preview_markdown, true)
    }

    /// 입력창 밑으로 들어가며 사라진다. 동작 줄이기에서는 바로 숨긴다.
    pub unsafe fn close_preview(&mut self) {
        if self.shown.take().is_none() { return; }
        self.slide_in.set(false);
        let Some(target) = self.target.get().filter(|_| !reduce_motion()) else {
            self.clip.setHidden(true);
            join_composer(&self.composer, false);
            return;
        };
        animate(&self.clip, folded(target));
        self.target.set(None);
        self.closing = Some(Instant::now() + Duration::from_secs_f64(SLIDE_SECONDS));
    }

    /// 칩은 `bottom`에서 시작하고, 미리보기는 그 위 `top`까지 쓸 수 있다.
    /// 미리보기 아래 끝은 입력창 윗선(`bottom - 4`)이다.
    pub unsafe fn frame(&self, width: f64, bottom: f64, top: f64) {
        let max_chip = (width - 32. - 28.).max(60.);
        // 글자 + 오른쪽 여백, 붙인 칩은 ×가 앉을 자리까지.
        let tail = if self.close.isHidden() { 12. } else { 28. };
        let chip_width = (self.chip.intrinsicContentSize().width.ceil() + tail).min(max_chip);
        self.chip.setFrame(rect(16., bottom, chip_width, CHIP_HEIGHT));
        self.close.setFrame(rect(16. + chip_width - 22., bottom + 4., 16., 16.));
        // 미리보기는 입력창 바로 위에 높이를 고정해 두고, 칩은 그 아래쪽에 떠서 겹친다.
        // 본문은 판 바닥까지 채우고, 마지막 줄은 스크롤하면 칩 위로 올라오게 칩 줄만큼 여백을 둔다.
        let preview_width = (width - 24.).max(120.);
        let floor = bottom - 4.;
        let height = PREVIEW_HEIGHT.min((top - floor).max(120.));
        let inner = preview_width - 28.;
        let used = self.preview_rich.height.ceil() + 12. + CHIP_ROW;
        let body = height - PREVIEW_HEADER;
        self.preview_close.setFrame(rect(preview_width - 32., height - 26., 22., 22.));
        self.preview_scroll.setFrame(rect(0., 0., preview_width, body));
        self.preview_page.setFrame(rect(0., 0., preview_width, used.max(body)));
        self.preview_rich.view.setFrame(rect(14., 2., inner, self.preview_rich.height.max(24.)));
        if self.shown.is_none() { return; }
        let target = rect(12., floor, preview_width, height);
        if self.slide_in.replace(false) && !reduce_motion() {
            // 다 편 틀에 내용을 맞춘 뒤 접고, 다시 펴며 올라오게 한다.
            self.clip.setFrame(target);
            self.preview.setFrame(rect(0., 0., preview_width, height));
            self.clip.setFrame(folded(target));
            animate(&self.clip, target);
        } else if self.target.get() != Some(target) {
            self.clip.setFrame(target);
            self.preview.setFrame(rect(0., 0., preview_width, height));
        }
        self.target.set(Some(target));
    }

    pub fn visible(&self) -> bool { !self.chip.isHidden() }
}
