//! Persistent native chat views and a background JSONL reader.
use super::*;
use std::{
    collections::HashMap,
    io::{BufRead, BufReader},
};
thread_local! {
    static EVENTS:RefCell<Vec<i64>>=const{RefCell::new(Vec::new())};
    /// 붙여넣기·끌어놓기로 받은 이미지. 입력란은 패널을 빌리지 않고 여기에 쌓는다.
    static INCOMING:RefCell<Vec<Result<chat_attachments::Attachment,String>>>=const{RefCell::new(Vec::new())};
}
const EVENT_INCOMING: i64 = 12;
const EVENT_ATTACH: i64 = 13;
const EVENT_PAUSE_RUN: i64 = 14;
use objc2::{runtime::AnyClass, DefinedClass};
const EVENT_RESIZE: i64 = 15;
const EVENT_HISTORY: i64 = 16;
const EVENT_HISTORY_RESIZE: i64 = 17;
const EVENT_HISTORY_RESIZE_END: i64 = 18;
const HISTORY_TOP: f64 = 24.;
const HISTORY_FADE_HEIGHT: f64 = 32.;
const HISTORY_HANDLE_HEIGHT: f64 = 20.;
const HISTORY_ROW_HEIGHT: f64 = 54.;
const HISTORY_EMPTY_HEIGHT: f64 = 44.;
const HISTORY_DEFAULT_HEIGHT: f64 = 360.;
const HISTORY_INSET: f64 = 8.;
const HISTORY_SLIDE_SECONDS: f64 = 0.18;
/// 입력창을 위로 끌어 키운 높이(pt). 드래그 핸들이 쓰고 레이아웃이 읽는다.
const INPUT_GROW_MAX: f64 = 360.;
thread_local! { static INPUT_GROW: Cell<f64> = const { Cell::new(0.) }; }
thread_local! { static HISTORY_DRAG_HEIGHT: Cell<f64> = const { Cell::new(0.) }; }
define_class!(
    /// 입력창 바로 위에서 대화를 흐리게 하는 띠. 눌림과 스크롤은 아래 대화가 받는다.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "StologyChatBottomBlur"]
    struct BottomBlur;

    unsafe impl NSObjectProtocol for BottomBlur {}

    impl BottomBlur {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> { None }
    }
);
/// 대화 영역의 바닥. 입력창 윗선(138)보다 6pt 위.
const MESSAGES_BOTTOM: f64 = 144.;
/// 입력창 위 흐림 띠 높이.
const BOTTOM_BLUR_HEIGHT: f64 = 56.;

/// 흐림 층 수. 층마다 약하게 흐리고 아래로 갈수록 짧아져, 입력창 쪽에서 흐림이 겹쳐 진해진다.
/// 배경 흐림 필터는 레이어 마스크를 따르지 않으므로 그라데이션 대신 층을 겹친다.
const BOTTOM_BLUR_LAYERS: usize = 7;

fn passthrough_view(frame: NSRect) -> Retained<NSView> {
    let view: Retained<BottomBlur> = unsafe { msg_send![BottomBlur::alloc(mtm()), initWithFrame: frame] };
    let view = Retained::into_super(view);
    view.setAccessibilityElement(false);
    view
}

/// 입력창 위 흐림 띠. 안쪽 층들은 `layout_bottom_blur`가 배치한다.
unsafe fn bottom_blur() -> Retained<NSView> {
    let band = passthrough_view(rect(0., 0., 300., BOTTOM_BLUR_HEIGHT));
    band.setWantsLayer(true);
    for _ in 0..BOTTOM_BLUR_LAYERS {
        let layer = passthrough_view(rect(0., 0., 300., BOTTOM_BLUR_HEIGHT));
        layer.setWantsLayer(true);
        layer.setLayerUsesCoreImageFilters(true);
        if let Some(class) = AnyClass::get(c"CIFilter") {
            let blur: Option<Retained<AnyObject>> = msg_send![class, filterWithName: &*NSString::from_str("CIGaussianBlur")];
            if let Some(blur) = blur {
                let _: () = msg_send![&blur, setDefaults];
                let _: () = msg_send![&blur, setValue: &*NSNumber::new_f64(1.6), forKey: &*NSString::from_str("inputRadius")];
                let _: () = msg_send![&layer, setBackgroundFilters: &*NSArray::from_slice(&[&*blur])];
            }
        }
        band.addSubview(&layer);
    }
    band
}

/// 층 `i`는 바닥에서 띠 높이의 (n - i)/n까지 덮는다. 맨 아래는 모든 층이 겹친다.
unsafe fn layout_bottom_blur(band: &NSView, width: f64) {
    let n = BOTTOM_BLUR_LAYERS as f64;
    for (i, layer) in band.subviews().iter().enumerate() {
        let height = BOTTOM_BLUR_HEIGHT * (n - i as f64) / n;
        layer.setFrame(rect(0., 0., width, height));
    }
}
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct HistoryViewport;

    unsafe impl NSObjectProtocol for HistoryViewport {}

    impl HistoryViewport {
        #[unsafe(method_id(hitTest:))]
        unsafe fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let hit: Option<Retained<NSView>> = msg_send![super(self), hitTest: point];
            hit.filter(|view| !std::ptr::eq(&**view, self.as_super()))
        }
    }
);
define_class!(
 #[unsafe(super(NSView))] #[thread_kind=MainThreadOnly] #[ivars = Cell<(f64,f64)>] struct ResizeHandle;
 unsafe impl NSObjectProtocol for ResizeHandle {}
 impl ResizeHandle {
  #[unsafe(method(resetCursorRects))] fn cursor(&self){self.addCursorRect_cursor(self.bounds(),&NSCursor::resizeUpDownCursor());}
  #[unsafe(method(acceptsFirstMouse:))] fn first(&self,_e:Option<&NSEvent>)->bool{true}
  #[unsafe(method(mouseDown:))] fn down(&self,e:&NSEvent){self.ivars().set((e.locationInWindow().y,INPUT_GROW.with(Cell::get)));}
  #[unsafe(method(mouseDragged:))] fn drag(&self,e:&NSEvent){
    let (y,grow)=self.ivars().get();
    INPUT_GROW.with(|g|g.set((grow+e.locationInWindow().y-y).clamp(0.,INPUT_GROW_MAX)));
    enqueue(EVENT_RESIZE);
  }
 }
);
define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<(f64, f64)>]
    struct HistoryResizeHandle;

    unsafe impl NSObjectProtocol for HistoryResizeHandle {}

    impl HistoryResizeHandle {
        #[unsafe(method(resetCursorRects))]
        fn reset_cursor_rects(&self) {
            self.addCursorRect_cursor(self.bounds(), &NSCursor::resizeUpDownCursor());
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool { true }

        #[unsafe(method(mouseDown:))]
        unsafe fn mouse_down(&self, event: &NSEvent) {
            if let Some(sheet) = self.superview() {
                self.ivars().set((event.locationInWindow().y, sheet.bounds().size.height));
            }
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let (start_y, start_height) = self.ivars().get();
            HISTORY_DRAG_HEIGHT.with(|height| height.set(start_height + start_y - event.locationInWindow().y));
            enqueue(EVENT_HISTORY_RESIZE);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let (start_y, start_height) = self.ivars().get();
            HISTORY_DRAG_HEIGHT.with(|height| height.set(start_height + start_y - event.locationInWindow().y));
            enqueue(EVENT_HISTORY_RESIZE_END);
        }
    }
);
unsafe fn take_images(board: &NSPasteboard, text_first: bool) -> bool {
    let Some(items) = chat_attachments::from_pasteboard(board, text_first) else { return false };
    INCOMING.with(|q| q.borrow_mut().extend(items));
    enqueue(EVENT_INCOMING);
    true
}
define_class!(
 #[unsafe(super(NSObject))] #[thread_kind=MainThreadOnly] struct ChatTarget;
 unsafe impl NSObjectProtocol for ChatTarget {}
 impl ChatTarget {#[unsafe(method(chatAction:))] fn action(&self,sender:&AnyObject){let tag:i64=unsafe{msg_send![sender,tag]};enqueue(tag);}}
);
define_class!(
 #[unsafe(super(NSTextView))] #[thread_kind=MainThreadOnly] struct ChatInput;
 unsafe impl NSObjectProtocol for ChatInput {}
 impl ChatInput {#[unsafe(method(keyDown:))] unsafe fn key_down(&self,event:&NSEvent){
   let marked:bool=msg_send![self,hasMarkedText];
   if !marked && event.modifierFlags().contains(NSEventModifierFlags::Command) && event.keyCode()==36 {enqueue(1);return;}
   let _:()=msg_send![super(self),keyDown:event];
 }
 // 이미지는 입력란 안에 넣지 않고 입력창 위 첨부 줄로 보낸다. 글자가 함께 있으면 글자를 붙인다.
 #[unsafe(method(paste:))] unsafe fn paste(&self,sender:Option<&AnyObject>){
   if !take_images(&NSPasteboard::generalPasteboard(),true) {let _:()=msg_send![super(self),paste:sender];}
 }
 #[unsafe(method(validateUserInterfaceItem:))] unsafe fn validate(&self,item:&AnyObject)->bool{
   let action:Option<Sel>=msg_send![item,action];
   if action==Some(sel!(paste:)) && self.isEditable() && chat_attachments::pasteboard_has_images(&NSPasteboard::generalPasteboard()) {true}
   else {msg_send![super(self),validateUserInterfaceItem:item]}
 }
 #[unsafe(method(performDragOperation:))] unsafe fn drop(&self,info:&AnyObject)->bool{
   let board:Retained<NSPasteboard>=msg_send![info,draggingPasteboard];
   if take_images(&board,false) {true} else {msg_send![super(self),performDragOperation:info]}
 }}
);
// Bind input actions to the current conversation before another native click
// can switch the workspace. The timer remains the fallback during reentrancy.
fn enqueue(event: i64) {
    EVENTS.with(|e| e.borrow_mut().push(event));
    STATE.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else {
            return;
        };
        if let Some(s) = state.as_mut() {
            unsafe {
                let ws = s.workspace.clone();
                if s.chat.tick(ws.as_deref()) {
                    s.connect();
                }
            }
        }
    });
}
struct Running {
    rx: mpsc::Receiver<Value>,
    stop: mpsc::Sender<()>,
    pending: Value,
    workspace: String,
    provider: String,
    conversation: String,
    turn: String,
    run: String,
    sequence: u64,
    accepted: bool,
    terminal: bool,
    answer: String,
    prompt: String,
    images: usize,
    /// 이 질문에 붙인 노트.
    context: Value,
}
impl Running {
    fn pending_in(&self, workspace: Option<&str>, provider: &str) -> bool {
        !self.accepted && workspace == Some(self.workspace.as_str()) && provider == self.provider
    }
}
pub struct Panel {
    pub view: Retained<NSView>,
    _target: Retained<ChatTarget>,
    model: Retained<NSPopUpButton>,
    model_display: Retained<NSMenuItem>,
    model_items: Vec<Value>,
    effort: Retained<NSPopUpButton>,
    model_chip: Retained<NSView>,
    model_dot: Retained<NSView>,
    effort_chip: Retained<NSView>,
    chevrons: [Retained<NSView>; 2],
    model_title: RefCell<String>,
    effort_items: Vec<String>,
    selected_efforts: HashMap<String, String>,
    selected_models: HashMap<String, String>,
    models_rx: Option<mpsc::Receiver<(String, Result<Value, String>)>>,
    catalogs: HashMap<String, Vec<Value>>,
    catalog_errors: HashMap<String, String>,
    /// 목록은 읽었지만 이 CLI로는 채팅 실행 제한을 걸 수 없는 공급자와 그 이유.
    chat_blocked: HashMap<String, String>,
    catalogs_pending: usize,
    model_refresh_queue: ModelRefreshQueue,
    provider_info: Value,
    send_is_stopping: Option<bool>,
    send_enabled: Option<bool>,
    status_shown: Option<(String, bool)>,
    messages: chat_messages::Messages,
    composer: Retained<NSView>,
    /// 입력창 위에서 스크롤되는 대화를 흐리게 하는 띠.
    bottom_blur: Retained<NSView>,
    placeholder: Retained<NSTextField>,
    input: Retained<NSTextView>,
    input_scroll: Retained<NSScrollView>,
    resize_handle: Retained<NSView>,
    send: Retained<NSButton>,
    new: Retained<NSButton>,
    search: Retained<NSSearchField>,
    history: Retained<NSButton>,
    history_viewport: Retained<NSView>,
    drawer: Retained<NSView>,
    list: Retained<NSScrollView>,
    history_handle: Retained<NSView>,
    history_open: bool,
    history_close_at: Option<Instant>,
    history_height: Option<f64>,
    history_query: String,
    matches: Vec<Value>,
    searched: Option<String>,
    bottom: Retained<NSButton>,
    retry_save: Retained<NSButton>,
    unsaved: Option<Value>,
    /// The subject whose run this turn waits behind, until the provider accepts the turn.
    waiting: Option<String>,
    pause_run: Retained<NSButton>,
    scroll_to_bottom: bool,
    status: Retained<NSTextField>,
    notice: Retained<NSTextField>,
    workspace: Option<String>,
    selected: String,
    data: Value,
    last_draft: String,
    positions: HashMap<String, NSPoint>,
    source_jobs: Vec<mpsc::Receiver<(String, Value)>>,
    running: Option<Running>,
    error: String,
    connection: String,
    attachments: Vec<chat_attachments::Attachment>,
    strip: chat_attachments::Strip,
    attach: Retained<NSButton>,
    size: (f64, f64),
    /// 마지막 답이 끝난 때. 이만큼 조용하면 정리 대상을 뒤에서 정리하게 넘긴다.
    quiet_since: Option<Instant>,
    quiet_for: Duration,
    /// 잠깐 보여 주는 안내.
    flash: Option<(String, Instant)>,
    /// 정리 제안이 바뀌었을 수 있다. 자료 탭이 다시 읽는다.
    notes_changed: bool,
    /// 편집기에 열려 있는 노트. 메인 화면이 틱마다 넘긴다.
    note: Option<chat_note::NoteSource>,
    note_chip: chat_note::NoteChip,
    /// ⌘L로 붙인 노트 발췌. 다음 질문과 함께 간다.
    attached: Option<Value>,
    /// 이름표를 눌러 노트에서 다시 볼 문맥. 메인 화면이 가져간다.
    reveal: Option<Value>,
}
fn versioned_model(provider: &str, id: &str) -> bool {
    if provider == "antigravity" { return !id.is_empty() && !["auto","default","latest"].contains(&id); }
    let prefix = if provider == "claude" { "claude-" } else { "gpt-" };
    id.starts_with(prefix) && id.bytes().any(|c| c.is_ascii_digit())
        && !id.contains("latest")
}
fn resolved_selection<'a>(provider: &str, selected: &str, catalog: &'a [Value]) -> Option<&'a Value> {
    if versioned_model(provider, selected) { return None; }
    let entry = catalog.iter().find(|m| m["id"] == selected || (selected.is_empty() && m["default"] == true))?;
    let id = entry["resolved_model"].as_str().or_else(|| entry["id"].as_str())?;
    if !versioned_model(provider, id) { return None; }
    catalog.iter().find(|m| m["id"] == id)
}
#[derive(Default)]
struct ModelRefreshQueue(Vec<&'static str>);
impl ModelRefreshQueue {
    fn observe(&mut self, previous: &Value, current: &Value) {
        let connected = |info: &Value, provider: &str| info["providers"].as_array()
            .is_some_and(|ps| ps.iter().any(|p| p["provider"] == provider && p["installed"] == true && p["logged_in"] == true));
        // 같은 공급자라도 CLI가 바뀌면(업데이트·다른 경로) 지원 기능을 다시 확인한다.
        let cli = |info: &Value, provider: &str| info["providers"].as_array()
            .and_then(|ps| ps.iter().find(|p| p["provider"] == provider))
            .map(|p| (p["path"].clone(), p["version"].clone()));
        for provider in ["claude", "codex", "antigravity"] {
            let changed = !connected(previous, provider) || cli(previous, provider) != cli(current, provider);
            if connected(current, provider) && changed && !self.0.contains(&provider) {
                self.0.push(provider);
            }
        }
    }
    fn take_if_idle(&mut self, loading: bool) -> Vec<&'static str> {
        if loading { Vec::new() } else { std::mem::take(&mut self.0) }
    }
}

fn combined_models(catalogs: &HashMap<String, Vec<Value>>, errors: &HashMap<String,String>, selected: &HashMap<String,String>, info: &Value) -> Vec<Value> {
    let mut items = Vec::new();
    for (provider, name) in [("claude", "Claude"), ("codex", "Codex"), ("antigravity", "Antigravity")] {
        let catalog = catalogs.get(provider).cloned().unwrap_or_default();
        let mut rows: Vec<_> = catalog.into_iter()
            .filter(|m| m["id"].as_str().is_some_and(|id| versioned_model(provider, id)))
            .map(|mut m| { m["listed"] = json!(true); m }).collect();
        if let Some(model) = selected.get(provider).filter(|m| versioned_model(provider, m)) {
            if !rows.iter().any(|m|m["id"]==*model) {rows.push(json!({"id":model,"label":model,"listed":false}));}
        }
        let connected = info["providers"].as_array().is_some_and(|a|a.iter().any(|p|p["provider"]==provider && p["installed"]==true && p["logged_in"]==true));
        for mut row in rows {
            row["provider"]=json!(provider);
            row["title"]=json!(format!("{name} · {}",strv(&row,"label")));
            row["available"]=json!(connected && !errors.contains_key(provider) && row["listed"]==true && row["available"]!=false);
            items.push(row);
        }
    }
    items
}
/// 공급자 모델 줄 바로 아래에 둘 CLI 도움말. 목록 조회 실패나 채팅 차단 이유를 그 공급자 곁에서 알린다.
fn with_cli_help(items: Vec<Value>, help: &HashMap<String, String>) -> Vec<Value> {
    let mut out = Vec::new();
    for provider in ["claude", "codex", "antigravity"] {
        out.extend(items.iter().filter(|m| m["provider"] == provider).cloned());
        if let Some(text) = help.get(provider) {
            out.push(json!({"provider":provider,"helper":true,"title":text,"available":false}));
        }
    }
    out
}
/// 도움말 글은 " · "로 나뉜 조각마다 줄을 바꾸고, 긴 조각은 메뉴 폭에 맞춰 다시 줄바꿈한다.
/// 첫 조각은 해야 할 일이다. SUIT가 "--"를 긴 줄표로 바꾸지 않게 사이를 끊는다.
fn cli_help_lines(text: &str) -> String {
    let mut lines = Vec::new();
    for part in text.split(" · ") {
        let mut line = String::new();
        for word in part.split(' ') {
            if !line.is_empty() && line.chars().count() + word.chars().count() > 34 {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() { line.push(' '); }
            line.push_str(word);
        }
        if !line.is_empty() { lines.push(line); }
    }
    lines.join("\n").replace("--", "-\u{200C}-")
}
unsafe fn helper_menu_title(item: &NSMenuItem, text: &str) {
    let lines = cli_help_lines(text);
    let shown = NSString::from_str(&lines);
    let attributed = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &shown);
    let full = NSRange::new(0, shown.length());
    attributed.addAttribute_value_range(NSFontAttributeName, &font::regular(11.), full);
    attributed.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), full);
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setLineSpacing(2.);
    attributed.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, full);
    if let Some(end) = lines.find('\n') {
        // 첫 줄(무엇을 해야 하는지)은 본문 색으로 또렷하게.
        let head = NSRange::new(0, NSString::from_str(&lines[..end]).length());
        attributed.addAttribute_value_range(NSFontAttributeName, &font::weighted(11., 0.2), head);
        attributed.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::labelColor(), head);
    }
    item.setAttributedTitle(Some(&attributed));
    item.setEnabled(false);
    let _: () = msg_send![item, setAccessibilityLabel:&*NSString::from_str(text)];
}
unsafe fn model_menu_title(item: &NSMenuItem, title: &str, available: bool) {
    let text = NSString::from_str(&format!("{title}\t●"));
    let attributed = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
    let full = NSRange::new(0,text.length());
    attributed.addAttribute_value_range(NSFontAttributeName, &font::regular(12.), full);
    let paragraph = NSMutableParagraphStyle::new();
    let tab = NSTextTab::initWithTextAlignment_location_options(NSTextTab::alloc(), NSTextAlignment::Right, 340., &NSDictionary::new());
    paragraph.setTabStops(Some(&NSArray::from_retained_slice(&[tab])));
    attributed.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, full);
    let dot = NSRange::new(text.length()-1,1);
    let color = if available {NSColor::systemGreenColor()} else {NSColor::systemGrayColor()};
    attributed.addAttribute_value_range(NSForegroundColorAttributeName, &color, dot);
    attributed.addAttribute_value_range(NSFontAttributeName, &font::regular(8.), dot);
    item.setAttributedTitle(Some(&attributed));
}
/// 상태 줄 글자. "● 연결됨"의 점만 초록으로 칠한다.
unsafe fn status_text(label: &NSTextField, message: &str, quiet: bool) {
    let text = NSString::from_str(message);
    let attributed = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &text);
    let full = NSRange::new(0, text.length());
    attributed.addAttribute_value_range(NSFontAttributeName, &font::regular(11.), full);
    let ink = if quiet { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
    attributed.addAttribute_value_range(NSForegroundColorAttributeName, &ink, full);
    if message.starts_with("● ") {
        let dot = NSRange::new(0, 1);
        attributed.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::systemGreenColor(), dot);
        attributed.addAttribute_value_range(NSFontAttributeName, &font::regular(8.), dot);
    }
    label.setAttributedStringValue(&attributed);
}
/// 칩 폭 (effort, model). 전송 버튼 왼쪽에 6pt 간격을 남기고, 모자라면 모델 칩이 줄어든다.
const CHIP_MODEL_PADDING: f64 = 50.;
/// 맨 왼쪽 이미지 첨부(+) 버튼이 차지하는 폭.
const ATTACH_SPACE: f64 = 26.;
fn chip_widths(panel_width: f64, model_text: f64, effort_text: f64) -> (f64, f64) {
    let effort = (effort_text + 38.).clamp(52., 150.);
    let room = (panel_width - 88. - ATTACH_SPACE - effort).max(64.);
    (effort, (model_text + CHIP_MODEL_PADDING).clamp(64., room))
}
unsafe fn send_button_state(button: &NSButton, stopping: bool) {
    button.setTitle(&NSString::from_str(""));
    button.setImagePosition(NSCellImagePosition::ImageOnly);
    if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(if stopping {"stop.fill"} else {"arrow.up"}),None) {
        let configuration = NSImageSymbolConfiguration::configurationWithPointSize_weight(theme::CHAT_CONTROL_ICON_SIZE,0.3);
        button.setImage(image.imageWithSymbolConfiguration(&configuration).as_deref());
    }
    let label = NSString::from_str(if stopping {"응답 중지"}else{"질문 전송"});
    let _: () = msg_send![button,setAccessibilityLabel:&*label];
}
unsafe fn area(input: bool) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    let scroll = crate::scrollbar::vertical(rect(0., 0., 300., 100.));
    let text: Retained<NSTextView> = if input {
        let t: Retained<ChatInput> =
            msg_send![ChatInput::alloc(mtm()),initWithFrame:rect(0.,0.,300.,100.)];
        Retained::into_super(t)
    } else {
        NSTextView::initWithFrame(NSTextView::alloc(mtm()), rect(0., 0., 300., 100.))
    };
    text.setRichText(false);
    text.setImportsGraphics(false);
    text.setEditable(input);
    text.setSelectable(true);
    text.setAllowsUndo(input);
    text.setFont(Some(&font::regular(13.)));
    text.setTextColor(Some(&NSColor::labelColor()));
    text.setBackgroundColor(&NSColor::textBackgroundColor());
    text.setVerticallyResizable(true);
    text.setHorizontallyResizable(false);
    text.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
    text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    text.setTextContainerInset(NSSize::new(10., 10.));
    if let Some(c) = text.textContainer() {
        c.setWidthTracksTextView(true);
        c.setContainerSize(NSSize::new(280., f64::MAX));
    }
    let _: () = msg_send![&text,setAccessibilityLabel:&*NSString::from_str(if input{"질문 입력"}else{"학습도우미 대화"})];
    scroll.setDocumentView(Some(&text));
    (scroll, text)
}
impl Panel {
    pub unsafe fn new() -> Self {
        let view = view(rect(0., 0., 340., 700.));
        let target: Retained<ChatTarget> = msg_send![ChatTarget::alloc(mtm()), init];
        let button = |title: &str, tag: i64| {
            let b = NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title),
                Some(&*target),
                Some(sel!(chatAction:)),
                mtm(),
            );
            b.setTag(tag as isize);
            theme::button(&b,false);
            view.addSubview(&b);
            b
        };
        let model = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm()), rect(0.,0.,120.,28.), false);
        // CLI 도움말 행은 꺼 둔 채로 보여야 한다.
        model.setAutoenablesItems(false);
        model.setTarget(Some(&*target));
        model.setAction(Some(sel!(chatAction:)));
        model.setTag(9);
        theme::button(&model, false);
        model.setBordered(false);
        model.setUsesItemFromMenu(false);
        model.setFont(Some(&font::regular(11.5)));
        let _: () = msg_send![&model,setAccessibilityLabel:&*NSString::from_str("모델 선택 · Claude, Codex, Antigravity")];
        let effort = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm()), rect(0.,0.,150.,28.), false);
        effort.setTarget(Some(&*target));
        effort.setAction(Some(sel!(chatAction:)));
        effort.setTag(10);
        theme::button(&effort, false);
        effort.setBordered(false);
        effort.setFont(Some(&font::regular(11.5)));
        let _: () = msg_send![&effort,setAccessibilityLabel:&*NSString::from_str("Effort · 추론 강도")];
        // 팝업의 위아래 겹화살표는 끄고, 칩에 아래 화살표 하나를 따로 그린다.
        for popup in [&model, &effort] {
            let cell: Retained<NSPopUpButtonCell> = msg_send![&**popup, cell];
            cell.setArrowPosition(NSPopUpArrowPosition::NoArrow);
        }
        let messages = chat_messages::Messages::new();
        view.addSubview(&messages.scroll);
        let bottom_blur = bottom_blur();
        view.addSubview(&bottom_blur);
        let composer = theme::chat_composer(rect(12., 30., 316., 140.));
        view.addSubview(&composer);
        let (input_scroll, input) = area(true);
        // 유리 캡슐 안에서는 글이 두 줄 보이는 낮은 입력란을 쓴다.
        input.setTextContainerInset(NSSize::new(6., 4.));
        input_scroll.setAutohidesScrollers(true);
        input_scroll.setDrawsBackground(false);
        input.setDrawsBackground(false);
        view.addSubview(&input_scroll);
        let placeholder = text_label("질문을 입력하세요…", rect(26., 108., 270., 20.), 13., false);
        placeholder.setTextColor(Some(&NSColor::placeholderTextColor()));
        view.addSubview(&placeholder);
        view.addSubview(&input_scroll);
        let resize_handle: Retained<NSView> = {
            let allocated = ResizeHandle::alloc(mtm()).set_ivars(Cell::new((0., 0.)));
            let v: Retained<ResizeHandle> = msg_send![super(allocated), initWithFrame:rect(0.,0.,100.,10.)];
            let _: () = msg_send![&v, setAccessibilityLabel:&*NSString::from_str("입력창 크기 조절")];
            Retained::into_super(v)
        };
        view.addSubview(&resize_handle);
        let status = text_label(
            "과목을 선택하면 대화를 시작할 수 있어요",
            rect(12., 600., 316., 40.),
            11.,
            false,
        );
        status.setMaximumNumberOfLines(3);
        view.addSubview(&status);
        let notice = text_label(
            "질문과 직접 첨부한 내용만 전송돼요 · ⌘ + Enter 전송",
            rect(12., 144., 316., 18.),
            10.5,
            false,
        );
        notice.setTextColor(Some(&NSColor::tertiaryLabelColor()));
        notice.setAlignment(NSTextAlignment::Right);
        view.addSubview(&notice);
        let preferences = rpc(json!({"op":"chat.preferences"})).unwrap_or(Value::Null);
        let selected_efforts = preferences["efforts"].as_object().into_iter().flatten().filter_map(|(k,v)|v.as_str().map(|v|(k.clone(),v.to_owned()))).collect();
        let selected_models = ["codex", "claude", "antigravity"].into_iter().map(|p| (p.to_owned(), strv(&preferences["models"],p))).collect();
        let selected = preferences["provider"].as_str().map(str::to_owned)
            .unwrap_or_else(|| "codex".into());
        let send = button("", 11);
        theme::chat_send(&send, false);
        // 칩 바탕과 상태 점은 팝업보다 먼저 넣어 글자 뒤에 깔리게 한다.
        let model_chip = theme::chat_chip(rect(22., 46., 170., 28.));
        let effort_chip = theme::chat_chip(rect(198., 46., 62., 28.));
        let model_dot = theme::chat_status_dot(rect(33., 56.5, 7., 7.));
        let chevrons = [theme::chat_chevron(), theme::chat_chevron()];
        view.addSubview(&model_chip);
        view.addSubview(&effort_chip);
        model_chip.addSubview(&model_dot);
        model_chip.addSubview(&chevrons[0]);
        effort_chip.addSubview(&chevrons[1]);
        model_chip.addSubview(&model);
        effort_chip.addSubview(&effort);
        let attach = button("", EVENT_ATTACH);
        theme::chat_attach_button(&attach);
        let strip = chat_attachments::Strip::new();
        view.addSubview(&strip.scroll);
        let search = NSSearchField::initWithFrame(NSSearchField::alloc(mtm()), rect(0.,0.,160.,24.));
        search.setPlaceholderString(Some(&NSString::from_str("대화 검색")));
        search.setAccessibilityLabel(Some(&NSString::from_str("현재 워크스페이스 대화 검색")));
        view.addSubview(&search);
        let history = button("", EVENT_HISTORY);
        theme::chat_pill_button(&history, "list");
        history.setImagePosition(NSCellImagePosition::ImageOnly);
        history.setAccessibilityLabel(Some(&NSString::from_str("대화 목록")));
        let history_viewport: Retained<HistoryViewport> =
            msg_send![HistoryViewport::alloc(mtm()), initWithFrame:rect(12., 0., 316., 0.)];
        let history_viewport = Retained::into_super(history_viewport);
        history_viewport.setClipsToBounds(true);
        history_viewport.setHidden(true);
        let (drawer, inner) = theme::panel(rect(0., 0., 316., 0.));
        if drawer.respondsToSelector(sel!(setCornerRadius:)) {
            let _: () = msg_send![&drawer, setCornerRadius: 0.0f64];
        }
        let layer: Retained<AnyObject> = msg_send![&drawer, layer];
        let flipped: bool = msg_send![&layer, isGeometryFlipped];
        // CACornerMask uses the layer's Y axis. Round only the lower corners.
        let bottom_corners = if flipped { 0b1100usize } else { 0b0011usize };
        let _: () = msg_send![&layer, setMaskedCorners: bottom_corners];
        let _: () = msg_send![&layer, setCornerRadius: 20.0f64];
        let _: () = msg_send![&layer, setMasksToBounds: true];
        if drawer.respondsToSelector(sel!(setTintColor:)) {
            let tint = suggest_color(1., 1., 1., 0.8);
            let _: () = msg_send![&drawer, setTintColor: &*tint];
        } else {
            let _: () = msg_send![&drawer, setBlendingMode: NSVisualEffectBlendingMode::WithinWindow];
        }
        let sheen_class = AnyClass::get(c"CAGradientLayer").expect("Core Animation gradient");
        let sheen: Retained<AnyObject> = msg_send![sheen_class, layer];
        let light = suggest_color(1., 1., 1., 0.8).CGColor();
        let depth = suggest_color(1., 1., 1., 0.9).CGColor();
        let colors = NSArray::from_slice(&[
            &*(Retained::as_ptr(&light) as *const AnyObject),
            &*(Retained::as_ptr(&depth) as *const AnyObject),
        ]);
        let _: () = msg_send![&sheen, setColors: &*colors];
        let _: () = msg_send![&sheen, setStartPoint: NSPoint::new(0., 1.)];
        let _: () = msg_send![&sheen, setEndPoint: NSPoint::new(1., 0.)];
        let _: () = msg_send![&sheen, setFrame: inner.bounds()];
        let _: () = msg_send![&sheen, setAutoresizingMask: 18u32];
        let content_layer = inner.layer().expect("glass panel content layer");
        let _: () = msg_send![&content_layer, insertSublayer: &*sheen, atIndex: 0u32];
        let rim = suggest_color(1., 1., 1., 0.7);
        let _: () = msg_send![&layer, setBorderColor: &*rim.CGColor()];
        let _: () = msg_send![&layer, setBorderWidth: 0.75f64];
        let list = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm()),
            rect(HISTORY_INSET, HISTORY_INSET, 300., 0.),
        );
        list.setHasVerticalScroller(true);
        list.setBorderType(NSBorderType::NoBorder);
        crate::scrollbar::style(&list);
        list.setDrawsBackground(false);
        inner.addSubview(&list);
        let handle = HistoryResizeHandle::alloc(mtm()).set_ivars(Cell::new((0., 0.)));
        let handle: Retained<HistoryResizeHandle> = msg_send![super(handle), initWithFrame:rect(8., 0., 300., HISTORY_HANDLE_HEIGHT)];
        let _: () = msg_send![&handle, setAccessibilityLabel:&*NSString::from_str("대화 목록 높이 조절")];
        handle.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let grip = super::view(rect(130., 8., 40., 4.));
        grip.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMaxXMargin);
        rounded(&grip, &suggest_color(0.4, 0.4, 0.4, 0.5), 2.);
        handle.addSubview(&grip);
        let handle = Retained::into_super(handle);
        inner.addSubview(&handle);
        history_viewport.addSubview(&drawer);
        view.addSubview_positioned_relativeTo(&history_viewport, NSWindowOrderingMode::Below, Some(&search));
        view.addSubview_positioned_relativeTo(&history, NSWindowOrderingMode::Above, Some(&search));
        let new = button("새 대화", 4);
        theme::chat_pill_button(&new, "note-plus");
        let note_chip = chat_note::NoteChip::new(&view, &target, &composer);
        let bottom = button("새 답변", 6);
        theme::chat_new_answer_button(&bottom);
        bottom.setHidden(true);
        let mut panel = Self {
            model,
            model_display: NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm()), &NSString::from_str(""), None, &NSString::from_str("")),
            effort,
            model_chip,
            model_dot,
            effort_chip,
            chevrons,
            model_title: RefCell::new(String::new()),
            effort_items: Vec::new(),
            selected_efforts,
            model_items: Vec::new(),
            selected_models,
            models_rx: None,
            catalogs: HashMap::new(),
            catalog_errors: HashMap::new(),
            chat_blocked: HashMap::new(),
            catalogs_pending: 0,
            model_refresh_queue: ModelRefreshQueue::default(),
            provider_info: Value::Null,
            send_is_stopping: None,
            send_enabled: None,
            status_shown: None,
            send,
            new,
            search,
            history,
            history_viewport,
            drawer,
            list,
            history_handle: handle,
            history_open: false,
            history_close_at: None,
            history_height: None,
            history_query: String::new(),
            matches: Vec::new(),
            searched: None,
            bottom,
            retry_save: button("저장 재시도", 8),
            unsaved: None,
            waiting: None,
            pause_run: button("정리 멈추고 바로 질문", EVENT_PAUSE_RUN),
            scroll_to_bottom: false,
            view,
            _target: target,
            messages,
            composer,
            bottom_blur,
            placeholder,
            input,
            input_scroll,
            resize_handle,
            status,
            notice,
            workspace: None,
            selected,
            data: Value::Null,
            last_draft: String::new(),
            positions: HashMap::new(),
            source_jobs: Vec::new(),
            running: None,
            error: String::new(),
            connection: "연결 확인 중".into(),
            attachments: Vec::new(),
            strip,
            attach,
            size: (340., 700.),
            quiet_since: None,
            quiet_for: Duration::from_secs(60 * rpc(json!({"op":"ai.settings.get"})).ok().and_then(|v| v["routing"]["distill"]["quiet_minutes"].as_u64()).unwrap_or(5)),
            flash: None,
            notes_changed: false,
            note: None,
            note_chip,
            attached: None,
            reveal: None,
        };
        panel.refresh_models();
        panel

    }
    fn selected_model(&self) -> &str {
        self.selected_models.get(&self.selected).map(String::as_str).unwrap_or("")
    }
    unsafe fn fill_models(&mut self) {
        // Resolve a legacy default/alias only from a fresh CLI catalog. Persist
        // the exact version before presenting it so subsequent requests agree.
        if !self.busy() && !self.catalog_errors.contains_key(&self.selected) {
            let replacement = self.catalogs.get(&self.selected)
                .and_then(|items| resolved_selection(&self.selected, self.selected_model(), items)).cloned();
            if let Some(item) = replacement {
                let id = strv(&item, "id");
                let key = format!("{}.{}", self.selected, id);
                let effort = self.selected_efforts.get(&key).cloned().unwrap_or_else(|| {
                    let old = self.selected_effort();
                    if item["efforts"].as_array().is_some_and(|a| a.iter().any(|v| v == old)) { old.to_owned() } else { String::new() }
                });
                match rpc(json!({"op":"chat.preferences","provider":self.selected,"model":id,"effort":effort})) {
                    Ok(_) => { self.selected_models.insert(self.selected.clone(), id); self.selected_efforts.insert(key, effort); }
                    Err(e) => self.error = format!("모델 선택 저장 실패 · {e}"),
                }
            }
        }
        let models = combined_models(&self.catalogs, &self.catalog_errors, &self.selected_models, &self.provider_info);
        self.model_items = with_cli_help(models, &self.cli_help());
        self.model.removeAllItems();
        for (index, item) in self.model_items.iter().enumerate() {
            let title = strv(item,"title");
            self.model.addItemWithTitle(&NSString::from_str(&title));
            if item["helper"] == true {
                if let Some(row) = self.model.itemAtIndex(index as isize) { helper_menu_title(&row, &title); }
                continue;
            }
            if let Some(row) = self.model.itemAtIndex(index as isize) {
                model_menu_title(&row, &title, item["available"] == true);
                let state = if item["available"] == true { "사용 가능 · 연결 및 모델 목록 확인됨" } else { "사용 불가 또는 확인 필요 · 왼쪽 구독 계정 연결에서 확인" };
                let _: () = msg_send![&row,setAccessibilityLabel:&*NSString::from_str(&format!("{title} · {state}"))];
            }
        }
        self.model.addItemWithTitle(&NSString::from_str("목록 새로고침…"));
        self.model.addItemWithTitle(&NSString::from_str("과목 자료 발췌로 비교 답변 생성…"));
        self.restore_model_selection();
        self.fill_efforts();
    }
    unsafe fn restore_model_selection(&self) {
        let index = self.model_items.iter().position(|v| v["provider"] == self.selected && v["id"] == self.selected_model());
        self.model.selectItemAtIndex(index.map(|i| i as isize).unwrap_or(-1));
        let title = index.map(|i| strv(&self.model_items[i], "title")).unwrap_or_else(|| {
            if self.models_rx.is_some() { "모델 목록 불러오는 중…" } else { "버전 모델 선택" }.into()
        });
        let cell: Retained<NSPopUpButtonCell> = msg_send![&self.model,cell];
        *self.model_title.borrow_mut() = title.clone();
        self.model_display.setTitle(&NSString::from_str(&title));
        cell.setMenuItem(Some(&self.model_display));
        theme::chat_status_dot_state(&self.model_dot, index.is_some_and(|i| self.model_items[i]["available"] == true));
        self.layout_controls();
    }
    /// 모델·effort 칩은 글 길이에 맞추고, 좁은 패널에서는 모델 이름을 먼저 줄인다.
    unsafe fn layout_controls(&self) {
        let w = self.view.frame().size.width;
        let measure = |title: &str| text_label(title, rect(0., 0., 10., 10.), 11.5, false).intrinsicContentSize().width;
        let effort_title = self.effort.titleOfSelectedItem().map(|t| t.to_string()).unwrap_or_else(|| "기본".into());
        let title = self.model_title.borrow().clone();
        let (effort_width, model_width) = chip_widths(w, measure(&title), measure(&effort_title));
        // 칩이 줄어들면 이름 끝을 말줄임표로 자른다. 전체 이름은 도움말에 남는다.
        let mut shown = title.clone();
        while shown.chars().count() > 1 && measure(&format!("{shown}…")) + CHIP_MODEL_PADDING > model_width && measure(&title) + CHIP_MODEL_PADDING > model_width {
            shown.pop();
        }
        if shown != title { shown.push('…'); }
        self.model_display.setTitle(&NSString::from_str(&shown));
        let size = theme::CHAT_CONTROL_FACE_SIZE;
        self.attach.setFrame(rect(20., 39. + (28. - size) / 2., size, size));
        self.model_chip.setFrame(rect(20. + ATTACH_SPACE, 39., model_width, 28.));
        self.model_dot.setFrame(rect(7., 10.5, 7., 7.));
        self.model.setFrame(rect(12., 0., model_width - 14., 28.));
        self.chevrons[0].setFrame(rect(model_width - 17., 9., 10., 10.));
        let x = 20. + ATTACH_SPACE + model_width + 2.;
        self.effort_chip.setFrame(rect(x, 39., effort_width, 28.));
        self.effort.setFrame(rect(4., 0., effort_width - 6., 28.));
        self.chevrons[1].setFrame(rect(effort_width - 17., 9., 10., 10.));
        // 모델·effort의 22pt 호버 면에 크기를 맞추고 같은 행 중심에 놓는다.
        let row = self.model_chip.frame();
        self.send.setFrame(rect(w - 22. - size, row.origin.y + (row.size.height - size) / 2., size, size));
    }

    fn effort_key(&self) -> String { format!("{}.{}", self.selected, self.selected_model()) }
    fn selected_effort(&self) -> &str {
        self.selected_efforts.get(&self.effort_key()).map(String::as_str).unwrap_or("")
    }
    unsafe fn fill_efforts(&mut self) {
        let item = self.model_items.iter().find(|m|m["provider"]==self.selected && m["id"]==self.selected_model());
        self.effort_items = vec![String::new()];
        self.effort_items.extend(item.and_then(|m|m["efforts"].as_array()).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned));
        let selected = self.selected_effort().to_owned();
        let supported = self.effort_items.contains(&selected);
        if !supported { self.effort_items.push(selected.clone()); }
        self.effort.removeAllItems();
        for value in &self.effort_items {
            let title = if value.is_empty() { "기본".to_owned() } else {
                let label = match value.as_str() {"none"=>"None","minimal"=>"Minimal","low"=>"Low","medium"=>"Medium","high"=>"High","xhigh"=>"XHigh","max"=>"Max","ultra"=>"Ultra",_=>value};
                format!("{label}{}",if !supported && *value==selected {" · 지원 확인 필요"}else{""})
            };
            self.effort.addItemWithTitle(&NSString::from_str(&title));
        }
        self.effort.selectItemAtIndex(self.effort_items.iter().position(|e|*e==selected).unwrap_or(0) as isize);
        self.layout_controls();
    }
    unsafe fn refresh_models(&mut self) {
        self.refresh_provider_models(&["claude", "codex", "antigravity"]);
    }
    unsafe fn refresh_provider_models(&mut self, providers: &[&'static str]) {
        let (tx, rx) = mpsc::channel();
        self.models_rx = Some(rx);
        self.catalogs_pending = providers.len();
        for &provider in providers {
            self.catalog_errors.insert(provider.into(), "목록 확인 중".into());
            let tx = tx.clone();
            std::thread::spawn(move || { let _ = tx.send((provider.into(), rpc(json!({"op":"chat.models","provider":provider})))); });
        }
        self.fill_models();
    }
    pub fn connection(&mut self, info: Option<&Value>) {
        let info = info.cloned().unwrap_or(Value::Null);
        if info != self.provider_info {
            // A login can finish while the pre-login model request is still
            // running. Queue a fresh request rather than losing that transition.
            self.model_refresh_queue.observe(&self.provider_info, &info);
            self.provider_info = info;
            unsafe { self.fill_models(); }
        }
        self.connection = self.provider_info["providers"].as_array()
            .and_then(|a|a.iter().find(|v|v["provider"]==self.selected))
            .map(|v| if v["installed"] != true {"CLI 설치 필요 · 왼쪽 구독 계정 연결"} else if v["logged_in"] == true {""} else {"로그인 필요 · 왼쪽 구독 계정 연결"})
            .unwrap_or("연결 확인 중").into();
    }
    pub fn busy(&self) -> bool {
        self.running.is_some()
    }
    /// 연결된 공급자의 CLI가 목록을 못 주었거나 채팅 제한을 걸 수 없을 때 모델 메뉴에 보일 이유.
    fn cli_help(&self) -> HashMap<String, String> {
        let connected = |provider: &str| self.provider_info["providers"].as_array()
            .is_some_and(|a| a.iter().any(|p| p["provider"] == provider && p["installed"] == true && p["logged_in"] == true));
        let mut help = HashMap::new();
        for provider in ["claude", "codex", "antigravity"] {
            if !connected(provider) { continue; }
            let text = match self.catalog_errors.get(provider) {
                Some(e) if e == "목록 확인 중" => continue,
                Some(e) => format!("모델 목록 확인 실패 · {e}"),
                None => match self.chat_blocked.get(provider) { Some(reason) => reason.clone(), None => continue },
            };
            help.insert(provider.to_owned(), text);
        }
        help
    }
    fn request(&self, op: &str) -> Value {
        json!({"op":op,"workspace_id":self.workspace,"provider":self.selected})
    }
    pub fn take_notes_changed(&mut self) -> bool {
        std::mem::take(&mut self.notes_changed)
    }
    /// 정리 대상을 가려 제안함에 올린다. 처음 강한 신호가 잡히면 한 번 묻는다.
    unsafe fn judge(&mut self, op: &str, workspace: &str, provider: &str) {
        match rpc(json!({"op":op,"workspace_id":workspace,"provider":provider})) {
            Ok(status) => {
                self.notes_changed = true;
                if op == "distill.scan" && status["consent_needed"] == true {
                    self.consent(workspace, provider);
                }
            }
            Err(e) => self.error = format!("대화 정리 확인 실패 · {e}"),
        }
    }
    unsafe fn consent(&mut self, workspace: &str, provider: &str) {
        let name = match provider { "codex" => "Codex", "claude" => "Claude", _ => "Antigravity" };
        let alert = NSAlert::new(mtm());
        alert.setMessageText(&NSString::from_str("이 대화를 노트로 정리해 둘까요?"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "여러 번 물었거나 표시해 둔 답을 다시 보기 좋은 노트 초안으로 정리할 수 있어요.\n\n정리할 때 그 질문과 답이 {name}에 한 번 더 전송되고 구독 사용량을 써요. 초안은 자료 탭의 정리 제안에 모이고, 등록하기 전에는 자료에 들어가지 않아요.\n\n왼쪽 ‘AI 정리’에서 언제든 바꿀 수 있어요."
        )));
        for title in ["이번만 정리", "앞으로 중요한 것만 알아서", "괜찮아요"] {
            alert.addButtonWithTitle(&NSString::from_str(title));
        }
        let choice = match alert.runModal() { 1000 => "once", 1001 => "auto", _ => "no" };
        match rpc(json!({"op":"distill.consent","workspace_id":workspace,"provider":provider,"choice":choice})) {
            Ok(_) => {
                self.notes_changed = true;
                if choice != "no" {
                    self.flash = Some(("노트 초안을 만드는 중이에요 · 자료 탭 ✦ 정리 제안에 모여요".into(), Instant::now()));
                }
            }
            Err(e) => self.error = format!("대화 정리 설정 실패 · {e}"),
        }
    }
    /// 답 아래 정리 버튼이 보여 줄 표시와 진행 상태를 다시 읽는다. 대화 화면은 그대로 둔다.
    pub unsafe fn sync_notes(&mut self) {
        if self.workspace.is_none() || self.busy() { return; }
        let Ok(fresh) = rpc(self.request("chat.get")) else { return };
        let Some(turns) = self.data["turns"].as_array_mut() else { return };
        for turn in turns {
            if let Some(found) = fresh["turns"].as_array().and_then(|all| all.iter().find(|t| t["id"] == turn["id"])) {
                turn["mark"] = found["mark"].clone();
                turn["distill"] = found["distill"].clone();
            }
        }
    }
    pub unsafe fn save_draft(&mut self) -> Result<(), String> {
        if self.workspace.is_none() || self.running.as_ref().is_some_and(|r| r.pending_in(self.workspace.as_deref(), &self.selected)) {
            return Ok(());
        }
        let draft = self.input.string().to_string();
        if draft == self.last_draft {
            return Ok(());
        }
        let mut r = self.request("chat.draft");
        r["draft"] = json!(draft);
        rpc(r)?;
        self.last_draft = draft;
        self.searched = None;
        if self.error.starts_with("초안 저장 실패") {
            self.error.clear();
        }
        Ok(())
    }
    /// The conversation scroll position for a workspace, to keep across launches.
    pub unsafe fn position_for(&self, ws: &str) -> Option<NSPoint> {
        if self.workspace.as_deref() == Some(ws) {
            return Some(self.messages.position());
        }
        self.positions.get(&format!("{:?}/{}", Some(ws), self.selected)).copied()
    }
    pub fn seed_position(&mut self, ws: &str, position: NSPoint) {
        self.positions.entry(format!("{:?}/{}", Some(ws), self.selected)).or_insert(position);
    }
    pub unsafe fn forget_workspace(&mut self) {
        self.quiet_since = None;
        self.workspace = None;
        self.load();
    }
    pub unsafe fn context(&mut self, ws: Option<&str>) {
        if self.workspace.as_deref() == ws {
            return;
        }
        if let Err(e) = self.save_draft() {
            self.error = format!("초안 저장 실패 · {e}");
            return;
        }
        self.positions.insert(
            format!("{:?}/{}", self.workspace, self.selected),
            self.messages.position(),
        );
        // 과목을 옮기면 떠나는 대화의 정리 대상을 뒤에서 정리하게 넘긴다.
        if let Some(old) = self.workspace.clone() {
            if self.quiet_since.take().is_some() {
                let provider = self.selected.clone();
                self.judge("distill.flush", &old, &provider);
            }
        }
        self.workspace = ws.map(str::to_owned);
        self.load();
    }
    unsafe fn load(&mut self) {
        self.searched = None;
        self.matches.clear();
        self.close_history(false);
        self.history_query.clear();
        self.search.setStringValue(&NSString::from_str(""));
        self.error.clear();
        self.data = Value::Null;
        if self.workspace.is_some() {
            match rpc(self.request("chat.get")) {
                Ok(d) => self.data = d,
                Err(e) => self.error = e,
            }
        }
        self.last_draft = strv(&self.data, "draft");
        let pending = self.running.as_ref().is_some_and(|r| r.pending_in(self.workspace.as_deref(), &self.selected));
        self.input.setString(&NSString::from_str(if pending { "" } else { &self.last_draft }));
        // 붙인 노트는 그 과목의 질문에만 간다.
        self.attached = None;
        self.note_chip.close_preview();
        self.messages.clear();
        self.render();
        if let Some(pos) = self
            .positions
            .get(&format!("{:?}/{}", self.workspace, self.selected))
            .copied()
        {
            self.messages.restore(pos);
        } else {
            self.messages.bottom();
        }
        self.bottom.setHidden(true);
    }
    pub unsafe fn layout(&mut self, w: f64, h: f64) {
        let width_changed = self.size.0 != w;
        self.size = (w, h);
        if width_changed && self.history_open { self.searched = None; }
        self.view.setFrame(rect(0., 0., w, h));
        // 첨부 썸네일 줄은 대화 영역을 밀지 않고 그 위에 겹친다. 겹치는 아래쪽만
        // 대화가 투명하게 사라지도록 그라데이션 마스크를 건다.
        let chip = if self.note_chip.visible() { chat_note::CHIP_ROW } else { 0. };
        let strip = chip + if self.attachments.is_empty() { 0. } else { chat_attachments::STRIP_HEIGHT + 6. };
        let grow = INPUT_GROW.with(Cell::get).clamp(0., INPUT_GROW_MAX.min((h - 282.).max(0.)));
        // 입력창 바로 위에 노트 칩, 그 위에 이미지 썸네일 줄이 놓인다.
        self.note_chip.frame(w, 142. + grow, h - 76.);
        self.strip.frame(rect(16., 142. + grow + chip, w - 32., chat_attachments::STRIP_HEIGHT));
        // 아래에서 위로: 안내 문구 · 유리 캡슐(조작 줄 + 입력란) · 대화 · 상태 줄.
        self.layout_controls();
        self.new.setFrame(rect(w - 94., h - 38., 80., 28.));
        self.history.setFrame(rect(14., h - 38., 28., 28.));
        self.search.setFrame(rect(48., h - 36., (w - 148.).max(0.), 24.));
        self.set_history_frame(false);
        self.status.setFrame(rect(18., h - 69., w - 36., 28.));
        // 대화 바닥은 입력창 윗선(138)에서 6pt 위다.
        self.messages.frame(rect(0., MESSAGES_BOTTOM + grow, w, (h - 70. - MESSAGES_BOTTOM - grow).max(80.)));
        self.bottom_blur.setFrame(rect(0., 138. + grow, w, BOTTOM_BLUR_HEIGHT));
        layout_bottom_blur(&self.bottom_blur, w);
        self.bottom_blur.setHidden(self.messages.at_bottom());
        self.shade_messages_bottom();
        self.notice.setFrame(rect(18., 10., w - 36., 16.));
        self.composer.setFrame(rect(12., 34., w - 24., 104. + grow));
        self.input_scroll.setFrame(rect(18., 86., w - 36., 42. + grow));
        self.resize_handle.setFrame(rect(36., 130. + grow, (w - 72.).max(0.), 10.));
        self.placeholder.setFrame(rect(27., 106. + grow, w - 54., 18.));
        self.bottom.setFrame(rect(w - 98., 146. + grow + strip, 82., 26.));
        self.pause_run.setFrame(rect(16., 146. + grow + strip, 168., 26.));
        self.retry_save.setFrame(rect(if self.waiting.is_some() { 190. } else { 16. }, 146. + grow + strip, 100., 26.));
    }
    /// 대화는 입력창 쪽 바닥에서 딱 잘리지 않고 흐려지며 사라진다. 썸네일 줄이 있으면 그 뒤까지.
    /// 맨 아래까지 내렸으면(흐림 띠가 꺼져 있으면) 마지막 말풍선의 버튼이 또렷하도록 바닥 페이드도 걷는다.
    /// 노트 칩은 그 위에 그대로 떠 있다.
    unsafe fn shade_messages_bottom(&self) {
        let chip = if self.note_chip.visible() { chat_note::CHIP_ROW } else { 0. };
        let thumbs = if self.attachments.is_empty() { 0. } else { 142. + chip + chat_attachments::STRIP_HEIGHT - MESSAGES_BOTTOM };
        let edge = if self.bottom_blur.isHidden() { 0. } else { 6. };
        self.messages.fade_bottom(thumbs.max(edge));
    }
    /// 스크롤이 맨 아래에 닿거나 떠날 때 흐림 띠를 끄고 켠다.
    /// 맨 아래면 새 답변을 이미 보고 있으니 "새 답변" 버튼도 걷어 마지막 말풍선 버튼을 가리지 않게 한다.
    unsafe fn track_bottom_blur(&self) {
        let at_bottom = self.messages.at_bottom();
        if at_bottom && !self.bottom.isHidden() { self.bottom.setHidden(true); }
        if self.bottom_blur.isHidden() == at_bottom { return; }
        self.bottom_blur.setHidden(at_bottom);
        self.shade_messages_bottom();
    }
    unsafe fn add_attachments(&mut self, items: Vec<Result<chat_attachments::Attachment, String>>) {
        if items.is_empty() { return; }
        let mut errors = Vec::new();
        for item in items {
            match item {
                Ok(a) if self.attachments.len() < chat_attachments::MAX_IMAGES => self.attachments.push(a),
                Ok(_) => { errors.push(format!("이미지는 한 번에 {}장까지 첨부할 수 있어요", chat_attachments::MAX_IMAGES)); }
                Err(e) => errors.push(e),
            }
        }
        errors.dedup();
        if let Some(e) = errors.first() { self.error = format!("이미지 첨부 실패 · {e}"); }
        else if self.error.starts_with("이미지 첨부 실패") { self.error.clear(); }
        self.attachments_changed();
        self.strip.reveal_last();
    }
    unsafe fn attachments_changed(&mut self) {
        self.strip.rebuild(&self.attachments, &self._target);
        let (w, h) = self.size;
        self.layout(w, h);
    }
    unsafe fn render(&mut self) {
        let notice = if self.data["source_excerpts"] == true {
            "자료 비교 시 관련 발췌도 전송돼요 · ⌘ + Enter 전송"
        } else {
            "질문과 직접 첨부한 내용만 전송돼요 · ⌘ + Enter 전송"
        };
        if self.notice.stringValue().to_string() != notice {
            self.notice.setStringValue(&NSString::from_str(notice));
        }

        let pending = self.running.as_ref()
            .filter(|r| r.pending_in(self.workspace.as_deref(), &self.selected) && !r.terminal)
            .map(|r| &r.pending);
        if self.messages.update(&self.data, pending, &self.selected, &self._target)
        {
            self.bottom.setHidden(false);
        }
        if self.scroll_to_bottom {
            self.messages.bottom();
            self.bottom.setHidden(true);
            self.scroll_to_bottom = false;
        }
        self.render_note_chip();
        self.placeholder
            .setHidden(!self.input.string().to_string().is_empty());
        let marked: bool = msg_send![&self.input, hasMarkedText];
        let can_send = self.busy() || (
            self.workspace.is_some() && self.unsaved.is_none()
                && versioned_model(&self.selected, self.selected_model())
                && !self.chat_blocked.contains_key(&self.selected)
                && self.data["needs_new"] != true
                && (!self.input.string().to_string().trim().is_empty() || marked || !self.attachments.is_empty()));
        self.send.setEnabled(can_send);
        if self.send_enabled != Some(can_send) {
            theme::chat_send(&self.send, can_send);
            self.send_enabled = Some(can_send);
        }
        if self.send_is_stopping != Some(self.busy()) {
            send_button_state(&self.send, self.busy());
            self.send_is_stopping = Some(self.busy());
        }
        self.retry_save.setHidden(self.unsaved.is_none());
        self.pause_run.setHidden(self.waiting.is_none());
        self.model.setEnabled(!self.busy() && self.models_rx.is_none());
        self.effort.setEnabled(!self.busy() && self.models_rx.is_none() && self.effort_items.len()>1);
        let can_switch = !self.busy() && self.unsaved.is_none() && self.workspace.is_some();
        self.new.setEnabled(can_switch);
        self.history.setEnabled(can_switch || self.history_open);
        self.attach.setEnabled(self.running.as_ref().is_none_or(|r| r.accepted));
        self.input.setEditable(
            self.workspace.is_some() && self.running.as_ref().is_none_or(|r| r.accepted),
        );
        if self.flash.as_ref().is_some_and(|(_, at)| at.elapsed() > Duration::from_secs(8)) { self.flash = None; }
        let message = if !self.error.is_empty() {
            self.error.clone()
        } else if let Some((flash, _)) = &self.flash {
            flash.clone()
        } else if self.unsaved.is_some() {
            "답변 저장 실패 · 저장 재시도를 눌러 주세요".into()
        } else if self.workspace.is_none() {
            "과목을 선택하면 대화를 시작할 수 있어요".into()
        } else if self.data["needs_new"] == true {
            "이전 CLI 세션을 이어갈 수 없어요. 새 대화를 시작하세요.".into()
        } else if let Some(r) = &self.running {
            if self.workspace.as_deref() != Some(&r.workspace) || self.selected != r.provider {
                "다른 과목·공급자에서 답변 중 · 돌아가면 대화를 볼 수 있어요".into()
            } else if r.accepted {
                if r.answer.is_empty() {
                    String::new()
                } else {
                    "답변 작성 중…".into()
                }
            } else if self.waiting.is_some() {
                "과목 정리의 지금 단계가 끝나면 바로 답변해요".into()
            } else {
                "연결 확인 중…".into()
            }
        } else {
            self.connection.clone()
        };
        // 평소 상태는 옅게, 오류와 저장 실패는 본문 색으로 또렷하게 보인다.
        let quiet = self.error.is_empty() && self.unsaved.is_none();
        let sheet_visible = self.history_open || self.history_close_at.is_some();
        self.status.setHidden(sheet_visible && quiet && message == self.connection);
        if self.status_shown.as_ref() != Some(&(message.clone(), quiet)) {
            status_text(&self.status, &message, quiet);
            self.status_shown = Some((message, quiet));
        }
    }
    unsafe fn send(&mut self) {
        let marked: bool = msg_send![&self.input, hasMarkedText];
        if marked || self.busy() || self.workspace.is_none() || self.unsaved.is_some()
            || !versioned_model(&self.selected, self.selected_model()) || self.chat_blocked.contains_key(&self.selected) {
            return;
        }
        let prompt = self.input.string().to_string();
        if prompt.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        if let Err(e) = self.save_draft() {
            self.error = e;
            return;
        }
        let child = Command::new(core_exe())
            .arg("--chat-stream")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                self.error = e.to_string();
                return;
            }
        };
        let mut input = child.stdin.take().unwrap();
        let output = child.stdout.take().unwrap();
        let context = self.attached.clone().unwrap_or(Value::Null);
        let request = json!({"workspace_id":self.workspace,"provider":self.selected,"model":self.selected_model(),"effort":self.selected_effort(),"text":prompt,
            "images":self.attachments.iter().map(|a|json!({"mime":a.mime,"data":a.data})).collect::<Vec<_>>(),
            "context":context});
        let (tx, rx) = mpsc::channel();
        let (stop, stop_requests) = mpsc::channel();
        let errors = tx.clone();
        std::thread::spawn(move || {
            if writeln!(input, "{request}").and_then(|_| input.flush()).is_err() {
                let _ = errors.send(json!({"event":"failed","error":"대화 요청을 접수하지 못했습니다"}));
                return;
            }
            if stop_requests.recv().is_ok() {
                let _ = writeln!(input, "{{\"op\":\"stop\"}}").and_then(|_| input.flush());
            }
        });
        std::thread::spawn(move || {
            for line in BufReader::new(output).lines() {
                let Ok(line) = line else { break };
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if tx.send(v).is_err() {
                        break;
                    }
                }
            }
            let _ = child.wait();
            let _ = tx.send(json!({"event":"process_exit"}));
        });
        self.error.clear();
        self.running = Some(Running {
            rx,
            stop,
            pending: json!({"id":"pending","user":prompt,"images":self.attachments.len(),"context":context,"answer":"","state":"running"}),
            workspace: self.workspace.clone().unwrap(),
            provider: self.selected.clone(),
            conversation: String::new(),
            turn: String::new(),
            run: String::new(),
            sequence: 0,
            accepted: false,
            terminal: false,
            answer: String::new(),
            prompt,
            images: self.attachments.len(),
            context,
        });
        self.input.setString(&NSString::from_str(""));
        self.scroll_to_bottom = true;
        self.render();
    }
    fn history_content_height(&self) -> f64 {
        if self.matches.is_empty() {
            HISTORY_EMPTY_HEIGHT
        } else {
            self.matches.len() as f64 * HISTORY_ROW_HEIGHT
        }
    }

    fn history_bottom_inset(&self) -> f64 {
        if !self.matches.is_empty() { HISTORY_HANDLE_HEIGHT } else { HISTORY_INSET }
    }

    fn history_height_bounds(&self) -> (f64, f64) {
        let h = self.size.1;
        let grow = INPUT_GROW.with(Cell::get).min((h - 282.).max(0.));
        let composer_top = MESSAGES_BOTTOM + grow + if self.attachments.is_empty() {
            0.
        } else {
            chat_attachments::STRIP_HEIGHT + 6.
        };
        let available = (h - HISTORY_TOP - composer_top).max(0.);
        let chrome = HISTORY_FADE_HEIGHT + self.history_bottom_inset();
        let content = self.history_content_height();
        // Fit only complete rows when the window or composer limits the height.
        let full_rows = ((available - chrome) / HISTORY_ROW_HEIGHT).floor().max(1.);
        let max = (content.min(full_rows * HISTORY_ROW_HEIGHT) + chrome).min(available);
        let min = (content.min(HISTORY_ROW_HEIGHT) + chrome).min(max);
        (min, max)
    }

    fn history_height_for(&self, requested: f64, min: f64, max: f64) -> f64 {
        let chrome = HISTORY_FADE_HEIGHT + self.history_bottom_inset();
        // Equal gaps above and below each card put the cutoff at its center.
        let rows = ((requested - chrome) / HISTORY_ROW_HEIGHT).round();
        (chrome + rows * HISTORY_ROW_HEIGHT).clamp(min, max)
    }

    unsafe fn layout_history_viewport(&self, frame: NSRect) {
        if self.history_viewport.frame() == frame {
            return;
        }
        self.history_viewport.setFrame(frame);
        self.history_viewport.setWantsLayer(true);
        let Some(layer) = self.history_viewport.layer() else { return };
        let Some(class) = AnyClass::get(c"CAGradientLayer") else { return };
        let mask: Retained<AnyObject> = msg_send![class, layer];
        let _: () = msg_send![&mask, setFrame: self.history_viewport.bounds()];
        let fade_end = (HISTORY_FADE_HEIGHT / frame.size.height.max(1.)).min(1.);
        let mut samples = Vec::new();
        let mut locations = Vec::new();
        for index in 0..=24 {
            let t = index as f64 / 24.;
            let alpha = t * t * (3. - 2. * t);
            samples.push(suggest_color(0., 0., 0., alpha).CGColor());
            locations.push(NSNumber::new_f64(t * fade_end));
        }
        if fade_end < 1. {
            samples.push(NSColor::blackColor().CGColor());
            locations.push(NSNumber::new_f64(1.));
        }
        let refs: Vec<_> = samples.iter().map(|color| {
            &*(Retained::as_ptr(color) as *const AnyObject)
        }).collect();
        let colors = NSArray::from_slice(&refs);
        let stops = NSArray::from_retained_slice(&locations);
        let flipped: bool = msg_send![&layer, contentsAreFlipped];
        let (top, bottom) = if flipped { (0., 1.) } else { (1., 0.) };
        let _: () = msg_send![&mask, setColors: &*colors];
        let _: () = msg_send![&mask, setLocations: &*stops];
        let _: () = msg_send![&mask, setStartPoint: NSPoint::new(0.5, top)];
        let _: () = msg_send![&mask, setEndPoint: NSPoint::new(0.5, bottom)];
        let _: () = msg_send![&layer, setMask: &*mask];
    }

    unsafe fn set_history_frame(&self, animate: bool) {
        let (w, h) = self.size;
        let width = (w - 24.).max(0.);
        let top = (h - HISTORY_TOP).max(0.);
        // The mask stays at the search row while the sheet slides behind it.
        self.layout_history_viewport(rect(12., 0., width, top));
        let current = self.drawer.frame();
        let mut frame = rect(0., top, width, current.size.height);
        if self.history_open {
            let (min, max) = self.history_height_bounds();
            let height = self.history_height.unwrap_or_else(|| self.history_height_for(HISTORY_DEFAULT_HEIGHT, min, max)).clamp((HISTORY_FADE_HEIGHT + self.history_bottom_inset()).min(max), max);
            let bottom = self.history_bottom_inset();
            self.history_handle.setHidden(self.matches.is_empty());
            let gutter = NSScroller::scrollerWidthForControlSize_scrollerStyle(NSControlSize::Regular, NSScrollerStyle::Legacy, mtm());
            self.list.setFrame(rect(
                gutter, bottom, (width - gutter).max(0.),
                (height - HISTORY_FADE_HEIGHT - bottom).max(0.),
            ));
            frame = rect(0., top - height, width, height);
        }
        if current == frame {
            return;
        }
        if animate && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            self.drawer.setFrameSize(frame.size);
            NSAnimationContext::beginGrouping();
            NSAnimationContext::currentContext().setDuration(HISTORY_SLIDE_SECONDS);
            let animator: Retained<AnyObject> = msg_send![&*self.drawer,animator];
            let _: () = msg_send![&animator,setFrameOrigin:frame.origin];
            NSAnimationContext::endGrouping();
        } else {
            self.drawer.setFrame(frame);
        }
    }

    unsafe fn close_history(&mut self, animate: bool) {
        let animate = animate && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        self.history_open = false;
        self.set_history_frame(animate);
        // A clipped NSView still exposes its rows to accessibility.
        self.history_close_at = animate.then(|| Instant::now() + Duration::from_secs_f64(HISTORY_SLIDE_SECONDS));
        if !animate {
            self.history_viewport.setHidden(true);
        }
    }

    unsafe fn refresh_history(&mut self, toggle_open: bool) {
        let term = self.search.stringValue().to_string();
        let query_changed = self.history_query != term;
        if query_changed {
            self.history_query.clone_from(&term);
        }
        if self.workspace.is_none() {
            return;
        }
        let opening = !self.history_open && (toggle_open || (query_changed && !term.is_empty()));
        if opening {
            self.history_open = true;
            self.history_close_at = None;
            self.history_viewport.setHidden(false);
        }
        if !self.history_open || (!opening && self.searched.as_ref() == Some(&term)) {
            return;
        }
        let mut request = self.request("chat.search");
        request["query"] = json!(term);
        self.searched = Some(term);
        self.matches.clear();
        match rpc(request) {
            Ok(rows) => self.matches = rows.as_array().cloned().unwrap_or_default(),
            Err(e) => self.error = e,
        }
        let gutter = NSScroller::scrollerWidthForControlSize_scrollerStyle(NSControlSize::Regular, NSScrollerStyle::Legacy, mtm());
        let width = (self.size.0 - 24. - 2. * gutter).max(80.);
        let height = self.history_content_height();
        let body = chat_messages::flipped();
        body.setFrame(rect(0., 0., width, height));
        if self.matches.is_empty() {
            let empty = text_label("대화 내역이 없습니다.", rect(12.,(height-20.)/2.,width-24.,20.),12.,false);
            empty.setTextColor(Some(&suggest_color(0.19,0.13,0.26,1.)));
            body.addSubview(&empty);
        }
        for (index,row) in self.matches.iter().enumerate() {
            let y = index as f64 * HISTORY_ROW_HEIGHT;
            let current = row["id"] == self.data["id"];
            let frame = rect(2.,y+2.,width-4.,50.);
            // 선택 배경은 구분선 사이 칸을 채우고, 구분선은 늘 그 위에 그린다.
            if current {
                body.addSubview(&theme::selection_wash(rect(0.,y,width,HISTORY_ROW_HEIGHT)));
            }
            if index > 0 {
                body.addSubview(&theme::soft_divider(rect(12.,y-0.5,width-24.,1.),true));
            }
            let card = view(frame);
            let title = strv(row,"title").split_whitespace().collect::<Vec<_>>().join(" ");
            let name = text_label(&title,rect(12.,25.,width-34.,18.),13.,true);
            name.setTextColor(Some(&suggest_color(0.19,0.13,0.26,1.)));
            name.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            if current {
                let badge = text_label("선택됨",rect((width-65.).max(12.),25.,48.,18.),10.5,true);
                name.setFrame(rect(12.,25.,(width-85.).max(1.),18.));
                name.setTextColor(Some(&suggest_color(0.23,0.23,0.65,1.)));
                badge.setTextColor(Some(&suggest_color(0.36,0.40,0.90,1.)));
                badge.setAlignment(NSTextAlignment::Right);
                card.addSubview(&badge);
            }
            card.addSubview(&name);
            let date:String = strv(row,"updated").chars().take(10).collect();
            let meta = format!("{}{}  ·  {date}",if current {"선택됨  ·  "} else {""},strv(row,"provider"));
            let detail = text_label(&strv(row,"provider"),rect(12.,5.,(width-112.).max(1.),17.),10.5,false);
            let timestamp = text_label(&date,rect(width-103.,5.,86.,17.),10.5,false);
            timestamp.setAlignment(NSTextAlignment::Right);
            timestamp.setTextColor(Some(&suggest_color(0.32,0.32,0.34,1.)));
            card.addSubview(&timestamp);
            detail.setTextColor(Some(&suggest_color(0.32,0.32,0.34,1.)));
            card.addSubview(&detail);
            let hit = NSButton::buttonWithTitle_target_action(&NSString::from_str(""),Some(&*self._target),Some(sel!(chatAction:)),mtm());
            hit.setBordered(false);
            hit.setFrame(card.bounds());
            hit.setTag(-1-index as isize);
            hit.setAccessibilityLabel(Some(&NSString::from_str(&format!("{title} · {meta}"))));
            card.addSubview(&hit);
            body.addSubview(&card);
        }
        self.list.setDocumentView(Some(&body));
        self.set_history_frame(opening);
        body.scrollPoint(NSPoint::ZERO);
    }

    unsafe fn switch_conversation(&mut self, index: Option<usize>) -> Result<(),String> {
        if self.busy() || self.unsaved.is_some() || self.workspace.is_none() { return Ok(()); }
        self.save_draft()?;
        if !self.attachments.is_empty() {
            return Err("대화를 전환하기 전에 첨부 이미지를 보내거나 제거하세요.".into());
        }
        if index.is_none() {
            if self.last_draft.is_empty() && self.data["turns"].as_array().is_none_or(Vec::is_empty) { return Ok(()); }
            let title: String = self.data["turns"][0]["user"].as_str().filter(|s|!s.is_empty()).unwrap_or(&self.last_draft).chars().take(50).collect();
            let alert = NSAlert::new(mtm());
            alert.setMessageText(&NSString::from_str("새 대화"));
            alert.setInformativeText(&NSString::from_str(&format!("「{title}」을 보관하면 대화 검색에서 이어갈 수 있어요.")));
            for label in ["보관하고 새 대화","취소","앱에서 삭제…"] { alert.addButtonWithTitle(&NSString::from_str(label)); }
            let disposition = match alert.runModal() {
                1000 => "archive",
                1002 if confirm_action("대화 삭제",&format!("「{title}」의 앱 기록과 초안을 삭제합니다. CLI 기록과 정리 제안·등록 노트는 유지됩니다."),"삭제하고 새 대화") => "delete",
                _ => return Ok(()),
            };
            let mut request = self.request("chat.new");
            request["disposition"] = json!(disposition);
            rpc(request)?;
            self.notes_changed = true;
        } else {
            let Some(row) = index.and_then(|i|self.matches.get(i)) else { return Ok(()); };
            if row["id"] == self.data["id"] {
                return Ok(());
            }
            let mut request = self.request("chat.open");
            request["provider"] = row["provider"].clone();
            request["id"] = row["id"].clone();
            rpc(request)?;
            self.selected = strv(row,"provider");
            self.restore_model_selection();
            self.fill_efforts();
        }
        // 목록에서 고른 경우에는 시트를 열어 둔 채 선택 표시만 옮긴다.
        let keep = index.is_some().then(|| (self.search.stringValue().to_string(), self.list.documentVisibleRect().origin.y));
        self.quiet_since = None;
        self.load();
        if let Some((term, scrolled)) = keep {
            self.reopen_history(&term, scrolled);
        }
        Ok(())
    }

    unsafe fn reopen_history(&mut self, term: &str, scrolled: f64) {
        self.search.setStringValue(&NSString::from_str(term));
        self.history_query = term.to_owned();
        self.history_open = true;
        self.history_close_at = None;
        self.history_viewport.setHidden(false);
        self.refresh_history(false);
        if let Some(document) = self.list.documentView() {
            document.scrollPoint(NSPoint::new(0., scrolled));
        }
    }
    pub unsafe fn tick(&mut self, ws: Option<&str>) -> bool {
        self.context(ws);
        self.track_bottom_blur();
        let mut results = Vec::new();
        if let Some(rx) = &self.models_rx { results.extend(rx.try_iter()); }
        if !results.is_empty() {
            for (provider,result) in results {
                self.catalogs_pending = self.catalogs_pending.saturating_sub(1);
                match result {
                    Ok(v)=>{
                        let mut models=v["models"].as_array().cloned().unwrap_or_default();
                        match v["blocked"].as_str() {
                            // 목록은 보여 주되 회색으로 두고, 전송 대신 이유를 알린다.
                            Some(reason)=>{for m in &mut models {m["available"]=json!(false);} self.chat_blocked.insert(provider.clone(),reason.to_owned());}
                            None=>{self.chat_blocked.remove(&provider);}
                        }
                        self.catalogs.insert(provider.clone(),models);self.catalog_errors.remove(&provider);
                    },
                    Err(e)=>{self.catalog_errors.insert(provider,e);}
                }
            }
            if self.catalogs_pending == 0 { self.models_rx = None; }
            self.fill_models();
        }
        let providers = self.model_refresh_queue.take_if_idle(self.models_rx.is_some());
        if !providers.is_empty() { self.refresh_provider_models(&providers); }
        for event in EVENTS.with(|e| std::mem::take(&mut *e.borrow_mut())) {
            match event{
   1=>self.send(),
   EVENT_PAUSE_RUN=>{if let Some(ws)=self.waiting.clone(){if let Err(e)=rpc(json!({"op":"organize.pause","workspace_id":ws})){self.error=e;}}},
   EVENT_RESIZE=>{let (w,h)=self.size;self.layout(w,h);},
   EVENT_INCOMING=>{let items=INCOMING.with(|q|std::mem::take(&mut *q.borrow_mut()));self.add_attachments(items);},
   EVENT_ATTACH=>{if self.running.as_ref().is_none_or(|r|r.accepted){let items=chat_attachments::choose();self.add_attachments(items);}},
   n if (chat_attachments::CLOSE_TAG as i64..chat_attachments::CLOSE_TAG as i64+100).contains(&n)=>{
       let i=(n-chat_attachments::CLOSE_TAG as i64) as usize;
       if i<self.attachments.len(){self.attachments.remove(i);self.attachments_changed();}
   },
   11=>{if self.busy(){if let Some(r)=&mut self.running {let _=r.stop.send(());}}else{self.send();}},
   9=>{
       let index = self.model.indexOfSelectedItem() as usize;
       if !self.busy() {
           if self.model_items.get(index).is_some_and(|m| m["helper"] == true) {
               self.restore_model_selection();
           } else if let Some(item) = self.model_items.get(index).cloned() {
               let provider = strv(&item,"provider");
               let model = strv(&item,"id");
               let result = self.save_draft().and_then(|_|rpc(json!({"op":"chat.preferences","provider":provider,"model":model})).map(|_|()));
               match result {
                   Ok(())=>{
                       let changed_provider = provider != self.selected;
                       self.positions.insert(format!("{:?}/{}",self.workspace,self.selected),self.messages.position());
                       self.selected=provider;
                       self.selected_models.insert(self.selected.clone(),model);
                       self.error.clear();
                       if changed_provider {self.load();}
                   },
                   Err(e)=>self.error=e,
               }
               self.restore_model_selection();
               self.fill_efforts();
           } else if index==self.model_items.len()+1 {
               self.restore_model_selection();
               if self.workspace.is_some(){
                   let alert=NSAlert::new(mtm());
                   let current=if self.data["source_excerpts"]==true{"켜짐"}else{"꺼짐"};
                   alert.setMessageText(&NSString::from_str(&format!("과목 자료 발췌로 비교 답변 생성 · {current}")));
                   alert.setInformativeText(&NSString::from_str("자료 비교를 요청하면 관련 자료 최대 3개의 제목과 발췌를 선택한 AI 공급자에게 보내 추가 답변을 만듭니다. 꺼도 로컬 자료 링크는 표시합니다."));
                   for title in ["켜기","끄기","취소"]{alert.addButtonWithTitle(&NSString::from_str(title));}
                   let choice=alert.runModal();
                   if choice==1000 || choice==1001{
                       let mut req=self.request("chat.source_preferences");req["enabled"]=json!(choice==1000);
                       match rpc(req){Ok(value)=>self.data["source_excerpts"]=value,Err(e)=>self.error=e}
                   }
               }
           } else {self.refresh_models();}
       } else {self.restore_model_selection();}
   },
   10=>{
       if !self.busy() {
           if let Some(effort) = self.effort_items.get(self.effort.indexOfSelectedItem() as usize).cloned() {
               match rpc(json!({"op":"chat.preferences","provider":self.selected,"model":self.selected_model(),"effort":effort})) {
                   Ok(_)=>{self.selected_efforts.insert(self.effort_key(),effort);self.error.clear();},
                   Err(e)=>self.error=e,
               }
           }
       }
       self.fill_efforts();
   },
   4=>{if let Err(error)=self.switch_conversation(None){self.error=error;}},
   EVENT_HISTORY=>{
       if self.history_open {
           self.close_history(true);
       } else {
           self.refresh_history(true);
       }
   },
   EVENT_HISTORY_RESIZE if self.history_open=>{
       let (_, max) = self.history_height_bounds();
       let chrome = HISTORY_FADE_HEIGHT + self.history_bottom_inset();
       let height = HISTORY_DRAG_HEIGHT.with(Cell::get).clamp(chrome.min(max), max);
       self.history_height = Some(height);
       self.set_history_frame(false);
   },
   EVENT_HISTORY_RESIZE_END if self.history_open=>{
       let (min, max) = self.history_height_bounds();
       let requested = HISTORY_DRAG_HEIGHT.with(Cell::get);
       let chrome = HISTORY_FADE_HEIGHT + self.history_bottom_inset();
       if !self.matches.is_empty() && requested < chrome + HISTORY_ROW_HEIGHT / 2. {
           self.history_height = None;
           self.close_history(true);
       } else {
           self.history_height = Some(self.history_height_for(requested, min, max));
           self.set_history_frame(false);
           if let Some(document) = self.list.documentView() {
               let y = self.list.documentVisibleRect().origin.y;
               document.scrollPoint(NSPoint::new(0., (y / HISTORY_ROW_HEIGHT).round() * HISTORY_ROW_HEIGHT));
           }
       }
   },
   n if n<0 && self.history_open=>{if let Err(error)=self.switch_conversation(Some((-1-n) as usize)){self.error=error;}},
   6=>{self.messages.bottom();self.bottom.setHidden(true);},
   chat_note::CHIP_TAG=>{if self.note_chip.preview_open(){self.note_chip.close_preview();}else{self.open_note_preview();}let (w,h)=self.size;self.layout(w,h);},
   chat_note::CHIP_CLOSE_TAG=>{self.attached=None;self.note_chip.close_preview();self.render_note_chip();},
   chat_note::PREVIEW_CLOSE_TAG=>{self.note_chip.close_preview();},
   8=>{if let Some(mut r)=self.unsaved.clone(){r["op"]=json!("chat.save");match rpc(r){Ok(_)=>{self.unsaved=None;self.error.clear();},Err(e)=>self.error=format!("저장 실패 · {e}")}}},
   n if (1000..2000).contains(&n)=>{
       let turn=self.data["turns"][n as usize-1000].clone();
       let paste=NSPasteboard::generalPasteboard();paste.clearContents();paste.setString_forType(&NSString::from_str(turn["answer"].as_str().unwrap_or("")),NSPasteboardTypeString);
       // 복사한 답은 약한 신호다. 제안함에 한 줄로만 남는다.
       if turn["state"]=="completed" && rpc(json!({"op":"distill.copied","turn_id":turn["id"]})).is_ok() {
           if let Some(ws)=self.workspace.clone(){let provider=self.selected.clone();self.judge("distill.scan",&ws,&provider);}
       }
   },
   n if (2000..3000).contains(&n)=>{if !self.busy(){let turn=self.data["turns"][n as usize-2000].clone();let prompt=strv(&turn,"user");if self.input.string().to_string().trim().is_empty(){
       // 다시 시도는 그 질문에 붙였던 노트도 다시 보낸다.
       if turn["context"].is_object(){self.attached=Some(turn["context"].clone());}
       self.input.setString(&NSString::from_str(&prompt));self.send();}else{self.error="작성 중인 질문을 먼저 전송하거나 비워 주세요.".into();}}},
   n if (chat_messages::NOTE_TAG as i64..chat_messages::NOTE_TAG as i64+3000).contains(&n)=>self.note_action(n-chat_messages::NOTE_TAG as i64),
   n if n >= chat_messages::SOURCE_TAG => {
       let index=(n-chat_messages::SOURCE_TAG) as usize;
       if let Some(source)=self.data["turns"].get(index/3).and_then(|turn|turn["followup"]["sources"].get(index%3)){
           if source["workspace_id"].as_str()==self.workspace.as_deref(){self.reveal=Some(source.clone());}
       }
   },
   n if (chat_messages::CONTEXT_TAG as i64..chat_messages::CONTEXT_TAG as i64+1000).contains(&n)=>{
       let context=self.data["turns"][(n-chat_messages::CONTEXT_TAG as i64) as usize]["context"].clone();
       if context.is_object(){self.reveal=Some(context);}
   },
   _=>{}}
        }
        self.refresh_history(false);
        if self.history_close_at.is_some_and(|at| Instant::now() >= at) {
            self.history_close_at = None;
            self.history_viewport.setHidden(true);
        }
        let mut clear_attachments = false;
        let mut finished = None;
        let mut events = Vec::new();
        if let Some(r) = &self.running {
            events.extend(r.rx.try_iter());
        }
        for e in events {
            let kind = e["event"].as_str().unwrap_or("");
            if kind == "waiting" {
                self.waiting = e["workspace_id"].as_str().map(str::to_owned);
                let (w, h) = self.size;
                self.layout(w, h);
                continue;
            }
            if self.waiting.is_some() && matches!(kind, "accepted" | "failed" | "process_exit") {
                self.waiting = None;
                let (w, h) = self.size;
                self.layout(w, h);
            }
            if kind == "process_exit" {
                if let Some(r) = self.running.take() {
                    if r.pending_in(self.workspace.as_deref(), &self.selected) {
                        self.input.setString(&NSString::from_str(&r.prompt));
                    }
                    if !r.terminal {
                        self.error="연결이 중단되었습니다. 전송 결과를 확인할 수 없어요. 자동 재전송하지 않았습니다.".into();
                    }
                }
                continue;
            }
            let Some(r) = &mut self.running else { continue };
            if kind == "accepted" {
                let mut request=e.clone();
                request["op"]=json!("chat.sources");
                let (sender,receiver)=mpsc::channel();
                self.source_jobs.push(receiver);
                std::thread::spawn(move || {
                    let turn=strv(&request,"turn_id");
                    if let Ok(result)=rpc(request){let _=sender.send((turn,result));}
                });
                r.accepted = true;
                clear_attachments = true;
                r.conversation = strv(&e, "conversation_id");
                r.turn = strv(&e, "turn_id");
                r.run = strv(&e, "run_id");
            }
            if r.terminal {
                continue;
            }
            if e["sequence"].is_number() {
                let seq = e["sequence"].as_u64().unwrap();
                if seq <= r.sequence || e["run_id"] != r.run {
                    continue;
                }
                r.sequence = seq;
            }
            if kind == "text_delta" {
                r.answer.push_str(&strv(&e, "text"));
            }
            if matches!(kind, "completed" | "interrupted" | "failed") {
                r.terminal = true;
                if kind == "completed" { finished = Some((r.workspace.clone(), r.provider.clone())); }
                if let Some(a) = e["answer"].as_str() {
                    r.answer = a.into();
                }
                self.error = strv(&e, "error");
            }
            let visible =
                self.workspace.as_deref() == Some(&r.workspace) && self.selected == r.provider;
            if e["save_failed"] == true {
                self.unsaved = Some(e.clone());
            }
            if visible && kind == "accepted" {
                self.input.setString(&NSString::from_str(""));
                self.last_draft.clear();
                if !self.data["turns"].is_array() {
                    self.data["turns"] = json!([])
                }
                let turns = self.data["turns"].as_array_mut().unwrap();
                if !turns.iter().any(|t| t["id"] == r.turn) {
                    turns.push(json!({"id":r.turn,"user":r.prompt,"images":r.images,"context":r.context,"answer":"","state":"running"}));
                }
                self.scroll_to_bottom = true;
            }
            if visible {
                if let Some(turns) = self.data["turns"].as_array_mut() {
                    if let Some(t) = turns.iter_mut().find(|t| t["id"] == r.turn) {
                        t["answer"] = json!(r.answer);
                        if r.terminal {
                            t["state"] = json!(kind);
                            t["error"] = e["error"].clone();
                        }
                    }
                }
            }
        }
        self.source_jobs.retain(|job| match job.try_recv() {
            Ok((id,result)) => {
                if let Some(turn)=self.data["turns"].as_array_mut().and_then(|all|all.iter_mut().find(|t|t["id"]==id)){
                    turn["followup"]=result;
                }
                false
            }
            Err(mpsc::TryRecvError::Empty)=>true,
            Err(mpsc::TryRecvError::Disconnected)=>false,
        });
        if clear_attachments && !self.attachments.is_empty() {
            self.attachments.clear();
            self.attachments_changed();
        }
        if clear_attachments && self.attached.take().is_some() {
            self.note_chip.close_preview();
        }
        // 답이 끝나면 앱 기록만으로 정리 대상을 가린다. 모델은 부르지 않는다.
        if let Some((workspace, provider)) = finished {
            self.searched = None;
            self.quiet_since = Some(Instant::now());
            self.judge("distill.scan", &workspace, &provider);
        }
        if !self.busy() && self.quiet_since.is_some_and(|at| at.elapsed() >= self.quiet_for) {
            self.quiet_since = None;
            if let Some(ws) = self.workspace.clone() {
                let provider = self.selected.clone();
                self.judge("distill.flush", &ws, &provider);
            }
        }
        let marked: bool = msg_send![&self.input, hasMarkedText];
        if !marked {
            if let Err(e) = self.save_draft() {
                self.error = format!("초안 저장 실패 · {e}");
            }
        }
        self.render();
        false
    }
    /// 답 아래의 나중에 정리(0대) · 노트로 정리(1000대) · 그대로 저장(2000대). `tag`는 `NOTE_TAG`를 뺀 값이다.
    unsafe fn note_action(&mut self, tag: i64) {
        let index = (tag % 1000) as usize;
        let turn = self.data["turns"][index].clone();
        let (Some(id), Some(ws)) = (turn["id"].as_str(), self.workspace.clone()) else { return };
        if turn["state"] != "completed" { return; }
        let provider = self.selected.clone();
        let result = match tag / 1000 {
            0 => {
                let mark = if turn["mark"] == "later" { "" } else { "later" };
                rpc(json!({"op":"distill.mark","turn_id":id,"mark":mark})).map(|_| {
                    self.data["turns"][index]["mark"] = json!(mark);
                    self.quiet_since = Some(Instant::now());
                    if mark.is_empty() { "표시를 지웠어요".to_owned() } else { "대화가 끝난 뒤 정리할 답으로 표시했어요".to_owned() }
                })
            }
            1 => rpc(json!({"op":"distill.now","turn_id":id})).map(|_| {
                self.data["turns"][index]["distill"] = json!("queued");
                "노트 초안을 만드는 중이에요 · 자료 탭 ✦ 정리 제안에 모여요".to_owned()
            }),
            _ => rpc(json!({"op":"distill.keep","turn_id":id})).map(|saved| {
                self.data["turns"][index]["distill"] = json!("kept");
                format!("‘{}’ 노트로 저장했어요 · 자료 탭 › 학습도우미 정리", strv(&saved["document"], "title"))
            }),
        };
        match result {
            Ok(message) => {
                self.error.clear();
                self.flash = Some((message, Instant::now()));
                self.notes_changed = true;
                // 표시는 다음 판단에서 반영된다. 처음이면 여기서 한 번 묻는다.
                if tag / 1000 == 0 { self.judge("distill.scan", &ws, &provider); }
            }
            Err(e) => self.error = e,
        }
    }
    /// 편집기에 열려 있는 노트를 알려 준다. 노트가 아니면 `None`.
    pub fn set_note(&mut self, note: Option<chat_note::NoteSource>) {
        self.note = note;
    }
    /// 이름표를 눌러 노트에서 다시 볼 문맥.
    pub fn take_reveal(&mut self) -> Option<Value> {
        self.reveal.take()
    }
    pub fn input(&self) -> Retained<NSTextView> {
        self.input.clone()
    }
    /// ⌘L·블록 메뉴: 지금 보고 있는 노트를 다음 질문에 붙인다.
    pub unsafe fn attach_note(&mut self) -> Result<(), String> {
        if self.workspace.is_none() { return Err("과목을 선택하면 노트를 붙일 수 있어요".into()); }
        if self.running.as_ref().is_some_and(|r| !r.accepted) { return Err("질문을 보내는 중이에요 · 잠시 뒤 다시 붙여 주세요".into()); }
        let note = self.note.as_ref().ok_or("노트를 열고 붙일 블록을 골라 주세요")?;
        self.attached = Some(note.context()?);
        if self.note_chip.preview_open() { self.open_note_preview(); }
        self.render_note_chip();
        Ok(())
    }
    /// 칩: 붙인 노트가 있으면 그것을, 없으면 보고 있는 노트를 흐리게 보여 준다.
    unsafe fn render_note_chip(&mut self) {
        let chip = if self.workspace.is_none() {
            None
        } else if let Some(a) = &self.attached {
            Some((format!("{} · {}", strv(a, "title"), strv(a, "label")), true))
        } else {
            // 제안 칩은 노트에서 블록을 골랐을 때만 보인다.
            self.note.as_ref().and_then(|n| n.label().map(|label| (format!("{} · {label}", n.title), false)))
        };
        let visible = self.note_chip.visible();
        let changed = self.note_chip.show(chip.clone()) | self.note_chip.tick();
        // 미리보기가 보고 있는 노트를 보여 주는 중이면 고른 블록이 바뀔 때 다시 채운다.
        if self.note_chip.preview_open() && self.attached.is_none() {
            match &chip {
                Some(_) => self.open_note_preview(),
                None => self.note_chip.close_preview(),
            }
        }
        if changed || visible != self.note_chip.visible() {
            let (w, h) = self.size;
            self.layout(w, h);
        }
    }
    /// 칩을 눌렀을 때: AI에게 보낼 원문 전체를 미리보기로 연다. 붙이지는 않는다.
    unsafe fn open_note_preview(&mut self) {
        let opened = if let Some(a) = &self.attached {
            let body = strv(a, "text");
            self.note_chip.open(&format!("a:{body}"), &body);
            true
        } else if let Some((note, label)) = self.note.as_ref().and_then(|n| n.label().map(|l| (n, l))) {
            let key = format!("s:{} · {label}", note.title);
            if self.note_chip.shown() != Some(key.as_str()) {
                match note.context() {
                    Ok(c) => self.note_chip.open(&key, &strv(&c, "text")),
                    Err(e) => self.note_chip.open(&key, &e),
                }
            }
            true
        } else {
            false
        };
        if opened {
            let (w, h) = self.size;
            self.layout(w, h);
        }
    }
    pub unsafe fn prepare_quit(&mut self) -> Result<bool, String> {
        self.save_draft()?;
        if let Some(r) = &mut self.running {
            let _ = r.stop.send(());
            return Ok(false);
        }
        if self.unsaved.is_some() || self.error.contains("저장 실패") {
            return Err(self.error.clone());
        }
        Ok(true)
    }
}

#[cfg(test)]
mod model_menu_tests {
    #[test]
    fn connection_refresh_survives_inflight_failure_without_repeating() {
        let offline = serde_json::json!({"providers":[{"provider":"codex","installed":false,"logged_in":false}]});
        let installed = serde_json::json!({"providers":[{"provider":"codex","installed":true,"logged_in":false}]});
        let online = serde_json::json!({"providers":[{"provider":"codex","installed":true,"logged_in":true}]});
        let mut queue = super::ModelRefreshQueue::default();
        queue.observe(&offline, &installed);
        assert!(queue.take_if_idle(false).is_empty());
        queue.observe(&installed, &online);
        assert!(queue.take_if_idle(true).is_empty());
        queue.observe(&online, &online);
        assert_eq!(queue.take_if_idle(false), vec!["codex"]);
        queue.observe(&online, &online);
        assert!(queue.take_if_idle(false).is_empty());
        queue.observe(&online, &installed);
        queue.observe(&installed, &online);
        assert_eq!(queue.take_if_idle(false), vec!["codex"]);
    }

    #[test]
    fn an_updated_cli_is_checked_again_without_logging_in_again() {
        let old = serde_json::json!({"providers":[{"provider":"claude","installed":true,"logged_in":true,"path":"/opt/homebrew/bin/claude","version":"2.1.108 (Claude Code)"}]});
        let new = serde_json::json!({"providers":[{"provider":"claude","installed":true,"logged_in":true,"path":"/opt/homebrew/bin/claude","version":"2.1.311 (Claude Code)"}]});
        let mut queue = super::ModelRefreshQueue::default();
        queue.observe(&old, &old);
        assert!(queue.take_if_idle(false).is_empty());
        queue.observe(&old, &new);
        assert_eq!(queue.take_if_idle(false), vec!["claude"]);
    }
    #[test]
    fn initial_connection_and_multiple_logins_refresh_only_connected_providers() {
        let online = serde_json::json!({"providers":[
            {"provider":"codex","installed":true,"logged_in":true},
            {"provider":"claude","installed":true,"logged_in":true},
            {"provider":"antigravity","installed":true,"logged_in":false}
        ]});
        let mut queue = super::ModelRefreshQueue::default();
        queue.observe(&serde_json::Value::Null, &online);
        queue.observe(&serde_json::Value::Null, &online);
        assert_eq!(queue.take_if_idle(false), vec!["claude", "codex"]);
        assert!(queue.take_if_idle(false).is_empty());
    }
    use super::*;
    #[test]
    fn antigravity_models_keep_cli_ids_and_account_status() {
        let catalogs=HashMap::from([("antigravity".into(),vec![json!({"id":"M123","label":"Gemini 4 Carbon","default":true,"efforts":["low","high"]})])]);
        let info=json!({"providers":[{"provider":"antigravity","installed":true,"logged_in":true}]});
        let items=combined_models(&catalogs,&HashMap::new(),&HashMap::new(),&info);
        assert_eq!(items.len(),1);
        assert_eq!(items[0]["title"],"Antigravity · Gemini 4 Carbon");
        assert_eq!(items[0]["id"],"M123");
        assert_eq!(items[0]["available"],true);
        let disconnected=combined_models(&catalogs,&HashMap::new(),&HashMap::new(),&Value::Null);
        assert_eq!(disconnected[0]["available"],false);
    }
    #[test]
    fn chips_hug_their_titles_and_the_model_chip_yields_first() {
        // 340pt 패널: 첨부 버튼 뒤 두 칩이 제 폭을 지키고 전송 버튼(w-54) 앞에서 끝난다.
        let (effort, model) = chip_widths(340., 110., 26.);
        assert_eq!((effort, model), (64., 160.));
        assert!(22. + ATTACH_SPACE + model + 6. + effort <= 340. - 54. - 6.);
        // 긴 모델 이름은 남은 자리까지만 차지한다.
        let (effort, model) = chip_widths(340., 400., 26.);
        assert_eq!(22. + ATTACH_SPACE + model + 6. + effort, 340. - 54. - 6.);
        // 넓은 패널에서도 칩은 글 길이를 넘어 늘어나지 않는다.
        assert_eq!(chip_widths(600., 120., 26.), (64., 170.));
    }
    #[test]
    fn combined_list_keeps_both_providers_and_requires_connection_for_green() {
        let catalogs=HashMap::from([
            ("claude".into(),vec![json!({"id":"claude-opus-4-6","label":"Opus 4.6"})]),
            ("codex".into(),vec![json!({"id":"gpt-6.1-terra","label":"GPT-6.1 Terra","default":true})]),
        ]);
        let info=json!({"providers":[{"provider":"claude","installed":true,"logged_in":false},{"provider":"codex","installed":true,"logged_in":true}]});
        let items=combined_models(&catalogs,&HashMap::new(),&HashMap::new(),&info);
        assert!(items.iter().any(|m|m["title"]=="Claude · Opus 4.6" && m["available"]==false));
        assert!(items.iter().any(|m|m["title"]=="Codex · GPT-6.1 Terra" && m["available"]==true));
        assert_eq!(items.len(),2);
        let unknown=combined_models(&catalogs,&HashMap::new(),&HashMap::new(),&Value::Null);
        assert!(unknown.iter().all(|m|m["available"]==false));
    }
    #[test]
    fn models_of_a_cli_that_cannot_chat_are_listed_in_gray() {
        // 코어가 blocked를 돌려주면 tick이 각 행을 available:false로 표시한다.
        let catalogs=HashMap::from([("codex".into(),vec![json!({"id":"gpt-6.1-terra","label":"Terra","available":false})])]);
        let info=json!({"providers":[{"provider":"codex","installed":true,"logged_in":true}]});
        let items=combined_models(&catalogs,&HashMap::new(),&HashMap::new(),&info);
        assert_eq!(items.len(),1);
        assert_eq!(items[0]["available"],false);
    }
    #[test]
    fn cli_help_sits_under_its_own_provider_and_wraps() {
        let items=vec![
            json!({"provider":"claude","id":"claude-sonnet-5-5"}),
            json!({"provider":"codex","id":"gpt-6.1-terra"}),
        ];
        let help=HashMap::from([("claude".into(),"Claude CLI 2.1.108 업데이트 필요 · 채팅 제한 옵션(--safe-mode)이 없어요. 업데이트 후 목록을 새로고침하세요.".to_owned())]);
        let rows=with_cli_help(items,&help);
        assert_eq!(rows.iter().map(|m|(m["provider"].as_str().unwrap(),m["helper"]==true)).collect::<Vec<_>>(),
            [("claude",false),("claude",true),("codex",false)]);
        assert_eq!(rows[1]["available"],false);
        let lines=cli_help_lines(rows[1]["title"].as_str().unwrap());
        assert!(lines.starts_with("Claude CLI 2.1.108 업데이트 필요\n"));
        assert!(lines.lines().all(|l|l.chars().count()<=40));
        assert!(lines.contains("(-\u{200C}-safe-mode)") && !lines.contains("--"));
        let failed=cli_help_lines("모델 목록 확인 실패 · Codex CLI 0.171.0 응답 전 종료(종료 코드 3) · Error: bad config");
        assert_eq!(failed.lines().collect::<Vec<_>>(),["모델 목록 확인 실패","Codex CLI 0.171.0 응답 전 종료(종료 코드 3)","Error: bad config"]);
        // 공급자 모델이 하나도 없어도 이유는 보인다.
        assert_eq!(with_cli_help(Vec::new(),&help).len(),1);
    }
    #[test]
    fn missing_saved_models_and_stale_catalogs_remain_visible_in_gray() {
        let catalogs=HashMap::from([("codex".into(),vec![json!({"id":"gpt-6.1-terra","label":"Terra","default":true})])]);
        let selected=HashMap::from([("codex".into(),"gpt-5.4".into())]);
        let errors=HashMap::from([("codex".into(),"offline".into())]);
        let info=json!({"providers":[{"provider":"codex","installed":true,"logged_in":true}]});
        let items=combined_models(&catalogs,&errors,&selected,&info);
        assert!(items.iter().any(|m|m["provider"]=="codex" && m["id"]=="gpt-5.4" && m["available"]==false));
        assert!(items.iter().all(|m|m["available"]==false));
    }
    #[test]
    fn aliases_and_defaults_are_hidden_and_resolve_to_exact_versions() {
        let claude = vec![
            json!({"id":"", "resolved_model":"claude-opus-4-6"}),
            json!({"id":"opus", "resolved_model":"claude-opus-4-6"}),
            json!({"id":"claude-opus-4-6", "label":"Opus 4.6"}),
            json!({"id":"claude-sonnet-latest", "label":"Sonnet"}),
        ];
        let codex = vec![json!({"id":"gpt-6.1-terra", "default":true, "label":"GPT-6.1 Terra"})];
        for alias in ["", "opus"] {
            assert_eq!(resolved_selection("claude", alias, &claude).unwrap()["id"], "claude-opus-4-6");
        }
        assert_eq!(resolved_selection("codex", "", &codex).unwrap()["id"], "gpt-6.1-terra");
        assert!(resolved_selection("claude", "sonnet", &claude).is_none());
        assert!(resolved_selection("claude", "claude-opus-4-5", &claude).is_none());
        let catalogs = HashMap::from([("claude".into(), claude), ("codex".into(), codex)]);
        let saved = HashMap::from([("claude".into(), "opus".into())]);
        let rows = combined_models(&catalogs, &HashMap::new(), &saved, &Value::Null);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|m| versioned_model(m["provider"].as_str().unwrap(), m["id"].as_str().unwrap())));
        assert!(combined_models(&HashMap::new(), &HashMap::new(), &saved, &Value::Null).is_empty());
    }

}
