//! Selectable message bubbles; streaming only updates the affected text view.
use super::*;
use objc2::DefinedClass;
use std::collections::HashMap;
define_class!(
 #[unsafe(super(NSView))] #[thread_kind=MainThreadOnly] struct Flipped;
 unsafe impl NSObjectProtocol for Flipped {}
 impl Flipped {#[unsafe(method(isFlipped))] fn flipped(&self)->bool {true}}
);
pub(crate) unsafe fn flipped() -> Retained<NSView> {
    let v: Retained<Flipped> = msg_send![Flipped::alloc(mtm()),initWithFrame:rect(0.,0.,300.,300.)];
    Retained::into_super(v)
}
// 반투명 말풍선 바탕. 꼬리 쪽 모서리 하나만 작게 깎아 말한 쪽을 가리킨다.
// 내 질문은 오른쪽 아래, 답변은 왼쪽 아래가 꼬리다.
const BUBBLE_RADIUS: f64 = 20.;
const BUBBLE_TAIL: f64 = 6.;
define_class!(
 #[unsafe(super(NSView))] #[thread_kind=MainThreadOnly] #[ivars = Cell<bool>] struct BubbleView;
 unsafe impl NSObjectProtocol for BubbleView {}
 impl BubbleView {#[unsafe(method(drawRect:))] fn draw(&self,_dirty:NSRect){
   let user=self.ivars().get();
   let size=self.bounds().size;
   let (w,h)=(size.width,size.height);
   let corner=|r:f64|r.min(w/2.).min(h/2.);
   let (round,tail)=(corner(BUBBLE_RADIUS),corner(BUBBLE_TAIL));
   let (bottom_left,bottom_right)=if user {(round,tail)} else {(tail,round)};
   let at=|x:f64,y:f64|NSPoint::new(x,y);
   let path=NSBezierPath::bezierPath();
   path.moveToPoint(at(bottom_left,0.));
   path.appendBezierPathWithArcFromPoint_toPoint_radius(at(w,0.),at(w,h),bottom_right);
   path.appendBezierPathWithArcFromPoint_toPoint_radius(at(w,h),at(0.,h),round);
   path.appendBezierPathWithArcFromPoint_toPoint_radius(at(0.,h),at(0.,0.),round);
   path.appendBezierPathWithArcFromPoint_toPoint_radius(at(0.,0.),at(w,0.),bottom_left);
   path.closePath();
   let fill=if user {(0.69,0.776,1.,0.46)} else {(1.,1.,1.,0.46)};
   NSColor::colorWithSRGBRed_green_blue_alpha(fill.0,fill.1,fill.2,fill.3).setFill();
   path.fill();
 }}
);
unsafe fn bubble_view(user: bool) -> Retained<NSView> {
    let allocated = BubbleView::alloc(mtm()).set_ivars(Cell::new(user));
    let v: Retained<BubbleView> = msg_send![super(allocated),initWithFrame:rect(0.,0.,60.,36.)];
    let _: () = msg_send![&v,setAccessibilityElement:false];
    Retained::into_super(v)
}
/// 답 아래 정리 버튼의 태그가 시작하는 값. 복사(1000대)·다시 시도(2000대)·첨부 닫기(3000대)와 겹치지 않는다.
pub const NOTE_TAG: isize = 6000;
/// 질문 위 대화 문맥 이름표의 태그가 시작하는 값. 정리 버튼(6000~8999) 다음이다.
pub const CONTEXT_TAG: isize = 9000;
const CONTEXT_HEIGHT: f64 = 20.;
/// 붙인 노트를 알려 주는 이름표. 누르면 노트의 그 블록으로 간다.
unsafe fn context_title(button: &NSButton, text: &str) {
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &NSString::from_str(text));
    let range = NSRange::new(0, title.length());
    let indigo = NSColor::colorWithSRGBRed_green_blue_alpha(0.31, 0.275, 0.75, 1.);
    title.addAttribute_value_range(NSForegroundColorAttributeName, &indigo, range);
    title.addAttribute_value_range(NSFontAttributeName, &font::regular(10.5), range);
    button.setAttributedTitle(&title);
}
/// 말풍선 아래 작은 회색 글자 버튼(복사·다시 시도·정리).
unsafe fn footer_title(button: &NSButton) {
    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &button.title());
    let range = NSRange::new(0, title.length());
    title.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), range);
    title.addAttribute_value_range(NSFontAttributeName, &font::regular(10.), range);
    button.setAttributedTitle(&title);
}
/// Layer animations keep the waiting dots smooth without another UI timer.
struct WaitingDots {
    view: Retained<NSView>,
    dots: Vec<Retained<NSView>>,
    animating: Cell<bool>,
}
impl WaitingDots {
    unsafe fn new() -> Self {
        let view = super::view(rect(0., 0., 32., 18.));
        let _: () = msg_send![&view, setAccessibilityElement:true];
        let _: () = msg_send![&view, setAccessibilityRole:&*NSString::from_str("AXStaticText")];
        let _: () = msg_send![&view, setAccessibilityLabel:&*NSString::from_str("답변 대기 중")];
        let mut dots = Vec::new();
        for i in 0..3 {
            let dot = super::view(rect(i as f64 * 10. + 3., 6., 5., 5.));
            dot.setWantsLayer(true);
            if let Some(layer) = dot.layer() {
                let color = NSColor::secondaryLabelColor().CGColor();
                let _: () = msg_send![&layer, setBackgroundColor:&*color];
                let _: () = msg_send![&layer, setCornerRadius:2.5f64];
            }
            view.addSubview(&dot);
            dots.push(dot);
        }
        view.setHidden(true);
        Self { view, dots, animating: Cell::new(false) }
    }
    unsafe fn update(&self, waiting: bool) {
        self.view.setHidden(!waiting);
        let animate = waiting && !self.view.isHiddenOrHasHiddenAncestor()
            && self.view.window().is_some_and(|w| w.isVisible() && !w.isMiniaturized())
            && NSApplication::sharedApplication(mtm()).isActive()
            && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        if self.animating.replace(animate) == animate { return; }
        for (i, dot) in self.dots.iter().enumerate() {
            let Some(layer) = dot.layer() else { continue };
            let _: () = msg_send![&layer, removeAllAnimations];
            if animate {
                if let Some(class) = objc2::runtime::AnyClass::get(c"CABasicAnimation") {
                    let animation: Retained<AnyObject> = msg_send![class, animationWithKeyPath:&*NSString::from_str("transform.translation.y")];
                    let _: () = msg_send![&animation, setFromValue:&*NSNumber::new_f64(-1.)];
                    let _: () = msg_send![&animation, setToValue:&*NSNumber::new_f64(3.)];
                    let _: () = msg_send![&animation, setDuration:0.4f64];
                    let _: () = msg_send![&animation, setAutoreverses:true];
                    let _: () = msg_send![&animation, setRepeatCount:f32::INFINITY];
                    let _: () = msg_send![&animation, setTimeOffset:i as f64 * 0.16];
                    let _: () = msg_send![&layer, addAnimation:&*animation forKey:&*NSString::from_str("waiting-dot")];
                }
            }
        }
    }
}
struct Bubble {
    shell: Retained<NSView>,
    background: Retained<NSView>,
    label: Retained<NSTextField>,
    text: Retained<NSTextView>,
    waiting: WaitingDots,
    rich: Option<crate::chat_rich_text::RichText>,
    copy: Option<Retained<NSButton>>,
    retry: Option<Retained<NSButton>>,
    /// 나중에 정리 · 노트로 정리 · 그대로 저장.
    notes: Option<[Retained<NSButton>; 3]>,
    /// 질문에 붙인 노트의 이름표.
    context: Option<Retained<NSButton>>,
    user: bool,
    body: String,
    state: String,
    /// 이 답의 정리 표시와 진행 상태.
    marks: String,
}
pub struct Messages {
    pub scroll: Retained<NSScrollView>,
    document: Retained<NSView>,
    empty: Retained<NSTextField>,
    hint: Retained<NSTextField>,
    bubbles: HashMap<String, Bubble>,
    order: Vec<String>,
    width: f64,
}
impl Messages {
    pub unsafe fn new() -> Self {
        let scroll = crate::scrollbar::vertical(rect(0., 0., 340., 400.));
        scroll.setDrawsBackground(false);
        scroll.setBorderType(NSBorderType::NoBorder);
        let document = flipped();
        scroll.setDocumentView(Some(&document));
        let empty = text_label("무엇이 궁금한가요?", rect(20., 80., 280., 28.), 19., true);
        empty.setAlignment(NSTextAlignment::Center);
        let hint = text_label(
            "어려운 개념부터 작은 질문까지.\n함께 차근차근 풀어봐요.",
            rect(20., 120., 280., 48.),
            12.,
            false,
        );
        hint.setMaximumNumberOfLines(2);
        hint.setAlignment(NSTextAlignment::Center);
        hint.setTextColor(Some(&NSColor::secondaryLabelColor()));
        document.addSubview(&empty);
        document.addSubview(&hint);
        Self {
            scroll,
            document,
            empty,
            hint,
            bubbles: HashMap::new(),
            order: Vec::new(),
            width: 300.,
        }
    }
    pub unsafe fn position(&self) -> NSPoint {
        self.scroll.contentView().bounds().origin
    }
    pub unsafe fn restore(&self, p: NSPoint) {
        self.document.scrollPoint(p);
    }
    pub unsafe fn bottom(&self) {
        self.document.scrollPoint(NSPoint::new(
            0.,
            (self.document.frame().size.height - self.scroll.contentView().bounds().size.height)
                .max(0.),
        ));
    }
    pub unsafe fn clear(&mut self) {
        for b in self.bubbles.values() {
            b.shell.removeFromSuperview();
        }
        self.bubbles.clear();
        self.order.clear();
        self.layout();
    }
    /// 아래 `height`pt(첨부 줄이 겹치는 곳)에서 대화가 투명하게 사라지게 한다. 0이면 해제.
    pub unsafe fn fade_bottom(&self, height: f64) {
        self.scroll.setWantsLayer(true);
        let Some(layer) = self.scroll.layer() else { return };
        let Some(class) = objc2::runtime::AnyClass::get(c"CAGradientLayer").filter(|_| height > 0.) else {
            let _: () = msg_send![&layer, setMask: std::ptr::null::<AnyObject>()];
            return;
        };
        let size = self.scroll.frame().size;
        let mask: Retained<AnyObject> = msg_send![class, layer];
        let _: () = msg_send![&mask, setFrame: rect(0., 0., size.width, size.height)];
        let alpha = |a: f64| NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., a).CGColor();
        let (clear, faint, solid) = (alpha(0.), alpha(0.), alpha(1.));
        let colors: [&AnyObject; 4] = [
            &*(Retained::as_ptr(&clear) as *const AnyObject),
            &*(Retained::as_ptr(&faint) as *const AnyObject),
            &*(Retained::as_ptr(&solid) as *const AnyObject),
            &*(Retained::as_ptr(&solid) as *const AnyObject),
        ];
        let _: () = msg_send![&mask, setColors: &*NSArray::from_slice(&colors)];
        // 썸네일 아래쪽은 거의 비우고 위 가장자리에서 서서히 살아난다.
        let h = size.height.max(1.);
        let stops = [0., (height * 0.7 / h).min(1.), ((height + 24.) / h).min(1.), 1.];
        let numbers: Vec<Retained<NSNumber>> = stops.iter().map(|v| NSNumber::new_f64(*v)).collect();
        let _: () = msg_send![&mask, setLocations: &*NSArray::from_retained_slice(&numbers)];
        // 스크롤 뷰 레이어는 뒤집힌 좌표일 수 있으니 실제 방향을 확인해 아래에서 위로 칠한다.
        let flipped: bool = msg_send![&layer, contentsAreFlipped];
        let (from, to) = if flipped { (1., 0.) } else { (0., 1.) };
        let _: () = msg_send![&mask, setStartPoint: NSPoint::new(0.5, from)];
        let _: () = msg_send![&mask, setEndPoint: NSPoint::new(0.5, to)];
        let _: () = msg_send![&layer, setMask: &*mask];
    }
    pub unsafe fn frame(&mut self, f: NSRect) {
        let old = self.position();
        let near = old.y + self.scroll.contentView().bounds().size.height
            >= self.document.frame().size.height - 40.;
        self.scroll.setFrame(f);
        self.width = self.scroll.contentView().bounds().size.width;
        self.layout();
        if near {
            self.bottom();
        } else {
            self.restore(old);
        }
    }
    pub unsafe fn update(&mut self, data: &Value, provider: &str, target: &AnyObject) -> bool {
        let bounds = self.scroll.contentView().bounds();
        let near = bounds.origin.y + bounds.size.height >= self.document.frame().size.height - 40.;
        let mut changed = false;
        let mut order = Vec::new();
        if let Some(turns) = data["turns"].as_array() {
            for (i, t) in turns.iter().enumerate() {
                for user in [true, false] {
                    let key = format!("{}/{}", strv(t, "id"), user);
                    order.push(key.clone());
                    let mut body = strv(t, if user { "user" } else { "answer" });
                    let images = if user { t["images"].as_u64().unwrap_or(0) } else { 0 };
                    if images > 0 {
                        let tags: Vec<_> = (1..=images).map(|n| format!("[Image #{n}]")).collect();
                        body = if body.trim().is_empty() { tags.join(" ") } else { format!("{}\n{body}", tags.join(" ")) };
                    }
                    if !user {
                        if let Some(error) = t["error"].as_str().filter(|e| !e.is_empty()) {
                            body.push_str("\n\n");
                            body.extend(error.chars().take(400));
                        }
                    }
                    let state = if user { "".into() } else { strv(t, "state") };
                    let marks = if user { String::new() } else { format!("{}/{}", strv(t, "mark"), strv(t, "distill")) };
                    if !self.bubbles.contains_key(&key) {
                        // shell은 한 줄 전체(말풍선 + 아래 동작 줄)를 담는 투명 상자다.
                        let shell = flipped();
                        let background = bubble_view(user);
                        shell.addSubview(&background);
                        // 이름표 대신 좌우 정렬로 말한 쪽을 구분하고, 여기에는 답변 상태만 적는다.
                        let label = text_label("", rect(12., 10., 220., 14.), 10., false);
                        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
                        shell.addSubview(&label);
                        let text = NSTextView::initWithFrame(
                            NSTextView::alloc(mtm()),
                            rect(12., 32., 240., 40.),
                        );
                        text.setRichText(false);
                        text.setEditable(false);
                        text.setSelectable(true);
                        text.setDrawsBackground(false);
                        text.setFont(Some(&font::regular(13.)));
                        let paragraph = NSMutableParagraphStyle::new();
                        paragraph.setLineSpacing(3.5);
                        text.setDefaultParagraphStyle(Some(&paragraph));
                        text.setTextContainerInset(NSSize::new(0., 0.));
                        text.setHorizontallyResizable(false);
                        text.setVerticallyResizable(false);
                        if let Some(c) = text.textContainer() {
                            c.setLineFragmentPadding(0.);
                            c.setWidthTracksTextView(false);
                        }
                        shell.addSubview(&text);
                        let rich = (!user).then(|| crate::chat_rich_text::RichText::new());
                        if let Some(rich) = &rich { shell.addSubview(&rich.view); }
                        let waiting = WaitingDots::new();
                        shell.addSubview(&waiting.view);
                        let action = |title: &str, tag: isize| {
                            let b = NSButton::buttonWithTitle_target_action(
                                &NSString::from_str(title),
                                Some(target),
                                Some(sel!(chatAction:)),
                                mtm(),
                            );
                            b.setTag(tag);
                            b.setBordered(false);
                            footer_title(&b);
                            shell.addSubview(&b);
                            b
                        };
                        let copy = (!user).then(|| action("복사", 1000 + i as isize));
                        let retry = (!user).then(|| action("다시 시도", 2000 + i as isize));
                        let notes = (!user).then(|| [action("나중에 정리", NOTE_TAG + i as isize), action("노트로 정리", NOTE_TAG + 1000 + i as isize), action("그대로 저장", NOTE_TAG + 2000 + i as isize)]);
                        let context = (user && t["context"].is_object()).then(|| {
                            let c = &t["context"];
                            let b = action("", CONTEXT_TAG + i as isize);
                            context_title(&b, &format!("{} · {}", strv(c, "title"), strv(c, "label")));
                            let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("doc.text"), None);
                            b.setImage(icon.as_deref());
                            b.setImagePosition(NSCellImagePosition::ImageLeading);
                            b.setContentTintColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(0.31, 0.275, 0.75, 1.)));
                            b.setToolTip(Some(&NSString::from_str("노트에서 이 부분 보기")));
                            b.setAccessibilityLabel(Some(&NSString::from_str(&format!("붙인 노트 {} · {} · 노트에서 보기", strv(c, "title"), strv(c, "label")))));
                            b
                        });
                        self.document.addSubview(&shell);
                        self.bubbles.insert(
                            key.clone(),
                            Bubble {
                                shell,
                                background,
                                label,
                                text,
                                waiting,
                                rich,
                                copy,
                                retry,
                                notes,
                                context,
                                user,
                                body: String::new(),
                                state: String::new(),
                                marks: String::new(),
                            },
                        );
                        changed = true;
                    }
                    let b = self.bubbles.get_mut(&key).unwrap();
                    if b.body != body || b.state != state || b.marks != marks || changed {
                        let selection = NSTextInputClient::selectedRange(&*b.text);
                        b.text.setString(&NSString::from_str(&body));
                        if selection.location + selection.length <= b.text.string().length() {
                            b.text.setSelectedRange(selection);
                        }
                        let role = if user {
                            "나"
                        } else if provider == "codex" {
                            "Codex"
                        } else {
                            "Claude"
                        };
                        let status = match state.as_str() {
                            "running" if !body.is_empty() => "답변 작성 중",
                            "interrupted" => "중단됨",
                            "failed" => "실패",
                            _ => "",
                        };
                        b.label.setStringValue(&NSString::from_str(status));
                        b.label.setHidden(status.is_empty());
                        let _: () = msg_send![&b.text,setAccessibilityLabel:&*NSString::from_str(&format!("{role} 메시지"))];
                        if let Some(copy) = &b.copy {
                            copy.setEnabled(!body.is_empty());
                            copy.setHidden(body.is_empty() && state == "running");
                        }
                        if let Some(retry) = &b.retry {
                            retry.setHidden(!matches!(state.as_str(), "failed" | "interrupted"));
                        }
                        if let Some(notes) = &b.notes {
                            let done = state == "completed" && !body.is_empty();
                            let (mark, distill) = marks.split_once('/').unwrap_or(("", ""));
                            let titles = [
                                if mark == "later" { "나중에 정리 ✓" } else { "나중에 정리" },
                                match distill { "queued" => "정리 중…", "done" => "정리됨 ✓", _ => "노트로 정리" },
                                if distill == "kept" { "저장됨 ✓" } else { "그대로 저장" },
                            ];
                            for (n, button) in notes.iter().enumerate() {
                                button.setTitle(&NSString::from_str(titles[n]));
                                footer_title(button);
                                button.setHidden(!done);
                            }
                            // 정리하는 중이거나 이미 노트가 된 답은 다시 누를 수 없다.
                            notes[0].setEnabled(!matches!(distill, "queued" | "done" | "kept"));
                            notes[1].setEnabled(!matches!(distill, "queued" | "done" | "kept"));
                            notes[2].setEnabled(!matches!(distill, "queued" | "kept"));
                        }
                        b.body = body;
                        b.state = state;
                        b.marks = marks;
                        changed = true;
                    }
                    let waiting = !b.user && b.body.is_empty() && b.state == "running";
                    if let Some(rich) = &mut b.rich {
                        changed |= rich.update(&b.body, b.state != "running");
                        rich.view.setHidden(waiting || !rich.ready);
                    }
                    b.text.setHidden(waiting || b.rich.as_ref().is_some_and(|r| r.ready));
                    b.waiting.update(waiting);
                }
            }
        }
        self.bubbles.retain(|key, b| {
            if !order.contains(key) {
                b.shell.removeFromSuperview();
                false
            } else {
                true
            }
        });
        self.order = order;
        if changed {
            self.layout();
            if near {
                self.bottom();
            } else {
                self.restore(bounds.origin);
            }
        }
        changed && !near
    }
    unsafe fn layout(&self) {
        let w = self.width;
        let mut y = 6.;
        self.empty.setHidden(!self.order.is_empty());
        self.hint.setHidden(!self.order.is_empty());
        self.empty.setFrame(rect(16., 90., w - 32., 28.));
        self.hint.setFrame(rect(16., 130., w - 32., 48.));
        // 말풍선은 글 폭에 맞추되 패널 폭의 86%를 넘지 않는다.
        let side = 14.;
        let (pad_x, pad_y) = (13., 9.);
        let max_text = ((w - side * 2.) * 0.86 - pad_x * 2.).max(60.);
        for key in &self.order {
            let b = &self.bubbles[key];
            let waiting = !b.user && b.body.is_empty() && b.state == "running";
            let (text_width, height) =
                if waiting { (32., 18.) }
                else if let Some(rich) = b.rich.as_ref().filter(|r| r.ready) { (max_text, rich.height) }
                else if let (Some(c), Some(l)) = (b.text.textContainer(), b.text.layoutManager()) {
                    c.setContainerSize(NSSize::new(max_text, f64::MAX));
                    l.ensureLayoutForTextContainer(&c);
                    let used = l.usedRectForTextContainer(&c).size;
                    // 배치된 영역은 컨테이너 폭 전체로 잡히므로, 글의 실제 폭은 따로 잰다.
                    let ink = b.text.textStorage().map(|storage| storage
                        .boundingRectWithSize_options(NSSize::new(max_text, f64::MAX), NSStringDrawingOptions::UsesLineFragmentOrigin)
                        .size.width).unwrap_or(max_text);
                    (ink.ceil().clamp(18., max_text), used.height.ceil().max(18.))
                } else {
                    (max_text, 40.)
                };
            let bubble_width = text_width + pad_x * 2.;
            let bubble_height = height + pad_y * 2.;
            // 답변 아래에는 복사·다시 시도와 상태 글자가 놓이는 줄이 있다.
            let footer = if b.user { 0. } else { 22. };
            // 붙인 노트의 이름표는 질문 말풍선 위, 오른쪽 끝에 맞춘다.
            let tag_width = b.context.as_ref().map_or(0., |c| (c.intrinsicContentSize().width.ceil() + 4.).min(w - side * 2.));
            let top = if b.context.is_some() { CONTEXT_HEIGHT } else { 0. };
            let row_width = if b.user { bubble_width.max(tag_width) } else { (w - side * 2.).max(bubble_width) };
            let x = if b.user { w - side - row_width } else { side };
            b.shell.setFrame(rect(x, y, row_width, top + bubble_height + footer));
            let bubble_x = if b.user { row_width - bubble_width } else { 0. };
            if let Some(c) = &b.context { c.setFrame(rect(row_width - tag_width, 0., tag_width, CONTEXT_HEIGHT - 4.)); }
            b.background.setFrame(rect(bubble_x, top, bubble_width, bubble_height));
            b.background.setNeedsDisplay(true);
            b.text.setFrame(rect(bubble_x + pad_x, top + pad_y, text_width + 1., height));
            if let Some(rich) = &b.rich { rich.view.setFrame(rect(pad_x, pad_y, text_width, height)); }
            b.waiting.view.setFrame(rect(pad_x, pad_y, 32., 18.));
            let mut next = 9.;
            if let Some(copy) = &b.copy {
                copy.setFrame(rect(next, bubble_height + 2., 30., 18.));
                next += 34.;
            }
            if let Some(retry) = &b.retry {
                retry.setFrame(rect(next, bubble_height + 2., 52., 18.));
                if !retry.isHidden() { next += 56.; }
            }
            if let Some(notes) = &b.notes {
                for button in notes.iter().filter(|button| !button.isHidden()) {
                    let width = button.intrinsicContentSize().width.ceil() + 4.;
                    button.setFrame(rect(next, bubble_height + 2., width, 18.));
                    next += width + 4.;
                }
            }
            b.label.setFrame(rect(next + 2., bubble_height + 4., 120., 14.));
            y += top + bubble_height + footer + 10.;
        }
        self.document.setFrame(rect(
            0.,
            0.,
            w,
            y.max(self.scroll.contentView().bounds().size.height),
        ));
    }
}
