//! 구독 연결 관리 창. 연결한 공급자는 모두 채팅 모델 메뉴에서 함께 쓰므로
//! 이 창은 공급자를 고르지 않고 공급자별 설치·로그인 상태만 관리한다.
//! 행을 펼쳐야 설치·로그인 텍스트 버튼이 나타난다.
use crate::theme::layer;
use crate::{font, hand_cursor, mtm, rect, text_label, theme};
use objc2::{define_class, msg_send, rc::Retained, runtime::AnyObject, sel, AnyThread, DefinedClass, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};

const IDS: [&str; 3] = ["codex", "claude", "antigravity"];
const NAMES: [&str; 3] = ["Codex", "Claude", "Antigravity"];
const ACCOUNTS: [&str; 3] = ["ChatGPT 계정", "Claude 계정", "Google 계정"];
const W: f64 = 420.;
const PAD: f64 = 18.;
const GROUP_Y: f64 = 94.;
const HEAD: f64 = 48.;
const DETAIL: f64 = 86.;
const INSET: f64 = 56.;
/// 그룹 아래 안내문·닫기 줄과 아래 여백.
const FOOT: f64 = 12. + 34. + 16.;
fn window_height(group: f64) -> f64 { GROUP_Y + group + FOOT }
const ROW_W: f64 = W - PAD * 2.;
const CLOSE: isize = 0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action { Install, Login }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tone { Ok, Warn, Idle }

/// 응답에 없는 값은 실패가 아니라 미확인이다.
fn flags(state: &Value, id: &str) -> (Option<bool>, Option<bool>) {
    let p = state["providers"].as_array().and_then(|ps| ps.iter().find(|p| p["provider"] == id));
    let Some(p) = p else { return (None, None) };
    (p["installed"].as_bool(), p["logged_in"].as_bool())
}
fn active_job(v: &Value) -> bool { matches!(v["state"].as_str(), Some("queued" | "running" | "waiting_login")) }
fn node_needed(state: &Value) -> bool { flags(state, "codex").0 == Some(false) && state["node"]["ready"] == false }
fn execution_error(state: &Value, id: &str) -> bool {
    state["providers"].as_array().is_some_and(|ps| ps.iter().any(|p| p["provider"] == id && p["execution_error"] == true))
}
fn show_notice(state: &Value, i: usize) -> bool { (i == 0 && (node_needed(state) || active_job(&state["setup_job"]))) || execution_error(state, IDS[i]) }
fn summary(state: &Value, id: &str) -> (&'static str, Tone) {
    if id == "codex" && active_job(&state["setup_job"]) { return ("Node.js 설치 중", Tone::Idle); }
    if id == "codex" && node_needed(state) { return ("준비 환경 확인 필요", Tone::Warn); }
    if execution_error(state, id) { return ("CLI 실행 확인 필요", Tone::Warn); }
    match flags(state, id) {
        (Some(false), _) => ("CLI 설치 필요", Tone::Warn),
        (Some(true), Some(true)) => ("연결됨", Tone::Ok),
        (Some(true), Some(false)) => ("로그인 필요", Tone::Warn),
        _ => ("미확인", Tone::Idle),
    }
}
/// 펼친 행의 (CLI 값, CLI 버튼, 로그인 값, 로그인 버튼).
fn details(state: &Value, id: &str) -> ((&'static str, Tone), bool, (&'static str, Tone), bool) {
    let (installed, logged_in) = flags(state, id);
    let cli = match installed { Some(true) => ("설치됨", Tone::Ok), Some(false) => ("설치 필요", Tone::Warn), None => ("미확인", Tone::Idle) };
    let login = match (installed, logged_in) {
        (Some(true), Some(true)) => ("로그인됨", Tone::Ok),
        (Some(true), Some(false)) => ("로그인 필요", Tone::Warn),
        _ => ("미확인", Tone::Idle),
    };
    (cli, installed == Some(false), login, installed == Some(true) && logged_in == Some(false))
}
/// 설치와 로그인이 모두 확인된 공급자만 고정 순서로 표시한다.
pub fn connected_names(state: &Value) -> Vec<&'static str> {
    IDS.iter().enumerate().filter(|(_, id)| flags(state, id) == (Some(true), Some(true)))
        .map(|(i, _)| NAMES[i]).collect()
}

/// 분석 공급자가 비어 있을 때 대신 쓸 첫 연결 공급자.
pub fn first_connected(state: &Value) -> Option<String> {
    IDS.iter().find(|id| flags(state, id) == (Some(true), Some(true))).map(|id| id.to_string())
}

fn tone_color(t: Tone) -> Retained<NSColor> {
    match t {
        Tone::Ok => dot_color(Tone::Ok),
        Tone::Warn | Tone::Idle => NSColor::secondaryLabelColor(),
    }
}
fn dot_color(t: Tone) -> Retained<NSColor> {
    match t {
        Tone::Ok => NSColor::colorWithSRGBRed_green_blue_alpha(0.19, 0.72, 0.38, 1.),
        Tone::Warn | Tone::Idle => NSColor::colorWithSRGBRed_green_blue_alpha(0.76, 0.76, 0.81, 1.),
    }
}

define_class!(
    /// 위에서 아래로 쌓는 배치를 위해 좌표를 뒤집은 뷰.
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyProviderFlippedView"]
    #[thread_kind = MainThreadOnly]
    struct Flipped;
    impl Flipped {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);
unsafe fn flipped(frame: NSRect) -> Retained<NSView> {
    let v: Retained<Flipped> = msg_send![Flipped::alloc(mtm()), initWithFrame: frame];
    v.into_super()
}

unsafe fn schedule(target: &Controller, selector: objc2::runtime::Sel, delay: f64) {
    let modes = NSArray::from_slice(&[NSDefaultRunLoopMode, NSModalPanelRunLoopMode]);
    let _: () = msg_send![target, performSelector: selector, withObject: Option::<&AnyObject>::None, afterDelay: delay, inModes: &*modes];
}

struct Row {
    container: Retained<NSView>,
    header: Retained<NSButton>,
    chevron: Retained<NSImageView>,
    detail: Retained<NSView>,
    controls: Retained<NSView>,
    notice: Retained<NSView>,
    notice_title: Retained<NSTextField>,
    notice_body: Retained<NSTextField>,
    node_auto: Retained<NSButton>,
    node_manual: Retained<NSButton>,
    node_check: Retained<NSButton>,
    dot: Retained<NSView>,
    status: Retained<NSTextField>,
    cli_value: Retained<NSTextField>,
    cli_button: Retained<NSButton>,
    spinner: Retained<NSProgressIndicator>,
    login_value: Retained<NSTextField>,
    login_button: Retained<NSButton>,
    recheck: Retained<NSButton>,
}
struct Data {
    state: RefCell<Value>,
    open: Cell<Option<usize>>,
    result: Cell<Option<(usize, Action)>>,
    group: Retained<NSView>,
    installing: RefCell<Option<(usize, String)>>,
    node_installing: Cell<bool>,
    rows: Vec<Row>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "StologyProviderDialog"]
    #[thread_kind = MainThreadOnly]
    #[ivars = Data]
    struct Controller;
    unsafe impl NSObjectProtocol for Controller {}
    impl Controller {
        #[unsafe(method(toggle:))]
        unsafe fn toggle(&self, sender: &NSButton) {
            let i = sender.tag() as usize;
            let open = if self.ivars().open.get() == Some(i) { None } else { Some(i) };
            self.ivars().open.set(open);
            self.layout(true);
        }
        /// tag = 공급자 × 10 + 동작(1 설치, 2 로그인).
        #[unsafe(method(act:))]
        unsafe fn act(&self, sender: &NSButton) {
            let (i, a) = ((sender.tag() / 10) as usize, sender.tag() % 10);
            if self.ivars().installing.borrow().is_some() { return; }
            if a == 1 {
                if self.ivars().installing.borrow().is_some() { return; }
                match crate::rpc(json!({"op":"provider.install", "provider":IDS[i]})) {
                    Ok(v) => if let Some(id) = v["id"].as_str() {
                        *self.ivars().installing.borrow_mut() = Some((i, id.to_owned()));
                        self.refresh();
                        schedule(self, sel!(pollInstall:), 0.5);
                    },
                    Err(e) => { let alert = NSAlert::new(mtm()); alert.setMessageText(&NSString::from_str(&e.to_string())); alert.runModal(); }
                }
            } else {
                self.ivars().result.set(Some((i, Action::Login)));
                NSApplication::sharedApplication(mtm()).stopModalWithCode(1);
            }
        }
        #[unsafe(method(nodeAction:))]
        unsafe fn node_action(&self, sender: &NSButton) {
            match sender.tag() {
                3 => {
                    if let Some(url) = NSURL::URLWithString(&NSString::from_str("https://nodejs.org/en/download")) {
                        NSWorkspace::sharedWorkspace().openURL(&url);
                    }
                }
                4 => {
                    if self.ivars().node_installing.get() {
                        let job = self.ivars().installing.borrow().clone();
                        if let Some((_, id)) = job {
                            if let Ok(j) = crate::rpc(json!({"op":"provider.job.cancel","id":id})) {
                                let _: () = msg_send![objc2::class!(NSObject), cancelPreviousPerformRequestsWithTarget: self, selector: sel!(pollInstall:), object: Option::<&AnyObject>::None];
                                self.ivars().state.borrow_mut()["setup_job"] = j;
                                *self.ivars().installing.borrow_mut() = None;
                                self.ivars().node_installing.set(false);
                            }
                        }
                    } else if self.ivars().installing.borrow().is_none() {
                        match crate::rpc(json!({"op":"provider.node.install"})) {
                            Ok(j) => if let Some(id) = j["id"].as_str() {
                                *self.ivars().installing.borrow_mut() = Some((0, id.to_owned()));
                                self.ivars().node_installing.set(true);
                                self.ivars().state.borrow_mut()["setup_job"] = j;
                                schedule(self, sel!(pollInstall:), 0.5);
                            },
                            Err(e) => { self.ivars().state.borrow_mut()["setup_job"] = json!({"state":"failed","error":e.to_string()}); }
                        }
                    }
                    self.layout(false);
                }
                _ => { let _: () = msg_send![self, recheck: sender]; }
            }
        }
        #[unsafe(method(pollInstall:))]
        unsafe fn poll_install(&self, _sender: Option<&AnyObject>) {
            let job = self.ivars().installing.borrow().clone();
            let Some((_, id)) = job else { return };
            if let Ok(j) = crate::rpc(json!({"op":"provider.job.status", "id":id})) {
                let node = self.ivars().node_installing.get();
                if node { self.ivars().state.borrow_mut()["setup_job"] = j.clone(); }
                let state = j["state"].as_str().unwrap_or("");
                if ["succeeded", "failed", "cancelled"].contains(&state) {
                    *self.ivars().installing.borrow_mut() = None;
                    self.ivars().node_installing.set(false);
                    if node {
                        if let Ok(v) = crate::rpc(json!({"op":"provider.setup.status"})) { self.ivars().state.borrow_mut()["node"] = v; }
                    } else if let Ok(v) = crate::rpc(json!({"op":"provider.status"})) { *self.ivars().state.borrow_mut() = v; }
                    self.layout(false);
                    if state == "failed" && !node {
                        let alert = NSAlert::new(mtm());
                        alert.setMessageText(&NSString::from_str(j["error"].as_str().unwrap_or("CLI 설치에 실패했습니다.")));
                        alert.runModal();
                    }
                    return;
                }
                self.refresh();
            }
            schedule(self, sel!(pollInstall:), 1.0);
        }
        #[unsafe(method(recheck:))]
        unsafe fn recheck(&self, sender: &NSButton) {
            sender.setEnabled(false);
            text_button_title(sender, "확인 중…");
            if let Some(w) = sender.window() { w.displayIfNeeded(); }
            // provider.status는 각 CLI를 실행하므로 잠시 걸릴 수 있다. 실패하면 기존 상태를 유지한다.
            if let Ok(v) = crate::rpc(json!({"op":"provider.status"})) {
                if v["providers"].is_array() { *self.ivars().state.borrow_mut() = v; }
            }
            sender.setEnabled(true);
            text_button_title(sender, if sender.tag() == 5 { "설치 여부 확인" } else { "새로고침" });
            self.layout(false);
        }
        #[unsafe(method(hideClosed:))]
        unsafe fn hide_closed(&self, _sender: Option<&AnyObject>) {
            let d = self.ivars();
            for (i, r) in d.rows.iter().enumerate() { r.detail.setHidden(d.open.get() != Some(i)); }
        }
        #[unsafe(method(close:))]
        unsafe fn close(&self, _sender: &NSButton) {
            if self.ivars().installing.borrow().is_none() || self.ivars().node_installing.get() { NSApplication::sharedApplication(mtm()).stopModalWithCode(CLOSE); }
        }
    }
);

impl Controller {
    unsafe fn refresh(&self) {
        let d = self.ivars(); let state = d.state.borrow();
        for (i, r) in d.rows.iter().enumerate() {
            let (text, tone) = summary(&state, IDS[i]);
            r.status.setStringValue(&NSString::from_str(text));
            let _: () = msg_send![&*layer(&r.dot), setBackgroundColor: &*dot_color(tone).CGColor()];
            let (cli, cli_btn, login, login_btn) = details(&state, IDS[i]);
            let installing = !d.node_installing.get() && d.installing.borrow().as_ref().map(|(row, _)| *row == i).unwrap_or(false);
            let needs_node = i == 0 && node_needed(&state);
            let broken = execution_error(&state, IDS[i]);
            r.notice.setHidden(!show_notice(&state, i));
            let node_busy = i == 0 && d.node_installing.get();
            let failed = needs_node && state["setup_job"]["state"] == "failed";
            r.notice_title.setStringValue(&NSString::from_str(if node_busy { "Node.js를 설치하고 있어요" } else if failed { "Node.js를 설치하지 못했어요" } else if needs_node { "Node.js 설치가 필요해요" } else { "CLI를 실행할 수 없어요" }));
            let progress = state["setup_job"]["events"].as_array().and_then(|es| es.last()).and_then(|e|e["message"].as_str()).unwrap_or("설치 준비 중…");
            let body = if node_busy { format!("{progress}\n이 창을 닫아도 설치는 계속됩니다.") }
                else if failed { "연결·저장 공간을 확인한 뒤 다시 시도해 주세요.".into() }
                else if needs_node { "Codex 설치에 Node.js와 npm이 필요합니다.\n자동 설치는 Stology 전용 공간을 사용합니다.".into() }
                else { "설치된 CLI의 실행 환경을 확인해 주세요.".into() };
            r.notice_body.setStringValue(&NSString::from_str(&body));
            r.node_manual.setHidden(node_busy || !needs_node);
            r.node_auto.setHidden(!needs_node && !node_busy);
            r.node_check.setHidden(node_busy);
            r.node_auto.setEnabled(d.installing.borrow().is_none() || node_busy);
            text_button_title(&r.node_auto, if node_busy { "취소" } else if failed { "다시 설치" } else { "자동 설치" });
            place_value(&r.cli_value, &r.cli_button, if installing { ("설치 중", Tone::Idle) } else { cli }, cli_btn && !installing);
            r.cli_button.setEnabled(d.installing.borrow().is_none() && !needs_node && !broken);
            r.login_button.setEnabled(d.installing.borrow().is_none());
            r.spinner.setHidden(!installing);
            if installing {
                let f = r.cli_value.frame();
                r.cli_value.setFrame(rect(f.origin.x - 20., f.origin.y, f.size.width, f.size.height));
                r.spinner.startAnimation(None);
            } else { r.spinner.stopAnimation(None); }
            place_value(&r.login_value, &r.login_button, login, login_btn);
            let open = d.open.get() == Some(i);
            let _: () = msg_send![&*r.header, setAccessibilityLabel: &*NSString::from_str(&format!("{}, {}", NAMES[i], text))];
            let _: () = msg_send![&*r.header, setAccessibilityExpanded: open];
        }
    }
    /// 펼친 행 하나만 상세 높이를 가진다. 아래 행과 그룹 높이를 함께 움직여 미끄러지듯 열고 닫는다.
    unsafe fn layout(&self, animate: bool) {
        let d = self.ivars();
        let animate = animate && !NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion();
        NSAnimationContext::beginGrouping();
        let ctx = NSAnimationContext::currentContext();
        ctx.setDuration(if animate { 0.28 } else { 0. });
        ctx.setAllowsImplicitAnimation(animate);
        let mut y = 0.;
        for (i, r) in d.rows.iter().enumerate() {
            let open = d.open.get() == Some(i);
            let extra = if show_notice(&d.state.borrow(), i) { 122. } else { 0. };
            r.detail.setFrame(rect(0., HEAD, ROW_W, DETAIL + extra));
            r.controls.setFrame(rect(0., extra, ROW_W, DETAIL));
            let h = HEAD + if open { DETAIL + extra } else { 0. };
            let frame = rect(0., y, ROW_W, h);
            if let Some(s) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(if open { "chevron.down" } else { "chevron.right" }), None) {
                r.chevron.setImage(Some(&s));
            }
            if open { r.detail.setHidden(false); }
            if animate {
                let c: Retained<AnyObject> = msg_send![&*r.container, animator];
                let _: () = msg_send![&c, setFrame: frame];
            } else {
                r.container.setFrame(frame);
            }
            y += h;
        }
        let group = rect(PAD, GROUP_Y, ROW_W, y);
        // 접힌 상태에서는 창을 작게 두고, 펼칠 때만 위쪽을 고정한 채 아래로 늘린다.
        let window = d.group.window();
        let target = window.as_ref().map(|w| {
            let f = w.frame();
            let content = NSRect::new(NSPoint::ZERO, NSSize::new(W, window_height(y)));
            let h = w.frameRectForContentRect(content).size.height;
            NSRect::new(NSPoint::new(f.origin.x, f.origin.y + f.size.height - h), NSSize::new(f.size.width, h))
        });
        if animate {
            let g: Retained<AnyObject> = msg_send![&*d.group, animator];
            let _: () = msg_send![&g, setFrame: group];
            if let (Some(w), Some(t)) = (&window, target) {
                let a: Retained<AnyObject> = msg_send![&**w, animator];
                let _: () = msg_send![&a, setFrame: t, display: true];
            }
        } else {
            d.group.setFrame(group);
            if let (Some(w), Some(t)) = (&window, target) { w.setFrame_display(t, true); }
        }
        NSAnimationContext::endGrouping();
        // 접힌 상세는 잘려 보이지 않아도 접근성 트리에 남는다. 닫힘 전환이 끝난 뒤 숨긴다.
        schedule(self, sel!(hideClosed:), if animate { 0.3 } else { 0.0 });
        self.refresh();
    }
}

/// 필요한 동작이 있으면 상태 문구 대신 버튼 하나만 표시한다.
unsafe fn place_value(value: &NSTextField, button: &NSButton, (text, tone): (&str, Tone), show: bool) {
    value.setStringValue(&NSString::from_str(text));
    value.setTextColor(Some(&tone_color(tone)));
    button.setHidden(!show);
    value.setHidden(show);
    let right = ROW_W - 16.;
    let f = value.frame();
    value.setFrame(rect(right - 160., f.origin.y, 160., f.size.height));
}
unsafe fn text_button_title(b: &NSButton, title: &str) {
    let s = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &NSString::from_str(title));
    let range = NSRange::new(0, s.length());
    s.addAttribute_value_range(NSFontAttributeName, &font::weighted(11.5, 0.), range);
    s.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), range);
    b.setAttributedTitle(&s);
}
unsafe fn text_button(title: &str, frame: NSRect, tag: isize) -> Retained<NSButton> {
    let b = NSButton::buttonWithTitle_target_action(&NSString::from_str(title), None, None, mtm());
    b.setFrame(frame); b.setBordered(false); b.setTag(tag);
    b.setAlignment(NSTextAlignment::Right);
    text_button_title(&b, title);
    hand_cursor::install(&b);
    b
}
unsafe fn label(text: &str, frame: NSRect, size: f64, weight: f64, color: &NSColor) -> Retained<NSTextField> {
    let l = text_label(text, frame, size, false);
    l.setFont(Some(&font::weighted(size, weight)));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}
unsafe fn icon(i: usize) -> Option<Retained<NSImage>> {
    let bytes: &[u8] = match i {
        0 => include_bytes!("../../assets/providers/codex.png"),
        1 => include_bytes!("../../assets/providers/claude.png"),
        _ => include_bytes!("../../assets/providers/antigravity.png"),
    };
    NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(bytes))
}

unsafe fn build_row(i: usize) -> Row {
    let secondary = NSColor::secondaryLabelColor();
    let container = flipped(rect(0., i as f64 * HEAD, ROW_W, HEAD));
    let _: () = msg_send![&*layer(&container), setMasksToBounds: true];
    if i > 0 {
        let line = flipped(rect(INSET, 0., ROW_W - INSET, 0.5));
        let _: () = msg_send![&*layer(&line), setBackgroundColor: &*NSColor::colorWithSRGBRed_green_blue_alpha(0.08, 0.08, 0.16, 0.10).CGColor()];
        container.addSubview(&line);
    }
    // 머리 행 전체가 하나의 펼침 버튼이다. 글자·아이콘은 버튼 안에 두어 클릭이 버튼으로 간다.
    let header = NSButton::buttonWithTitle_target_action(&NSString::new(), None, None, mtm());
    header.setFrame(rect(0., 0., ROW_W, HEAD)); header.setBordered(false); header.setTag(i as isize);
    let header_view: &NSView = &header;
    let content = flipped(rect(0., 0., ROW_W, HEAD));
    header_view.addSubview(&content);
    let img = NSImageView::initWithFrame(NSImageView::alloc(mtm()), rect(12., 9., 30., 30.));
    if let Some(image) = icon(i) { image.setSize(NSSize::new(22., 22.)); img.setImage(Some(&image)); }
    img.setImageScaling(NSImageScaling::ScaleProportionallyDown);
    let il = layer(&img);
    let _: () = msg_send![&il, setBackgroundColor: &*NSColor::whiteColor().CGColor()];
    let _: () = msg_send![&il, setCornerRadius: 8.0f64];
    let _: () = msg_send![&il, setBorderWidth: 0.5f64];
    let _: () = msg_send![&il, setBorderColor: &*NSColor::colorWithSRGBRed_green_blue_alpha(0., 0., 0., 0.10).CGColor()];
    content.addSubview(&img);
    content.addSubview(&label(NAMES[i], rect(INSET, 6., 220., 19.), 14., 0.3, &NSColor::labelColor()));
    let dot = flipped(rect(INSET, 31., 7., 7.));
    let _: () = msg_send![&*layer(&dot), setCornerRadius: 3.5f64];
    content.addSubview(&dot);
    let status = label("", rect(INSET + 12., 25., 220., 17.), 12., 0., &secondary);
    content.addSubview(&status);
    let chevron = NSImageView::initWithFrame(NSImageView::alloc(mtm()), rect(ROW_W - 28., 16., 14., 16.));
    if let Some(s) = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str("chevron.right"), None) {
        chevron.setImage(Some(&s));
    }
    chevron.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));
    content.addSubview(&chevron);
    hand_cursor::install(&header);
    container.addSubview(&header);

    let detail = flipped(rect(0., HEAD, ROW_W, DETAIL));
    detail.setHidden(true);
    container.addSubview(&detail);
    let controls = flipped(rect(0., 0., ROW_W, DETAIL));
    detail.addSubview(&controls);
    let notice = flipped(rect(INSET, 2., ROW_W-INSET-16., 112.));
    let nl = layer(&notice);
    let _: () = msg_send![&nl, setCornerRadius: 8.0f64];
    let _: () = msg_send![&nl, setBackgroundColor: &*NSColor::colorWithSRGBRed_green_blue_alpha(0.5, 0.5, 0.55, 0.07).CGColor()];
    let nw = ROW_W-INSET-16.;
    let notice_title = label("", rect(12., 8., nw-24., 18.), 11.5, 0.3, &secondary);
    let notice_body = label("", rect(12., 29., nw-24., 34.), 11., 0., &secondary);
    notice_body.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    notice_body.setMaximumNumberOfLines(2);
    let node_manual = text_button("직접 설치하기 ↗", rect(nw-184., 65., 100., 22.), 3);
    let node_auto = text_button("자동 설치", rect(nw-78., 65., 66., 22.), 4);
    let node_check = text_button("설치 여부 확인", rect(nw-112., 87., 100., 22.), 5);
    notice.addSubview(&notice_title); notice.addSubview(&notice_body);
    for b in [&node_manual, &node_auto, &node_check] { notice.addSubview(b); }
    detail.addSubview(&notice);
    let kv = |y: f64, title: &str, tag: isize, btn: &str| {
        controls.addSubview(&label(title, rect(INSET, y, 150., 18.), 11.5, 0., &secondary));
        let value = label("", rect(0., y, 160., 18.), 11.5, 0., &secondary);
        value.setAlignment(NSTextAlignment::Right);
        controls.addSubview(&value);
        let b = text_button(btn, rect(ROW_W - 16. - 52., y - 3., 52., 24.), tag);
        controls.addSubview(&b);
        (value, b)
    };
    let (cli_value, cli_button) = kv(2., "공식 CLI", i as isize * 10 + 1, "설치");
    let spinner = NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm()), rect(ROW_W - 32., 3., 16., 16.));
    spinner.setStyle(NSProgressIndicatorStyle::Spinning);
    spinner.setDisplayedWhenStopped(false);
    spinner.setHidden(true);
    controls.addSubview(&spinner);
    let (login_value, login_button) = kv(30., ACCOUNTS[i], i as isize * 10 + 2, "로그인");
    let recheck = text_button("새로고침", rect(ROW_W - 16. - 120., 56., 120., 22.), i as isize);
    controls.addSubview(&recheck);
    Row { container, header, chevron, detail, controls, notice, notice_title, notice_body, node_auto, node_manual, node_check, dot, status, cli_value, cli_button, spinner, login_value, login_button, recheck }
}

/// 사용자가 행 안의 설치·로그인을 누르면 그 공급자와 동작을, 닫으면 None을 돌려준다.
/// 두 번째 값은 창에서 다시 확인했을 수 있는 최신 상태다.
pub unsafe fn show(state: &Value) -> (Option<(String, Action)>, Value) {
    let app = NSApplication::sharedApplication(mtm());
    let window = NSWindow::initWithContentRect_styleMask_backing_defer(
        NSWindow::alloc(mtm()), rect(0., 0., W, window_height(HEAD * 3.)),
        NSWindowStyleMask::Titled | NSWindowStyleMask::FullSizeContentView, NSBackingStoreType::Buffered, false);
    window.setReleasedWhenClosed(false);
    window.setTitle(&NSString::from_str("구독 연결"));
    theme::apply_window(&window);
    let root = flipped(rect(0., 0., W, window_height(HEAD * 3.)));
    root.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
    theme::canvas(&root);
    window.setContentView(Some(&root));

    root.addSubview(&label("구독 연결", rect(PAD + 4., 28., 300., 30.), 22., 0.56, &NSColor::labelColor()));
    root.addSubview(&label("연결한 공급자의 모델을 채팅에서 자유롭게 고를 수 있어요.", rect(PAD + 4., 64., W - PAD * 2., 18.), 12.5, 0., &NSColor::secondaryLabelColor()));

    let group = flipped(rect(PAD, GROUP_Y, ROW_W, HEAD * 3.));
    let gl = layer(&group);
    let _: () = msg_send![&gl, setBackgroundColor: &*NSColor::colorWithSRGBRed_green_blue_alpha(1., 1., 1., 0.78).CGColor()];
    let _: () = msg_send![&gl, setCornerRadius: 16.0f64];
    let _: () = msg_send![&gl, setCornerCurve: &*NSString::from_str("continuous")];
    let _: () = msg_send![&gl, setMasksToBounds: true];
    let _: () = msg_send![&gl, setBorderWidth: 0.5f64];
    let _: () = msg_send![&gl, setBorderColor: &*NSColor::colorWithSRGBRed_green_blue_alpha(0.08, 0.08, 0.20, 0.08).CGColor()];
    root.addSubview(&group);
    let rows: Vec<Row> = (0..3).map(|i| build_row(i)).collect();
    for r in &rows { group.addSubview(&r.container); }

    // 안내문과 닫기는 창 아래에 붙어 창 높이가 바뀌면 함께 움직인다(뒤집힌 좌표에서 MinY = 위쪽 여백).
    let bottom = window_height(HEAD * 3.);
    let note = label("원문과 이미지는 선택한 모델의 공급자에게 전송됩니다. 로그인은 공식 CLI가 관리하며 API 과금으로 자동 전환하지 않습니다.",
        rect(PAD + 4., bottom - 16. - 34., W - PAD * 2. - 96., 34.), 11., 0., &NSColor::secondaryLabelColor());
    note.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    note.setMaximumNumberOfLines(2);
    note.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
    root.addSubview(&note);
    let close = NSButton::buttonWithTitle_target_action(&NSString::from_str("닫기"), None, None, mtm());
    close.setFrame(rect(W - PAD - 76., bottom - 16. - 30., 76., 28.));
    close.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin);
    theme::button(&close, false);
    close.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    hand_cursor::install(&close);
    root.addSubview(&close);

    let target: Retained<Controller> = msg_send![super(Controller::alloc(mtm()).set_ivars(Data {
        state: RefCell::new(state.clone()), open: Cell::new(None), result: Cell::new(None), group, rows, installing: RefCell::new(None), node_installing: Cell::new(false),
    })), init];
    if active_job(&state["setup_job"]) {
        if let Some(id) = state["setup_job"]["id"].as_str() {
            *target.ivars().installing.borrow_mut() = Some((0, id.to_owned()));
            target.ivars().node_installing.set(true);
            target.ivars().open.set(Some(0));
            schedule(&target, sel!(pollInstall:), 0.5);
        }
    }
    for r in &target.ivars().rows {
        r.header.setTarget(Some(&target)); r.header.setAction(Some(sel!(toggle:)));
        for b in [&r.cli_button, &r.login_button] { b.setTarget(Some(&target)); b.setAction(Some(sel!(act:))); }
        for b in [&r.node_auto, &r.node_manual, &r.node_check] { b.setTarget(Some(&target)); b.setAction(Some(sel!(nodeAction:))); }
        r.recheck.setTarget(Some(&target)); r.recheck.setAction(Some(sel!(recheck:)));
    }
    close.setTarget(Some(&target)); close.setAction(Some(sel!(close:)));
    target.layout(false);
    window.center(); window.makeKeyAndOrderFront(None);
    app.runModalForWindow(&window);
    window.orderOut(None);
    let _: () = msg_send![objc2::class!(NSObject), cancelPreviousPerformRequestsWithTarget: &*target];
    let picked = target.ivars().result.get().map(|(i, a)| (IDS[i].to_string(), a));
    let latest = target.ivars().state.borrow().clone();
    (picked, latest)
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn prerequisite_notice_only_for_affected_provider() {
        let mut v=json!({"node":{"ready":false},"providers":[{"provider":"codex","installed":false},{"provider":"claude","installed":false}]});
        assert!(show_notice(&v,0)); assert!(!show_notice(&v,1));
        assert_eq!(summary(&v,"codex").0,"준비 환경 확인 필요");
        v["node"]["ready"]=json!(true); assert!(!show_notice(&v,0));
        v["setup_job"]=json!({"state":"running"}); assert!(show_notice(&v,0));
        assert_eq!(summary(&v,"codex").0,"Node.js 설치 중");
        v["setup_job"]["state"]=json!("succeeded"); assert!(!show_notice(&v,0));
        v["providers"][0]["execution_error"]=json!(true); assert!(show_notice(&v,0));
        assert_eq!(summary(&v,"codex").0,"CLI 실행 확인 필요");
    }
    #[test] fn missing_state_is_unknown_not_failure() {
        assert_eq!(summary(&json!({}), "codex"), ("미확인", Tone::Idle));
        let (cli, cli_btn, login, login_btn) = details(&json!({}), "codex");
        assert_eq!((cli.0, cli_btn, login.0, login_btn), ("미확인", false, "미확인", false));
    }
    #[test] fn connected_summary_includes_all_and_excludes_unverified() {
        let state = json!({"providers":[
            {"provider":"claude","installed":true,"logged_in":true},
            {"provider":"codex","installed":true,"logged_in":true},
            {"provider":"antigravity","installed":true,"logged_in":false}
        ]});
        assert_eq!(connected_names(&state).join(" · "), "Codex · Claude");
        assert!(connected_names(&json!({})).is_empty());
        assert!(connected_names(&json!({"providers":[{"provider":"codex","logged_in":true}]})).is_empty());
    }
    #[test] fn actions_only_where_needed() {
        let v = json!({"providers":[{"provider":"codex","installed":true,"logged_in":true},{"provider":"claude","installed":true,"logged_in":false},{"provider":"antigravity","installed":false}]});
        assert_eq!(summary(&v, "codex").0, "연결됨");
        let (_, a, _, b) = details(&v, "codex"); assert!(!a && !b);
        let (_, a, login, b) = details(&v, "claude"); assert!(!a && b); assert_eq!(login.0, "로그인 필요");
        // 설치 전에는 로그인 버튼을 주지 않는다.
        let (cli, a, login, b) = details(&v, "antigravity"); assert!(a && !b); assert_eq!((cli.0, login.0), ("설치 필요", "미확인"));
        assert_eq!(first_connected(&v).as_deref(), Some("codex"));
    }
}
