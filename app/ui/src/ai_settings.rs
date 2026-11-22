use crate::{confirm_action, font, mtm, rect, rpc, text_label, theme, view};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, AnyThread, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSApplication, NSBackingStoreType, NSButton, NSColor, NSImage, NSImageView, NSLineBreakMode, NSPopUpButton, NSScrollView,
    NSTextField, NSView, NSWindow, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSData, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoop, NSString, NSTimer, NSURL};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::sync::mpsc::{self, Receiver, TryRecvError};

// The helper content spans y=10..690 before its symmetric vertical inset.
const HELPER_VERTICAL_PADDING: f64 = 24.;
const HELPER_DOCUMENT_HEIGHT: f64 = 680. + HELPER_VERTICAL_PADDING * 2.;

const MODES: [&str; 3] = ["auto", "fast", "thorough"];
const DISTILL: [&str; 3] = ["off", "ask", "auto"];
const BACKGROUND: [&str; 3] = ["low", "normal", "high"];
const PRESETS: [&str; 6] = ["custom", "codex-plus", "codex-pro", "claude-pro", "claude-max", "claude-ultra"];
const STAGES: [(&str, &str); 5] = [
    ("extract", "자료 읽기"),
    ("align", "개념 맞추기"),
    ("connect", "개념 잇기"),
    ("review", "연결 검수"),
    ("distill", "대화 정리"),
];

thread_local! {
    static STATE: RefCell<Option<SettingsWindow>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Vec<(isize, isize)>> = const { RefCell::new(Vec::new()) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct AiSettingsTarget;
    unsafe impl NSObjectProtocol for AiSettingsTarget {}
    impl AiSettingsTarget {
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _window: &NSWindow) -> bool {
            unsafe { NSApplication::sharedApplication(mtm()).stopModal(); }
            false
        }
        #[unsafe(method(settingsTick:))]
        fn settings_tick(&self, _timer: &NSTimer) { unsafe { tick(); } }
        #[unsafe(method(aiSettingsAction:))]
        fn action(&self, sender: &AnyObject) {
            let tag: isize = unsafe { msg_send![sender, tag] };
            let selected = if tag == 8 {
                unsafe { msg_send![sender, selectedSegment] }
            } else if (1..=4).contains(&tag) || (100..105).contains(&tag) {
                unsafe { msg_send![sender, indexOfSelectedItem] }
            } else {
                -1
            };
            EVENTS.with(|events| events.borrow_mut().push((tag, selected)));
        }
    }
);

struct StageRow {
    popup: Retained<NSPopUpButton>,
    note: Retained<NSTextField>,
    models: Vec<(String, String)>,
}

struct Pending {
    provider: &'static str,
    mode: String,
    revision: u64,
    rx: Receiver<Result<Value, String>>,
}

struct SettingsWindow {
    window: Retained<NSWindow>,
    _target: Retained<AiSettingsTarget>,
    tabs: [Retained<NSButton>; 2],
    tab_selection: Retained<NSView>,
    scroll: Retained<NSScrollView>,
    document: Retained<NSView>,
    helpers: Retained<NSScrollView>,
    upper: Retained<NSView>,
    mode: Retained<NSPopUpButton>,
    mode_note: Retained<NSTextField>,
    distill: Retained<NSPopUpButton>,
    distill_note: Retained<NSTextField>,
    background: Retained<NSPopUpButton>,
    usage: Retained<NSTextField>,
    skills: Retained<NSTextField>,
    skills_error: Retained<NSTextField>,
    reset: Retained<NSButton>,
    provider: Retained<NSPopUpButton>,
    load: Retained<NSButton>,
    tiers_status: Retained<NSTextField>,
    stages: Vec<StageRow>,
    status: Retained<NSTextField>,
    status_error: bool,
    settings: Value,
    tiers: Option<Value>,
    pending: Option<Pending>,
    tiers_revision: u64,
}

unsafe fn label(parent: &NSView, title: &str, frame: NSRect, size: f64, bold: bool) -> Retained<NSTextField> {
    let label = text_label(title, frame, size, bold);
    let ink = if bold { theme::soft_selection_ink() } else { NSColor::secondaryLabelColor() };
    label.setTextColor(Some(&ink));
    label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    label.setMaximumNumberOfLines(0);
    if let Some(cell) = label.cell() {
        cell.setWraps(true);
        cell.setScrollable(false);
    }
    parent.addSubview(&label);
    label
}

unsafe fn popup(parent: &NSView, target: &AiSettingsTarget, tag: isize, title: &str, frame: NSRect, items: &[&str]) -> Retained<NSPopUpButton> {
    let popup = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm()), frame, false);
    popup.setFont(Some(&font::regular(12.)));
    popup.setBezelStyle(objc2_app_kit::NSBezelStyle::Push);
    popup.setContentTintColor(Some(&theme::soft_selection_ink()));
    popup.setTarget(Some(target));
    popup.setAction(Some(sel!(aiSettingsAction:)));
    popup.setTag(tag);
    for item in items {
        popup.addItemWithTitle(&NSString::from_str(item));
    }
    let _: () = msg_send![&popup, setAccessibilityLabel: &*NSString::from_str(title)];
    parent.addSubview(&popup);
    popup
}

unsafe fn button(parent: &NSView, target: &AiSettingsTarget, tag: isize, title: &str, frame: NSRect) -> Retained<NSButton> {
    let button = NSButton::buttonWithTitle_target_action(
        &NSString::from_str(title), Some(target), Some(sel!(aiSettingsAction:)), mtm(),
    );
    button.setTag(tag);
    button.setFrame(frame);
    theme::button(&button, false);
    parent.addSubview(&button);
    button
}

fn select(popup: &NSPopUpButton, values: &[&str], selected: Option<&str>) {
    let index = selected.and_then(|value| values.iter().position(|item| *item == value));
    popup.selectItemAtIndex(index.map_or(-1, |index| index as isize));
}

fn skill_name(name: &str) -> &str {
    match name {
        "extract-material" => "자료 읽기",
        "align-concepts" => "개념 맞추기",
        "connect-concepts" => "개념 잇기",
        "review-links" => "연결 검수",
        "distill-chat" => "대화 정리",
        "routing" => "기준값",
        _ => name,
    }
}

fn tier_name(tier: &str) -> &str {
    match tier {
        "fast" => "빠름",
        "balanced" => "균형",
        "precise" => "정밀",
        "custom" => "직접 선택",
        _ => tier,
    }
}

fn automatic_title(tiers: &Value, stage: &Value) -> String {
    if stage.is_null() {
        return "방식에 맡김 — 이 방식에서는 하지 않아요".into();
    }
    let mut tier = stage["tier"].as_str().unwrap_or("");
    let mut model = stage["model"].as_str().unwrap_or("");
    // The returned stage includes overrides; the first item must describe clearing one.
    if tier == "custom" {
        let names = ["fast", "balanced", "precise"];
        let requested = stage["requested"].as_str().unwrap_or("balanced");
        let rank = names.iter().position(|name| *name == requested).unwrap_or(1);
        if let Some(name) = names[..=rank].iter().rev()
            .chain(names.iter().rev())
            .find(|name| tiers[**name].is_object())
        {
            tier = name;
            model = tiers[*name]["model"].as_str().unwrap_or("");
        }
    }
    format!("방식에 맡김 — {} · {model}", tier_name(tier))
}

impl SettingsWindow {
    unsafe fn new() -> Self {
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm()), rect(0., 0., 560., 700.),
            NSWindowStyleMask::Titled | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered, false,
        );
        window.setTitle(&NSString::from_str("AI 설정"));
        theme::apply_window(&window);
        window.setReleasedWhenClosed(false);
        let root = view(rect(0., 0., 560., 700.));
        window.setContentView(Some(&root));
        theme::canvas(&root);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(20., 64., 520., 578.));
        scroll.setHasVerticalScroller(true);
        scroll.setHasHorizontalScroller(false);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        root.addSubview(&scroll);
        crate::scrollbar::style(&scroll);
        let document = view(rect(0., 0., 500., 578.));
        scroll.setDocumentView(Some(&document));
        let upper = view(rect(20., 42., 460., 520.));
        document.addSubview(&upper);
        for y in [416., 304., 216., 118.] {
            upper.addSubview(&theme::soft_divider(rect(0., y, 460., 1.), false));
        }
        let target: Retained<AiSettingsTarget> = msg_send![AiSettingsTarget::alloc(mtm()), init];
        let _: () = msg_send![&window, setDelegate: &*target];
        label(&upper, "과목 정리 방식", rect(0., 486., 460., 22.), 13., true);
        let mode = popup(&upper, &target, 1, "과목 정리 방식", rect(0., 452., 460., 28.), &["알아서", "빠르게", "꼼꼼하게"]);
        let mode_note = label(&upper, "", rect(0., 424., 460., 24.), 11., false);
        label(&upper, "대화 정리", rect(0., 374., 460., 22.), 13., true);
        let distill = popup(&upper, &target, 2, "대화 정리", rect(0., 340., 460., 28.), &["끔", "물어보고", "중요한 것만 알아서"]);
        let distill_note = label(&upper, "", rect(0., 312., 460., 24.), 11., false);
        label(&upper, "뒤에서 쓰는 양", rect(0., 262., 460., 22.), 13., true);
        let background = popup(&upper, &target, 3, "뒤에서 쓰는 양", rect(0., 228., 460., 28.), &[]);
        label(&upper, "이번 주 사용", rect(0., 174., 460., 22.), 13., true);
        let usage = label(&upper, "", rect(0., 128., 460., 42.), 12., false);
        label(&upper, "정리 방법(스킬)", rect(0., 92., 460., 22.), 13., true);
        let skills = label(&upper, "", rect(0., 38., 460., 48.), 12., false);
        let skills_error = label(&document, "", rect(20., 64., 460., 0.), 11., false);
        skills_error.setTextColor(Some(&NSColor::systemRedColor()));
        button(&document, &target, 5, "스킬 폴더 열기", rect(20., 28., 170., 30.));
        let reset = button(&document, &target, 6, "기본값으로 되돌리기", rect(206., 28., 220., 30.));
        reset.setEnabled(false);
        let close = button(&root, &target, 11, "닫기", rect(464., 20., 76., 28.));
        close.setKeyEquivalent(&NSString::from_str("\u{1b}"));
        crate::hand_cursor::install(&close);
        let (track, track_body) = theme::chat_tab_group(rect(266., 650., 274., 40.));
        root.addSubview(&track);
        let tab_selection = theme::chat_tab_selection(rect(4., 4., 133., 32.));
        track_body.addSubview(&tab_selection);
        let tabs = [
            button(&track_body, &target, 9, "정리 방식", rect(4., 4., 133., 32.)),
            button(&track_body, &target, 10, "도우미 설정", rect(137., 4., 133., 32.)),
        ];
        theme::chat_tab(&tabs[0], true, 0);
        theme::chat_tab(&tabs[1], false, 1);
        tabs[0].setState(1);
        let helpers = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), scroll.frame());
        helpers.setHasVerticalScroller(true);
        helpers.setAutohidesScrollers(true);
        helpers.setDrawsBackground(false);
        root.addSubview(&helpers);
        crate::scrollbar::style(&helpers);
        helpers.setHidden(true);
        let advanced = view(rect(0., 0., 500., HELPER_DOCUMENT_HEIGHT));
        helpers.setDocumentView(Some(&advanced));
        let provider = popup(&advanced, &target, 4, "추천 프리셋", rect(20., 662., 230., 28.), &["직접 선택", "Codex / Plus", "Codex / Pro", "Claude / Pro", "Claude / Max", "Claude / Ultra"]);
        let load = button(&advanced, &target, 7, "모델 목록 불러오기", rect(260., 662., 220., 28.));
        let tiers_status = label(&advanced, "모델 목록을 불러오면 직접 고를 수 있어요", rect(20., 633., 460., 24.), 11., false);
        let roles: [(&str, &str, &[u8]); 5] = [
            ("자료 읽기 도우미", "자료에서 핵심 개념과 근거를 찾아요.", include_bytes!("../../assets/helpers/extract.png")),
            ("개념 정리 도우미", "이름이 비슷한 개념을 비교해 합칠 후보를 골라요.", include_bytes!("../../assets/helpers/align.png")),
            ("학습 지도 도우미", "개념을 단원으로 묶고 서로 어떻게 연결되는지 제안해요.", include_bytes!("../../assets/helpers/connect.png")),
            ("연결 확인 도우미", "꼼꼼하게 정리할 때, 제안된 연결의 근거를 확인해요.", include_bytes!("../../assets/helpers/review.png")),
            ("대화 노트 도우미", "질문과 답변에서 다시 볼 내용을 노트 초안으로 정리해요.", include_bytes!("../../assets/helpers/distill.png")),
        ];
        let mut stages = Vec::with_capacity(STAGES.len());
        for (index, (title, description, bytes)) in roles.into_iter().enumerate() {
            let y = 552. - index as f64 * 122.;
            let card = theme::chat_tab_group(rect(10., y - 54., 480., 118.)).0;
            advanced.addSubview(&card);
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes)) {
                let icon = NSImageView::imageViewWithImage(&image, mtm());
                icon.setFrame(rect(18., y - 24., 76., 76.));
                advanced.addSubview(&icon);
            }
            label(&advanced, title, rect(104., y + 32., 376., 22.), 14., true);
            label(&advanced, description, rect(104., y + 8., 384., 22.), 12., false);
            let popup = popup(&advanced, &target, 100 + index as isize, title, rect(104., y - 24., 376., 28.), &[]);
            popup.setEnabled(false);
            let note = label(&advanced, "", rect(104., y - 50., 376., 26.), 10.5, false);
            note.setTextColor(Some(&NSColor::secondaryLabelColor()));
            stages.push(StageRow { popup, note, models: Vec::new() });
        }
        for child in advanced.subviews().iter() {
            let origin = child.frame().origin;
            child.setFrameOrigin(NSPoint::new(origin.x, origin.y + HELPER_VERTICAL_PADDING - 10.));
        }
        let status = label(&root, "", rect(28., 8., 416., 46.), 11., false);
        window.center();
        Self {
            window, _target: target, tabs, tab_selection, scroll, helpers, document, upper, mode, mode_note, distill,
            distill_note, background, usage, skills, skills_error, reset, provider, load,
            tiers_status, stages, status, status_error: false, settings: Value::Null,
            tiers: None, pending: None, tiers_revision: 0,
        }
    }

    unsafe fn status(&mut self, message: &str, error: bool) {
        self.status_error = error;
        self.status.setStringValue(&NSString::from_str(message));
        let color = if error { NSColor::systemRedColor() } else { NSColor::secondaryLabelColor() };
        self.status.setTextColor(Some(&color));
    }

    unsafe fn fail(&mut self, error: &str) {
        if self.status_error {
            self.status(&format!("{}\n{error}", self.status.stringValue()), true);
        } else {
            self.status(error, true);
        }
    }

    fn provider_id(&self) -> &'static str {
        self.settings["organize_provider"].as_str().and_then(|p| match p { "claude" => Some("claude"), "antigravity" => Some("antigravity"), "codex" => Some("codex"), _ => None }).unwrap_or("codex")
    }

    fn mode_id(&self) -> &str {
        self.settings["organize_mode"].as_str().unwrap_or("auto")
    }

    unsafe fn render_settings(&self) {
        let ready = self.settings.is_object();
        select(&self.provider, &PRESETS, Some(self.settings["helper_preset"].as_str().unwrap_or("custom")));
        self.mode.setEnabled(ready);
        self.distill.setEnabled(ready);
        self.background.setEnabled(ready);
        self.load.setEnabled(ready && self.pending.is_none());
        select(&self.mode, &MODES, self.settings["organize_mode"].as_str());
        // Until the first choice the menu names the unset state instead of showing an empty box.
        let unset = ready && self.settings["distill_mode"].is_null();
        let placeholder = self.distill.numberOfItems() as usize > DISTILL.len();
        if unset && !placeholder {
            self.distill.insertItemWithTitle_atIndex(&NSString::from_str("아직 정하지 않음"), 0);
        } else if !unset && placeholder {
            self.distill.removeItemAtIndex(0);
        }
        if unset {
            self.distill.selectItemAtIndex(0);
        } else {
            select(&self.distill, &DISTILL, self.settings["distill_mode"].as_str());
        }
        select(&self.background, &BACKGROUND, self.settings["background"].as_str());
        self.mode_note.setStringValue(&NSString::from_str(if !ready { "" } else {
            match self.mode_id() {
                "fast" => "사용량을 아껴요",
                "thorough" => "가장 정밀하게 읽고 이은 연결을 한 번 더 확인해요",
                _ => "읽기는 균형, 맞추기와 잇기는 정밀하게 해요",
            }
        }));
        self.distill_note.setStringValue(&NSString::from_str(if ready && self.settings["distill_mode"].is_null() {
            "처음 필요할 때 한 번 물어요"
        } else { "" }));
    }

    unsafe fn refresh_settings(&mut self) {
        match rpc(json!({"op":"ai.settings.get"})) {
            Ok(result) => {
                self.settings = result["settings"].clone();
                self.background.removeAllItems();
                for (index, title) in ["적게", "보통", "넉넉하게"].iter().enumerate() {
                    let daily = &result["routing"]["background_daily"][BACKGROUND[index]];
                    let title = match daily.as_u64() {
                        Some(n) => format!("{title}(하루 {n}번)"),
                        None => format!("{title}(기준값을 확인해 주세요)"),
                    };
                    self.background.addItemWithTitle(&NSString::from_str(&title));
                }
                let usage = &result["usage"];
                self.usage.setStringValue(&NSString::from_str(&format!(
                    "과목 정리 {}번 · AI 호출 {}번\n뒤에서 정리 {}번 (오늘 {}번)",
                    usage["organize_runs"].as_u64().unwrap_or(0),
                    usage["organize_calls"].as_u64().unwrap_or(0),
                    usage["background_calls"].as_u64().unwrap_or(0),
                    usage["background_today"].as_u64().unwrap_or(0),
                )));
            }
            Err(error) => self.fail(&error),
        }
        self.render_settings();
    }

    unsafe fn layout_skills(&self) {
        let clip = self.scroll.contentView();
        let bounds = clip.bounds();
        let old_height = self.document.frame().size.height;
        let offset = (old_height - bounds.origin.y - bounds.size.height).max(0.);
        let error_height = if self.skills_error.stringValue().is_empty() { 0. } else {
            self.skills_error.cell().map(|cell| cell.cellSizeForBounds(rect(0., 0., 460., 10000.)).height.ceil() + 12.).unwrap_or(40.)
        };
        self.skills_error.setFrame(rect(20., 64., 460., error_height));
        self.upper.setFrame(rect(20., 42. + error_height, 460., 520.));
        let height = 578. + error_height;
        self.document.setFrame(rect(0., 0., 500., height));
        clip.scrollToPoint(NSPoint::new(0., (height - bounds.size.height - offset).max(0.)));
        self.scroll.reflectScrolledClipView(&clip);
    }

    unsafe fn refresh_skills(&mut self) {
        match rpc(json!({"op":"ai.skills.status"})) {
            Ok(result) => {
                let customized: Vec<&str> = result["customized"].as_array().into_iter().flatten()
                    .filter_map(Value::as_str).map(skill_name).collect();
                let summary = if customized.is_empty() {
                    format!("앱에 든 기본 방법 사용 중 · 묶음 버전 {}", result["bundle_version"].as_str().unwrap_or("1"))
                } else {
                    format!("고친 방법 사용 중: {}", customized.join(", "))
                };
                self.skills.setStringValue(&NSString::from_str(&summary));
                self.reset.setEnabled(result["user_copy"].as_bool() == Some(true));
                let mut errors = Vec::new();
                if let Some(error) = result["routing_error"].as_str().filter(|error| !error.is_empty()) {
                    errors.push(error);
                }
                for skill in result["skills"].as_array().into_iter().flatten() {
                    if let Some(error) = skill["error"].as_str().filter(|error| !error.is_empty()) {
                        errors.push(error);
                    }
                }
                self.skills_error.setStringValue(&NSString::from_str(&errors.join("\n")));
                self.layout_skills();
            }
            Err(error) => self.fail(&error),
        }
    }

    unsafe fn save(&mut self, request: Value) -> bool {
        match rpc(request) {
            Ok(result) => {
                self.settings = result["settings"].clone();
                self.render_settings();
                true
            }
            Err(error) => {
                self.fail(&error);
                self.render_settings();
                self.render_tiers();
                false
            }
        }
    }

    unsafe fn clear_tiers(&mut self) {
        self.tiers_revision = self.tiers_revision.wrapping_add(1);
        self.tiers = None;
        for row in &mut self.stages {
            row.popup.removeAllItems();
            row.popup.setEnabled(false);
            row.note.setStringValue(&NSString::from_str(""));
            row.models.clear();
        }
    }

    unsafe fn load_tiers(&mut self) {
        if self.pending.is_some() || !self.settings.is_object() { return; }
        let provider = self.provider_id();
        let mode = self.mode_id().to_owned();
        let request = json!({"op":"ai.tiers","provider":provider,"mode":mode});
        let custom = self.settings["helper_preset"].as_str().unwrap_or("custom") == "custom";
        let (tx, rx) = mpsc::channel();
        self.pending = Some(Pending { provider, mode, revision: self.tiers_revision, rx });
        self.load.setEnabled(false);
        self.provider.setEnabled(false);
        for row in &self.stages { row.popup.setEnabled(false); }
        self.tiers_status.setStringValue(&NSString::from_str("불러오는 중…"));
        std::thread::spawn(move || {
            let result = if custom { rpc(json!({"op":"ai.helper_catalog"})) } else { rpc(request) };
            let _ = tx.send(result);
        });
    }

    unsafe fn render_tiers(&mut self) {
        let Some(result) = self.tiers.as_ref() else { return; };
        let provider = self.provider_id();
        let custom = self.settings["helper_preset"].as_str().unwrap_or("custom") == "custom";
        let thorough = self.mode_id() == "thorough";
        for ((stage, _), row) in STAGES.iter().zip(self.stages.iter_mut()) {
            row.popup.removeAllItems();
            row.models.clear();
            if !custom {
                let info = &result["stages"][*stage];
                row.popup.addItemWithTitle(&NSString::from_str(&automatic_title(&result["tiers"], info)));
                row.popup.selectItemAtIndex(0);
                row.popup.setEnabled(false);
                row.note.setStringValue(&NSString::from_str(info["note"].as_str().unwrap_or("추천 구성 · 직접 선택에서 변경할 수 있어요")));
                continue;
            }
            row.popup.addItemWithTitle(&NSString::from_str("정리 방식에 맡김"));
            row.models.push((provider.into(), String::new()));
            for model in result["models"].as_array().into_iter().flatten() {
                let (Some(id), Some(p)) = (model["id"].as_str(), model["provider"].as_str()) else { continue; };
                if id.is_empty() { continue; }
                let label = model["label"].as_str().filter(|s| !s.is_empty()).unwrap_or(id);
                row.popup.addItemWithTitle(&NSString::from_str(&format!("{p} / {label}")));
                row.models.push((p.to_owned(), id.to_owned()));
            }
            let saved = &self.settings["helper_models"][*stage];
            let legacy = &self.settings["stage_models"][provider][*stage];
            let chosen_provider = saved["provider"].as_str().unwrap_or(provider);
            let chosen_model = if saved.is_object() { saved["model"].as_str() } else { legacy["model"].as_str() }.unwrap_or("");
            let selected = row.models.iter().position(|(p, m)| p == chosen_provider && m == chosen_model);
            let selected = selected.unwrap_or_else(|| {
                row.popup.addItemWithTitle(&NSString::from_str(&format!("{chosen_provider} / {chosen_model} (미확인)")));
                row.models.push((chosen_provider.into(), chosen_model.into()));
                row.models.len() - 1
            });
            row.popup.selectItemAtIndex(selected as isize);
            row.popup.setEnabled(self.pending.is_none());
            row.note.setStringValue(&NSString::from_str(if *stage == "review" && !thorough {
                "꼼꼼하게 정리할 때 사용해요"
            } else { "공급자가 달라도 도우미별로 자유롭게 고를 수 있어요" }));
        }
    }

    unsafe fn poll(&mut self) {
        let result = match self.pending.as_ref().map(|pending| pending.rx.try_recv()) {
            Some(Ok(result)) => result,
            Some(Err(TryRecvError::Disconnected)) => Err("모델 목록을 불러오는 작업이 끝나지 못했어요".into()),
            _ => return,
        };
        let pending = self.pending.take().unwrap();
        self.load.setEnabled(self.settings.is_object());
        self.provider.setEnabled(true);
        if pending.provider != self.provider_id() || pending.mode != self.mode_id()
            || pending.revision != self.tiers_revision
        {
            self.clear_tiers();
            self.load_tiers();
            return;
        }
        match result {
            Ok(result) => {
                self.tiers = Some(result);
                let message = if self.settings["helper_preset"].as_str().unwrap_or("custom") == "custom" {
                    if self.tiers.as_ref().is_some_and(|v| v["errors"].as_array().is_some_and(|e| !e.is_empty())) {
                        "조회 가능한 공급자만 표시해요 · 구독 계정 연결을 확인해 주세요"
                    } else { "도우미마다 공급자와 모델을 자유롭게 선택하세요" }
                } else { "요금제별 추천 구성이에요 · 실제 계정의 모델 목록을 사용해요" };
                self.tiers_status.setStringValue(&NSString::from_str(message));
                self.render_tiers();
            }
            Err(error) => {
                self.clear_tiers();
                self.tiers_status.setStringValue(&NSString::from_str("모델 목록을 불러오지 못했어요 · 아래 오류를 확인해 주세요"));
                self.fail(&error);
            }
        }
    }

    unsafe fn handle(&mut self, tag: isize, selected: isize) {
        self.status("", false);
        match tag {
            1 => {
                if let Some(mode) = MODES.get(selected as usize) {
                    if self.save(json!({"op":"ai.settings.set","organize_mode":mode})) {
                        let had_tiers = self.tiers.is_some();
                        self.clear_tiers();
                        if had_tiers { self.load_tiers(); }
                    }
                }
            }
            2 => {
                let placeholder = (self.distill.numberOfItems() as usize > DISTILL.len()) as usize;
                if let Some(mode) = (selected as usize).checked_sub(placeholder).and_then(|index| DISTILL.get(index)) {
                    self.save(json!({"op":"ai.settings.set","distill_mode":mode}));
                }
            }
            3 => {
                if let Some(background) = BACKGROUND.get(selected as usize) {
                    self.save(json!({"op":"ai.settings.set","background":background}));
                }
            }
            4 => {
                if let Some(preset) = PRESETS.get(selected as usize) {
                    if self.save(json!({"op":"ai.settings.set","helper_preset":preset})) {
                        self.clear_tiers();
                        self.load_tiers();
                    }
                }
            }
            5 => {
                match rpc(json!({"op":"ai.skills.open"})) {
                    Ok(result) => {
                        if let Some(path) = result["path"].as_str().filter(|path| !path.is_empty()) {
                            let url = NSURL::fileURLWithPath(&NSString::from_str(path));
                            if !NSWorkspace::sharedWorkspace().openURL(&url) {
                                self.fail("스킬 폴더를 열지 못했어요");
                            }
                        } else { self.fail("스킬 폴더 경로를 받지 못했어요"); }
                        self.refresh_skills();
                    }
                    Err(error) => self.fail(&error),
                }
            }
            6 => {
                if confirm_action("기본값으로 되돌릴까요?", "고친 내용은 지우지 않고 옆 폴더로 옮겨요", "기본값으로 되돌리기") {
                    match rpc(json!({"op":"ai.skills.reset"})) {
                        Ok(result) => {
                            if result["reset"].as_bool() == Some(true) {
                                self.status(&format!("기본값으로 되돌렸어요 · 이전 내용: {}", result["backup"].as_str().unwrap_or("")), false);
                            } else { self.status("이미 기본 방법을 쓰고 있어요", false); }
                            self.refresh_settings();
                            self.refresh_skills();
                            let had_tiers = self.tiers.is_some();
                            self.clear_tiers();
                            if had_tiers { self.load_tiers(); }
                        }
                        Err(error) => self.fail(&error),
                    }
                }
            }
            7 => self.load_tiers(),
            9 | 10 => {
                let show = tag == 10;
                self.tab_selection.setFrameOrigin(NSPoint::new(if show { 137. } else { 4. }, 4.));
                for (index, tab) in self.tabs.iter().enumerate() {
                    let active = index == usize::from(show);
                    theme::chat_tab(tab, active, index);
                }
                self.scroll.setHidden(show);
                self.helpers.setHidden(!show);
                if show {
                    let clip = self.helpers.contentView();
                    clip.scrollToPoint(NSPoint::new(0., (HELPER_DOCUMENT_HEIGHT - clip.bounds().size.height).max(0.)));
                    self.helpers.reflectScrolledClipView(&clip);
                    if self.tiers.is_none() { self.load_tiers(); }
                }
            }
            11 => {
                NSApplication::sharedApplication(mtm()).stopModal();
            }
            100..=104 => {
                let index = (tag - 100) as usize;
                if let Some((provider, model)) = self.stages[index].models.get(selected as usize).cloned() {
                    if self.save(json!({"op":"ai.settings.set","helper_model":{
                        "provider":provider,"stage":STAGES[index].0,"model":model,"effort":"",
                    }})) { self.load_tiers(); }
                }
            }
            _ => (),
        }
    }
}

pub unsafe fn open() {
    STATE.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else { return; };
        let created = state.is_none();
        let settings = state.get_or_insert_with(|| SettingsWindow::new());
        settings.status("", false);
        settings.refresh_settings();
        settings.refresh_skills();
        if created {
            select(&settings.provider, &PRESETS, Some(settings.settings["helper_preset"].as_str().unwrap_or("custom")));
            let clip = settings.scroll.contentView();
            clip.scrollToPoint(NSPoint::new(0., (settings.document.frame().size.height - clip.bounds().size.height).max(0.)));
            settings.scroll.reflectScrolledClipView(&clip);
        } else if settings.tiers.is_some() {
            settings.clear_tiers();
            settings.load_tiers();
        }

    });
    let window = STATE.with(|state| state.borrow().as_ref().unwrap().window.clone());
    let target = STATE.with(|state| state.borrow().as_ref().unwrap()._target.clone());
    let timer: Retained<NSTimer> = msg_send![NSTimer::class(), timerWithTimeInterval: 0.1f64, target: &*target, selector: sel!(settingsTick:), userInfo: std::ptr::null::<AnyObject>(), repeats: true];
    NSRunLoop::mainRunLoop().addTimer_forMode(&timer, objc2_app_kit::NSModalPanelRunLoopMode);
    window.center();
    window.makeKeyAndOrderFront(None);
    NSApplication::sharedApplication(mtm()).runModalForWindow(&window);
    timer.invalidate();
    window.orderOut(None);
}

pub unsafe fn tick() {
    STATE.with(|state| {
        let Ok(mut state) = state.try_borrow_mut() else { return; };
        let Some(settings) = state.as_mut() else { return; };
        let events = EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()));
        for (tag, selected) in events { settings.handle(tag, selected); }
        settings.poll();
    });
}
