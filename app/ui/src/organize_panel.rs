use crate::{font, mtm, rect, rpc, scrollbar, text_label, theme, view};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSBackingStoreType, NSButton, NSCell, NSColor,
    NSLineBreakMode, NSPopUpButton, NSScrollView, NSTextField, NSTextView, NSView,
    NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::VecDeque, sync::mpsc};

pub enum Event {
    Started(Value),
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
    static ACTIONS: RefCell<Vec<isize>> = const { RefCell::new(Vec::new()) };
    static EVENTS: RefCell<VecDeque<Event>> = const { RefCell::new(VecDeque::new()) };
}

const CANCEL: isize = 1;
const START: isize = 2;
const RESUME: isize = 3;
const PROVIDER: isize = 4;
const MODE: isize = 5;
const WIDTH: f64 = 504.;
const PROVIDERS: [(&str, &str); 3] = [
    ("codex", "Codex"),
    ("claude", "Claude"),
    ("antigravity", "Antigravity"),
];
const MODES: [(&str, &str); 3] = [
    ("auto", "알아서"),
    ("fast", "빠르게"),
    ("thorough", "꼼꼼하게"),
];

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct OrganizeTarget;
    unsafe impl NSObjectProtocol for OrganizeTarget {}
    impl OrganizeTarget {
        #[unsafe(method(organizeAction:))]
        fn action(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            ACTIONS.with(|actions| actions.borrow_mut().push(tag));
        }
    }
);

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    struct OrganizeContent;
    unsafe impl NSObjectProtocol for OrganizeContent {}
    impl OrganizeContent {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
    }
);

enum Reply {
    Plan(u64, Result<Value, String>),
    Started {
        request: u64,
        result: Result<Value, String>,
        saved_guidance: Option<String>,
    },
}

struct Panel {
    window: Retained<NSWindow>,
    _target: Retained<OrganizeTarget>,
    scroll: Retained<NSScrollView>,
    content: Retained<NSView>,
    provider_menu: Retained<NSPopUpButton>,
    mode_menu: Retained<NSPopUpButton>,
    guidance_scroll: Retained<NSScrollView>,
    guidance: Retained<NSTextView>,
    start: Retained<NSButton>,
    resume: Retained<NSButton>,
    status: Retained<NSTextField>,
    sender: mpsc::Sender<Reply>,
    receiver: mpsc::Receiver<Reply>,
    workspace_id: String,
    workspace_name: String,
    reread: Option<(String, String)>,
    provider: Option<String>,
    mode: Option<String>,
    plan: Option<Value>,
    plan_guidance: String,
    guidance_loaded: bool,
    request: u64,
    active: bool,
    loading: bool,
    ready: bool,
    starting: bool,
}

unsafe fn content(frame: NSRect) -> Retained<NSView> {
    let v: Retained<OrganizeContent> =
        msg_send![OrganizeContent::alloc(mtm()), initWithFrame: frame];
    Retained::into_super(v)
}

unsafe fn button(target: &OrganizeTarget, title: &str, tag: isize) -> Retained<NSButton> {
    let b = NSButton::buttonWithTitle_target_action(
        &NSString::from_str(title), Some(target), Some(sel!(organizeAction:)), mtm(),
    );
    b.setTag(tag);
    theme::button(&b, tag == START);
    b
}

unsafe fn popup(target: &OrganizeTarget, items: &[(&str, &str)], tag: isize) -> Retained<NSPopUpButton> {
    let p = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(mtm()), rect(0., 0., 210., 28.), false,
    );
    p.setFont(Some(&font::regular(13.)));
    for (_, title) in items {
        p.addItemWithTitle(&NSString::from_str(title));
    }
    p.setTag(tag);
    p.setTarget(Some(target));
    p.setAction(Some(sel!(organizeAction:)));
    p.setAccessibilityLabel(Some(&NSString::from_str(if tag == PROVIDER { "보낼 곳" } else { "정리 방식" })));
    p
}

unsafe fn paragraph(parent: &NSView, text: &str, y: &mut f64, size: f64, bold: bool, quiet: bool) {
    let label = text_label(text, rect(0., *y, WIDTH, 20.), size, bold);
    label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    label.setMaximumNumberOfLines(0);
    let cell: Retained<NSCell> = msg_send![&label, cell];
    cell.setWraps(true);
    cell.setScrollable(false);
    cell.setUsesSingleLineMode(false);
    let height = cell.cellSizeForBounds(rect(0., 0., WIDTH, 10000.)).height.ceil().max(size + 5.);
    label.setFrame(rect(0., *y, WIDTH, height));
    let color = if quiet { NSColor::secondaryLabelColor() } else { NSColor::labelColor() };
    label.setTextColor(Some(&color));
    parent.addSubview(&label);
    *y += height + 5.;
}

fn stage_summary(plan: &Value) -> String {
    [("extract", "자료 읽기"), ("align", "맞추기"), ("connect", "잇기"), ("review", "검수")]
        .into_iter()
        .filter_map(|(key, label)| {
            let tier = match plan["stages"][key]["tier"].as_str()? {
                "fast" => "빠름",
                "balanced" => "균형",
                "precise" => "정밀",
                _ => return None,
            };
            Some(format!("{label} {tier}"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

impl Panel {
    unsafe fn new() -> Self {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm()), rect(0., 0., 560., 620.),
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
            NSBackingStoreType::Buffered, false,
        );
        window.setTitle(&NSString::from_str("과목 정리"));
        window.setReleasedWhenClosed(false);
        theme::apply_window(&window);
        window.center();
        let target: Retained<OrganizeTarget> = msg_send![OrganizeTarget::alloc(mtm()), init];
        let root = view(rect(0., 0., 560., 620.));
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(24., 102., 528., 502.));
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scrollbar::style(&scroll);
        let content = content(rect(0., 0., WIDTH, 502.));
        scroll.setDocumentView(Some(&content));
        root.addSubview(&scroll);
        let provider_menu = popup(&target, &PROVIDERS, PROVIDER);
        let mode_menu = popup(&target, &MODES, MODE);
        let guidance_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., 0., WIDTH, 70.));
        guidance_scroll.setHasVerticalScroller(true);
        scrollbar::style(&guidance_scroll);
        let guidance = NSTextView::initWithFrame(NSTextView::alloc(mtm()), rect(0., 0., WIDTH - 18., 70.));
        guidance.setRichText(false);
        guidance.setImportsGraphics(false);
        guidance.setEditable(true);
        guidance.setSelectable(true);
        guidance.setAllowsUndo(true);
        guidance.setFont(Some(&font::regular(13.)));
        guidance.setTextColor(Some(&NSColor::labelColor()));
        guidance.setBackgroundColor(&NSColor::textBackgroundColor());
        guidance.setVerticallyResizable(true);
        guidance.setHorizontallyResizable(false);
        guidance.setMinSize(NSSize::new(0., 70.));
        guidance.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
        guidance.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        guidance.setTextContainerInset(NSSize::new(8., 8.));
        if let Some(container) = guidance.textContainer() {
            container.setWidthTracksTextView(true);
            container.setContainerSize(NSSize::new(WIDTH - 34., f64::MAX));
        }
        guidance.setAccessibilityLabel(Some(&NSString::from_str("이 과목의 정리 지침")));
        guidance_scroll.setDocumentView(Some(&guidance));
        let cancel = button(&target, "취소", CANCEL);
        cancel.setFrame(rect(318., 16., 80., 30.));
        cancel.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        root.addSubview(&cancel);
        let start = button(&target, "정리 시작", START);
        start.setFrame(rect(402., 16., 134., 30.));
        root.addSubview(&start);
        let resume = button(&target, "이어서 정리", RESUME);
        let status = text_label("", rect(24., 52., 512., 42.), 12., false);
        status.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        status.setMaximumNumberOfLines(2);
        root.addSubview(&status);
        window.setContentView(Some(&root));
        let (sender, receiver) = mpsc::channel();
        Self {
            window, _target: target, scroll, content, provider_menu, mode_menu,
            guidance_scroll, guidance, start, resume, status, sender, receiver,
            workspace_id: String::new(), workspace_name: String::new(), reread: None,
            provider: None, mode: None, plan: None, plan_guidance: String::new(),
            guidance_loaded: false, request: 0, active: false, loading: false,
            ready: false, starting: false,
        }
    }

    fn request_value(&self, op: &str) -> Value {
        let mut request = json!({"op": op, "workspace_id": self.workspace_id});
        if let Some(provider) = &self.provider {
            request["provider"] = json!(provider);
        }
        if let Some(mode) = &self.mode {
            request["mode"] = json!(mode);
        }
        if let Some((id, _)) = &self.reread {
            request["materials"] = json!([id]);
            request["reread"] = json!(true);
        }
        request
    }

    unsafe fn status(&self, text: &str, error: bool) {
        self.status.setStringValue(&NSString::from_str(text));
        let color = if error { NSColor::systemRedColor() } else { NSColor::secondaryLabelColor() };
        self.status.setTextColor(Some(&color));
        let tooltip = (!text.is_empty()).then(|| NSString::from_str(text));
        self.status.setToolTip(tooltip.as_deref());
    }

    unsafe fn update_enabled(&self) {
        let has_targets = self.plan.as_ref().is_some_and(|p| {
            p["targets"].as_u64().unwrap_or(0) > 0 || p["organised"].as_u64().unwrap_or(0) > 0
        });
        self.start.setEnabled(self.ready && !self.starting && has_targets);
        self.resume.setEnabled(self.ready && !self.starting);
        self.provider_menu.setEnabled(!self.starting);
        self.mode_menu.setEnabled(!self.starting);
        self.guidance.setEditable(self.guidance_loaded && !self.starting);
    }

    unsafe fn load_plan(&mut self) {
        self.request += 1;
        self.loading = true;
        self.ready = false;
        self.status("불러오는 중…", false);
        self.render();
        let request = self.request;
        let args = self.request_value("organize.plan");
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let _ = sender.send(Reply::Plan(request, rpc(args)));
        });
    }

    unsafe fn render(&self) {
        let origin = self.scroll.contentView().bounds().origin;
        for child in self.content.subviews().iter() {
            child.removeFromSuperview();
        }
        let mut y = 0.;
        let plan = self.plan.as_ref();
        if let Some(resumable) = plan.map(|p| &p["resumable"]).filter(|r| r.is_object()) {
            let box_view = content(rect(0., y, WIDTH, 64.));
            box_view.setWantsLayer(true);
            let layer: Retained<AnyObject> = msg_send![&box_view, layer];
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(0.95, 0.88, 0.65, 0.45);
            let _: () = msg_send![&layer, setBackgroundColor: &*color.CGColor()];
            let _: () = msg_send![&layer, setCornerRadius: 8.0f64];
            let message = if resumable["state"] == "limit" {
                "구독 사용 한도에 닿아 멈췄어요".to_owned()
            } else {
                format!("멈춘 정리가 있어요 — {}", resumable["progress"]["label"].as_str().unwrap_or("이어서 정리할 수 있어요"))
            };
            let label = text_label(&message, rect(10., 8., WIDTH - 20., 20.), 12., true);
            label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            label.setToolTip(Some(&NSString::from_str(&message)));
            box_view.addSubview(&label);
            self.resume.setFrame(rect(WIDTH - 128., 32., 118., 26.));
            box_view.addSubview(&self.resume);
            self.content.addSubview(&box_view);
            y += 74.;
        }
        paragraph(&self.content, &self.workspace_name, &mut y, 18., true, false);
        if let Some((_, title)) = &self.reread {
            paragraph(&self.content, &format!("‘{title}’ 꼼꼼하게 다시 정리"), &mut y, 14., true, false);
        }
        let summary = if let Some(p) = plan {
            let targets = p["targets"].as_u64().unwrap_or(0);
            if targets > 0 {
                format!("자료 {targets}개 · 약 {}쪽을 읽어요", p["pages"].as_u64().unwrap_or(0))
            } else if p["organised"].as_u64().unwrap_or(0) > 0 {
                "새로 읽을 자료는 없어요 · 개념 맞추기와 잇기만 다시 해요".into()
            } else {
                "정리할 자료가 없어요. 과목에 PDF·노트·이미지·PPTX 자료를 먼저 넣어 주세요.".into()
            }
        } else if self.loading {
            "불러오는 중…".into()
        } else {
            "정리 계획을 불러오지 못했어요.".into()
        };
        paragraph(&self.content, &summary, &mut y, 12., false, true);
        if let Some(materials) = plan.and_then(|p| p["materials"].as_array()).filter(|m| !m.is_empty()) {
            let height = (materials.len() as f64 * 24.).clamp(48., 108.);
            let list_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., y, WIDTH, height));
            list_scroll.setHasVerticalScroller(true);
            list_scroll.setDrawsBackground(false);
            scrollbar::style(&list_scroll);
            let list = content(rect(0., 0., WIDTH - 18., (materials.len() as f64 * 24.).max(height)));
            for (index, material) in materials.iter().enumerate() {
                let title = material["title"].as_str().unwrap_or("");
                let target = material["target"] == true;
                let detail = if let Some(reason) = material["unreadable"].as_str() {
                    format!("읽을 수 없어요: {reason}")
                } else if target {
                    let state = match material["state"].as_str() {
                        Some("changed") => "바뀐 자료",
                        Some("organised") => "다시 읽을 자료",
                        _ => "새 자료",
                    };
                    format!("{state} · {}쪽 · {}묶음", material["pages"].as_u64().unwrap_or(0), material["chunks"].as_u64().unwrap_or(0))
                } else if material["state"] == "organised" {
                    if material["quality"] == "low" { "정리 품질 낮음" } else { "정리됨" }.into()
                } else {
                    "이번 정리 대상이 아니에요".into()
                };
                let text = format!("{title} — {detail}");
                let label = text_label(&text, rect(0., index as f64 * 24., WIDTH - 22., 22.), 12., false);
                label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                label.setToolTip(Some(&NSString::from_str(&text)));
                let color = if target { NSColor::labelColor() } else { NSColor::secondaryLabelColor() };
                label.setTextColor(Some(&color));
                list.addSubview(&label);
            }
            list_scroll.setDocumentView(Some(&list));
            self.content.addSubview(&list_scroll);
            y += height + 10.;
        }
        paragraph(&self.content, "보낼 곳", &mut y, 13., true, false);
        self.provider_menu.setFrame(rect(0., y, 220., 28.));
        self.content.addSubview(&self.provider_menu);
        y += 33.;
        let provider_name = self.provider.as_deref().and_then(|id| PROVIDERS.iter().find(|(key, _)| *key == id)).map(|(_, name)| *name).unwrap_or("선택한 공급자");
        paragraph(&self.content, &format!("자료의 글과 쪽 이미지가 {provider_name}에 전송돼요. 구독 사용량을 쓰고 API 과금으로 바꾸지 않아요."), &mut y, 11., false, true);
        y += 5.;
        if self.reread.is_none() {
            paragraph(&self.content, "정리 방식", &mut y, 13., true, false);
            self.mode_menu.setFrame(rect(0., y, 220., 28.));
            self.content.addSubview(&self.mode_menu);
            y += 33.;
        }
        if let Some(p) = plan {
            paragraph(&self.content, &stage_summary(p), &mut y, 11., false, true);
            y += 5.;
            paragraph(&self.content, "예상 사용량", &mut y, 13., true, false);
            paragraph(&self.content, &format!("AI를 약 {}번 불러요 · {}", p["calls"]["total"].as_u64().unwrap_or(0), p["usage"].as_str().unwrap_or("")), &mut y, 12., false, false);
            if let Some(notes) = p["notes"].as_array() {
                for note in notes.iter().filter_map(Value::as_str) {
                    paragraph(&self.content, note, &mut y, 11., false, true);
                }
            }
        }
        y += 6.;
        paragraph(&self.content, "이 과목의 정리 지침 (선택)", &mut y, 13., true, false);
        self.guidance_scroll.setFrame(rect(0., y, WIDTH, 70.));
        self.content.addSubview(&self.guidance_scroll);
        y += 76.;
        paragraph(&self.content, "예: 정의는 교재 표현 그대로, 영어 약어는 풀어 쓰기", &mut y, 11., false, true);
        let height = (y + 8.).max(self.scroll.contentSize().height);
        self.content.setFrameSize(NSSize::new(WIDTH, height));
        let clip = self.scroll.contentView();
        clip.scrollToPoint(NSPoint::new(0., origin.y.min((height - clip.bounds().size.height).max(0.))));
        self.scroll.reflectScrolledClipView(&clip);
        self.update_enabled();
    }

    unsafe fn begin(&mut self, resume: bool) {
        if !self.ready || self.starting || (!resume && !self.start.isEnabled()) {
            return;
        }
        if resume && self.plan.as_ref().is_none_or(|p| !p["resumable"].is_object()) {
            return;
        }
        self.starting = true;
        self.status(if resume { "이어서 정리하는 중…" } else { "정리를 시작하는 중…" }, false);
        self.update_enabled();
        let args = if resume {
            json!({"op": "organize.resume", "workspace_id": self.workspace_id})
        } else {
            self.request_value("organize.start")
        };
        let guidance = self.guidance.string().to_string();
        let old_guidance = self.plan_guidance.clone();
        let workspace_id = self.workspace_id.clone();
        let request = self.request;
        let sender = self.sender.clone();
        std::thread::spawn(move || {
            let mut saved_guidance = None;
            let result = (|| {
                if guidance != old_guidance {
                    rpc(json!({"op": "ontology.guidance.set", "workspace_id": workspace_id, "text": guidance}))?;
                    saved_guidance = Some(guidance);
                }
                rpc(args)
            })();
            let _ = sender.send(Reply::Started { request, result, saved_guidance });
        });
    }

    unsafe fn hide(&mut self) {
        self.request += 1;
        self.active = false;
        self.loading = false;
        self.ready = false;
        self.starting = false;
        self.window.close();
    }

    unsafe fn poll(&mut self) {
        while let Ok(reply) = self.receiver.try_recv() {
            match reply {
                Reply::Plan(request, result) if request == self.request && self.active => {
                    self.loading = false;
                    match result {
                        Ok(plan) => {
                            self.provider = plan["provider"].as_str().map(str::to_owned);
                            self.mode = plan["mode"].as_str().map(str::to_owned);
                            if let Some(index) = PROVIDERS.iter().position(|(id, _)| Some(*id) == self.provider.as_deref()) {
                                self.provider_menu.selectItemAtIndex(index as isize);
                            }
                            if let Some(index) = MODES.iter().position(|(id, _)| Some(*id) == self.mode.as_deref()) {
                                self.mode_menu.selectItemAtIndex(index as isize);
                            }
                            self.plan_guidance = plan["guidance"].as_str().unwrap_or("").to_owned();
                            if !self.guidance_loaded {
                                self.guidance.setString(&NSString::from_str(&self.plan_guidance));
                                self.guidance_loaded = true;
                            }
                            self.plan = Some(plan);
                            self.ready = true;
                            self.status("", false);
                        }
                        Err(error) => {
                            self.ready = false;
                            self.status(&format!("정리 계획을 불러오지 못했어요: {error}"), true);
                        }
                    }
                    self.render();
                }
                Reply::Started { request, result, saved_guidance } => {
                    let current = request == self.request && self.active;
                    if current {
                        self.starting = false;
                        if let Some(guidance) = saved_guidance {
                            self.plan_guidance = guidance;
                        }
                    }
                    match result {
                        Ok(job) => {
                            // Closing or replacing the confirmation must not lose an accepted job.
                            EVENTS.with(|events| events.borrow_mut().push_back(Event::Started(job)));
                            if current { self.hide(); }
                        }
                        Err(error) if current => {
                            self.status(&format!("정리를 시작하지 못했어요: {error}"), true);
                            self.update_enabled();
                        }
                        Err(_) => (),
                    }
                }
                Reply::Plan(_, _) => (),
            }
        }
    }
}

pub unsafe fn open(workspace_id: &str, workspace_name: &str, reread: Option<(String, String)>) {
    ACTIONS.with(|actions| actions.borrow_mut().clear());
    PANEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let panel = slot.get_or_insert_with(|| Panel::new());
        panel.workspace_id = workspace_id.to_owned();
        panel.workspace_name = workspace_name.to_owned();
        panel.reread = reread;
        panel.provider = None;
        panel.mode = None;
        panel.plan = None;
        panel.plan_guidance.clear();
        panel.guidance_loaded = false;
        panel.guidance.setString(&NSString::from_str(""));
        let undo: Option<Retained<AnyObject>> = msg_send![&panel.guidance, undoManager];
        if let Some(undo) = undo {
            let _: () = msg_send![&undo, removeAllActions];
        }
        panel.provider_menu.selectItemAtIndex(0);
        panel.mode_menu.selectItemAtIndex(0);
        panel.start.setTitle(&NSString::from_str(if panel.reread.is_some() { "다시 정리 시작" } else { "정리 시작" }));
        theme::button(&panel.start, true);
        panel.active = true;
        panel.starting = false;
        panel.scroll.contentView().scrollToPoint(NSPoint::new(0., 0.));
        panel.load_plan();
        panel.window.makeKeyAndOrderFront(None);
    });
}

pub unsafe fn tick() {
    let actions = ACTIONS.with(|actions| std::mem::take(&mut *actions.borrow_mut()));
    PANEL.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(panel) = slot.as_mut() else { return };
        if panel.active && !panel.window.isVisible() {
            panel.hide();
        }
        for action in actions {
            if !panel.active { break; }
            match action {
                CANCEL => panel.hide(),
                START => panel.begin(false),
                RESUME => panel.begin(true),
                PROVIDER if !panel.starting => {
                    if let Some((id, _)) = PROVIDERS.get(panel.provider_menu.indexOfSelectedItem() as usize) {
                        panel.provider = Some((*id).to_owned());
                        panel.load_plan();
                    }
                }
                MODE if !panel.starting && panel.reread.is_none() => {
                    if let Some((id, _)) = MODES.get(panel.mode_menu.indexOfSelectedItem() as usize) {
                        panel.mode = Some((*id).to_owned());
                        panel.load_plan();
                    }
                }
                _ => (),
            }
        }
        panel.poll();
    });
}

pub fn take_event() -> Option<Event> {
    EVENTS.with(|events| events.borrow_mut().pop_front())
}

pub unsafe fn close() {
    ACTIONS.with(|actions| actions.borrow_mut().clear());
    PANEL.with(|slot| {
        if let Some(panel) = slot.borrow_mut().as_mut() {
            panel.hide();
        }
    });
}
