#![allow(deprecated)]
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{define_class, msg_send, sel, AnyThread, ClassType, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::{json, Value};
use std::os::unix::process::CommandExt;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, Ordering},
    mpsc, Arc,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
mod ai_settings;
mod chat_panel;
mod chat_messages;
mod chat_rich_text;
mod chat_attachments;
mod diagram_canvas;
mod document_tree;
mod editor;
mod font;
mod graph;
mod graph_controls;
mod ontology_cleanup;
mod hand_cursor;
mod provider_dialog;
mod pdf_annotations;
mod pdf_save;
mod pdf_session;
mod organize_panel;
mod pptx_preview;
mod save_dialog;
mod scrollbar;
mod shortcut_panel;
mod sidebar_layout;
mod sidebar_resize;
mod sidebar_hide;
mod sidebar_reveal;
mod theme;
mod updater;
mod window_appearance;

const TITLE_DIVIDER_INSET: f64 = 14.;

define_class!(
    #[unsafe(super(NSTextField, NSControl, NSView, NSResponder))]
    #[name = "StologyNoteTitleField"]
    #[thread_kind = MainThreadOnly]
    struct NoteTitleField;
    impl NoteTitleField {
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            if self.isEditable() {
                self.setPlaceholderString(None);
                STATE.with(|state| {
                    if let Ok(mut state) = state.try_borrow_mut() {
                        if let Some(s) = state.as_mut() { s.editing_new_title = true; }
                    }
                });
            }
            let _: () = msg_send![super(self), mouseDown: event];
        } }
    }
);

thread_local! { static WORKSPACE_COMMAND: Cell<i8> = const { Cell::new(0) }; static TITLE_COMMAND: Cell<i8> = const { Cell::new(0) }; static DESC_COMMAND: Cell<i8> = const { Cell::new(0) }; static STATE: RefCell<Option<State>> = const {RefCell::new(None)}; static PENDING_QUIT:Cell<bool>=const{Cell::new(false)}; static PDF_QUIT_PENDING:Cell<bool>=const{Cell::new(false)}; }

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    struct Delegate;
    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, app: &NSApplication) -> NSApplicationTerminateReply {
            let ready = STATE.with(|state| {
                let Ok(mut state) = state.try_borrow_mut() else { return None; };
                Some(state.as_mut().map(|s| unsafe {
                    if !s.discard_diagram(){return Err("다이어그램 편집을 계속합니다".to_owned());}
                    s.poll_pdf_sessions();
                    if !s.chat.prepare_quit()? { return Ok(false); }
                    if !s.flush_body() || !s.finish_new_title(true) { return Err("현재 노트를 저장하지 못했습니다".to_owned()); }
                    pdf_session::request_flush_all();
                    if s.annotation_publish.is_some() { Ok(false) } else { pdf_session::flush_state() }
                }).unwrap_or(Ok(true)))
            });
            match ready {
                Some(Ok(true)) => {
                    STATE.with(|s| if let Some(s) = s.borrow_mut().as_mut() { unsafe { s.cancel_background_work(); } });
                    NSApplicationTerminateReply::TerminateNow
                }
                Some(Err(error)) => {
                    STATE.with(|s| if let Some(s) = s.borrow_mut().as_mut() { unsafe { s.set_status(&format!("종료 보류 · {error}")); } });
                    NSApplicationTerminateReply::TerminateCancel
                }
                pending => unsafe {
                    if !PENDING_QUIT.with(|p| p.replace(true)) {
                        let timer: Retained<NSTimer> = msg_send![NSTimer::class(), timerWithTimeInterval:0.05f64, target:self, selector:sel!(drainQuit:), userInfo:std::ptr::null::<AnyObject>(), repeats:true];
                        let runloop = NSRunLoop::mainRunLoop();
                        runloop.addTimer_forMode(&timer, NSDefaultRunLoopMode);
                        runloop.addTimer_forMode(&timer, NSModalPanelRunLoopMode);
                    }
                    if pending.is_some() {
                        PDF_QUIT_PENDING.with(|p| p.set(true));
                        NSApplicationTerminateReply::TerminateLater
                    } else {
                        app.abortModal();
                        NSApplicationTerminateReply::TerminateCancel
                    }
                }
            }
        }
        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn close_app(&self, _app:&NSApplication)->bool {true}
    }
    unsafe impl NSControlTextEditingDelegate for Delegate{
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn search_command(&self,control:&NSControl,text:&NSTextView,selector:Sel)->bool{(||{
            if control.identifier().is_some_and(|id| id.to_string()=="workspace-rename") {
                let marked:bool=msg_send![text,hasMarkedText]; if marked{return false;}
                if selector==sel!(insertNewline:) { WORKSPACE_COMMAND.with(|c|c.set(1)); return true; }
                if selector==sel!(cancelOperation:) { WORKSPACE_COMMAND.with(|c|c.set(-1)); return true; }
                return false;
            }
            if control.identifier().is_some_and(|id| id.to_string()=="new-note-title") {
                let marked:bool=msg_send![text,hasMarkedText]; if marked{return false;}
                if selector==sel!(insertNewline:) { TITLE_COMMAND.with(|c|c.set(1)); return true; }
                if selector==sel!(cancelOperation:) { TITLE_COMMAND.with(|c|c.set(-1)); return true; }
                return false;
            }
            if control.identifier().is_some_and(|id| id.to_string()=="concept-desc") {
                let marked:bool=msg_send![text,hasMarkedText]; if marked{return false;}
                if selector==sel!(insertNewline:) { DESC_COMMAND.with(|c|c.set(1)); return true; }
                if selector==sel!(cancelOperation:) { DESC_COMMAND.with(|c|c.set(-1)); return true; }
                return false;
            }
            if selector!=sel!(cancelOperation:){return false;}let marked:bool=msg_send![text,hasMarkedText];if marked{return false;}
            STATE.with(|state|{let Ok(mut state)=state.try_borrow_mut()else{return false;};let Some(s)=state.as_mut()else{return false;};let is_file_search=s.file_search.as_ref().is_some_and(|field|Retained::as_ptr(field)as *const AnyObject==control as *const NSControl as *const AnyObject);if is_file_search{text.setString(&NSString::from_str(""));if let Some(field)=&s.file_search{field.setStringValue(&NSString::from_str(""));}return true;}let is_search=s.search.as_ref().is_some_and(|field|Retained::as_ptr(field)as *const AnyObject==control as *const NSControl as *const AnyObject);if !is_search{return false;}text.setString(&NSString::from_str(""));if let Some(field)=&s.search{field.setStringValue(&NSString::from_str(""));}s.last_query.clear();graph::search("");s.set_status("개념 검색을 지웠습니다");true})
        })()}
    }
    unsafe impl NSTextFieldDelegate for Delegate{
        #[unsafe(method(controlTextDidBeginEditing:))]
        unsafe fn title_editing_began(&self, notification: &NSNotification) {
            let Some(object) = notification.object() else { return; };
            if object.downcast_ref::<NSControl>().and_then(|c| c.identifier()).is_some_and(|id| id.to_string() == "concept-desc") {
                STATE.with(|state| if let Ok(mut state) = state.try_borrow_mut() { if let Some(s) = state.as_mut() { s.concept_desc_began(); } });
                return;
            }
            STATE.with(|state| {
                let Ok(mut state) = state.try_borrow_mut() else { return; };
                if let Some(s) = state.as_mut() {
                    if Retained::as_ptr(&object) == Retained::as_ptr(&s.header).cast() {
                        s.editing_new_title = true;
                        s.header.setPlaceholderString(None);
                    }
                }
            });
        }
    }
    impl Delegate {
        #[unsafe(method(workspaceContextWillOpen:))]
        fn workspace_context(&self, row: &NSControl) { unsafe { workspace_context_will_open(row.tag()); } }
        #[unsafe(method(controlTextDidEndEditing:))]
        unsafe fn concept_desc_ended(&self, notification: &NSNotification) {
            let Some(object) = notification.object() else { return; };
            if object.downcast_ref::<NSControl>().and_then(|c| c.identifier()).is_some_and(|id| id.to_string() == "concept-desc") {
                // Leaving the field without Enter closes it without saving.
                DESC_COMMAND.with(|c| if c.get() == 0 { c.set(-2) });
            }
        }
    }
    unsafe impl NSSearchFieldDelegate for Delegate{}
    unsafe impl NSWindowDelegate for Delegate {
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            let divider = STATE.with(|state| state.try_borrow().ok()
                .and_then(|s| s.as_ref().map(|s| [s.left_resizer.clone(), s.right_resizer.clone()])));
            if let Some(dividers) = divider { for divider in dividers { divider.cancel(); } }
        }
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, _notification: &NSNotification) {
            // The default-mode 200ms work timer pauses during AppKit's resize
            // tracking loop. Geometry must follow the window synchronously.
            STATE.with(|state| if let Ok(mut state) = state.try_borrow_mut() {
                if let Some(s) = state.as_mut() { unsafe {
                    s.layout_frames(s.root.bounds().size);
                } }
            });
        }
        #[unsafe(method(windowDidEndLiveResize:))]
        fn window_did_end_live_resize(&self, _notification: &NSNotification) {
            STATE.with(|state| if let Ok(mut state) = state.try_borrow_mut() {
                if let Some(s) = state.as_mut() { unsafe {
                    s.layout(s.root.bounds().size);
                } }
            });
        }
        #[unsafe(method(windowShouldClose:))]
        fn window_should_close(&self, _window:&NSWindow) -> bool {
            NSApplication::sharedApplication(mtm()).terminate(None);
            false
        }
    }
    impl Delegate {
        #[unsafe(method(drainQuit:))]
        fn drain_quit(&self, timer:&NSTimer) { unsafe {
            if !PENDING_QUIT.with(Cell::get) { timer.invalidate(); return; }
            let app = NSApplication::sharedApplication(mtm());
            if PDF_QUIT_PENDING.with(Cell::get) {
                let result = STATE.with(|state| {
                    let Ok(mut state) = state.try_borrow_mut() else { return None; };
                    if let Some(s) = state.as_mut() {
                        let ws = s.workspace.clone();
                        s.chat.tick(ws.as_deref());
                        match s.chat.prepare_quit() { Ok(true) => {}, other => return Some(other), }
                        s.poll_pdf_sessions();
                        s.poll_annotation_conversion();
                        if s.annotation_publish.is_some() { return Some(Ok(false)); }
                    }
                    Some(pdf_session::flush_state())
                });
                match result {
                    Some(Ok(true)) => {
                        STATE.with(|s| if let Some(s) = s.borrow_mut().as_mut() { s.cancel_background_work(); });
                        PENDING_QUIT.with(|p| p.set(false)); PDF_QUIT_PENDING.with(|p| p.set(false));
                        timer.invalidate(); app.replyToApplicationShouldTerminate(true);
                    }
                    Some(Err(error)) => {
                        STATE.with(|s| if let Some(s) = s.borrow_mut().as_mut() { s.set_status(&format!("종료 보류 · PDF 저장 실패: {error}")); });
                        PENDING_QUIT.with(|p| p.set(false)); PDF_QUIT_PENDING.with(|p| p.set(false));
                        timer.invalidate(); app.replyToApplicationShouldTerminate(false);
                    }
                    _ => {}
                }
            } else if STATE.with(|s| s.try_borrow_mut().is_ok()) {
                PENDING_QUIT.with(|p| p.set(false)); timer.invalidate(); app.terminate(None);
            } else { app.abortModal(); }
        }}
        #[unsafe(method(action:))]
        fn action(&self,sender:&AnyObject) { let tag:isize=unsafe{msg_send![sender,tag]}; STATE.with(|s|if let Ok(mut s)=s.try_borrow_mut(){if let Some(s)=s.as_mut(){unsafe{s.action(tag)}}}); }
        #[unsafe(method(newDocument:))]
        fn new_document(&self,_sender:Option<&AnyObject>) {STATE.with(|s|if let Ok(mut s)=s.try_borrow_mut(){if let Some(s)=s.as_mut(){unsafe{if s.flush(){if s.map.is_some(){s.close_map();}s.show_blank();s.refresh_right();s._window.makeFirstResponder(Some(&s.text));}}}});}
        #[unsafe(method(saveDocument:))]
        fn save_document(&self,_sender:Option<&AnyObject>) {STATE.with(|s|if let Ok(mut s)=s.try_borrow_mut(){if let Some(s)=s.as_mut(){unsafe{s.save()}}});}
        #[unsafe(method(shareDocument:))]
        fn share_document(&self, _sender:Option<&AnyObject>) { STATE.with(|s| if let Ok(mut s)=s.try_borrow_mut() { if let Some(s)=s.as_mut() { unsafe{s.share_pdf();} } }); }
        #[unsafe(method(resolvePdfSave:))]
        fn resolve_pdf_save(&self, _sender:Option<&AnyObject>) { STATE.with(|s| if let Ok(mut s)=s.try_borrow_mut() { if let Some(s)=s.as_mut() { unsafe{s.resolve_pdf_save();} } }); }
        #[unsafe(method(focusMapSearch:))]
        fn focus_map_search(&self,_sender:Option<&AnyObject>){unsafe{
            // Cmd+K adds a link when document text is selected. Keep map search
            // available otherwise, and release STATE before entering a modal.
            let text = STATE.with(|state| state.try_borrow().ok().and_then(|s| s.as_ref().and_then(|s| {
                let responder: *mut AnyObject = msg_send![&*s._window, firstResponder];
                (responder == Retained::as_ptr(&s.text) as *mut AnyObject).then(|| s.text.clone())
            })));
            if text.as_ref().is_some_and(|text| editor::add_link(text)) { return; }
            STATE.with(|s|if let Ok(mut s)=s.try_borrow_mut(){if let Some(s)=s.as_mut(){if let Some(field)=&s.search{s._window.makeFirstResponder(Some(field));field.selectText(None);}}});
        }}
        #[unsafe(method(animateSidebar:))]
        fn animate_sidebar(&self, _timer: &NSTimer) {
            STATE.with(|s| if let Ok(mut s) = s.try_borrow_mut() {
                if let Some(s) = s.as_mut() { unsafe { s.animate_sidebar(); } }
            });
        }
        #[unsafe(method(tick:))]
        fn tick(&self,_timer:&NSTimer) {STATE.with(|s| if let Ok(mut s)=s.try_borrow_mut() {if let Some(s)=s.as_mut(){unsafe{s.tick()}}});}
    }
);

#[derive(Clone)]
enum Action {
    Workspace(String),
    Open(Value),
    Node(String),
    Evidence(Value),
    NewWorkspace,
    RenameWorkspace(String, String),
    DeleteWorkspace(String, String),
    DeleteDocument(Value),
    Save,
    Delete,
    ToggleMap,
    ToggleShortcuts,
    CloseShortcuts,
    ToggleSidebar(usize),
    RightMode(bool),
    Import,
    Connect,
    Cancel,
    Fit,
    Focus,
    ZoomIn,
    ZoomOut,
    Rotate,
    Deselect,
    Search,
    Relation,
    ClearSearch,
    ToggleNodes,
    MoreNodes,
    MoreNeighbors,
    AiSettings,
    Organize,
    OrganizePause,
    OrganizeResume,
    OrganizeDismiss,
    OrganizeHow,
    Unit(String),
    ExpandLink(String),
    CleanupReview,
    DeleteLink(Value),
    HideLink(Value, bool),
    Unmerge(String),
    RestoreDescription(String),
    Reread(String, String),
    /// 정리 제안: register · edit · ask_dismiss · dismiss · draft, 그리고 제안 ID.
    Suggest(&'static str, String),
    /// 검색바 옆 ✦ 버튼: 정리 제안 창을 열고 닫는다.
    SuggestToggle,
    SuggestClose,
    SuggestIntroSeen,
}
struct Pending {
    receiver: mpsc::Receiver<Result<Value, String>>,
    pid: Arc<AtomicI32>,
    cancelled: Arc<AtomicBool>,
}
impl Pending {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        let pid = self.pid.load(Ordering::SeqCst);
        if pid > 0 {
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
        if self
            .receiver
            .recv_timeout(Duration::from_millis(800))
            .is_err()
        {
            let active = self.pid.load(Ordering::SeqCst);
            if active > 0 {
                unsafe {
                    libc::kill(-active, libc::SIGKILL);
                }
            }
            let _ = self.receiver.recv_timeout(Duration::from_secs(2));
        }
        let _ = rpc(json!({"op":"analysis.cancel_active"}));
    }
}
struct WorkspaceScreen {
    blank_title: String,
    file_query: String,
    document: Option<Value>,
    evidence: Option<Value>,
    selection: NSRange,
    scroll: NSPoint,
    map_open: bool,
    folders: HashSet<String>,
    pdf_page: Option<usize>,
}

unsafe fn pdf_view(view: &NSView) -> Option<Retained<NSView>> {
    if view.isKindOfClass(objc2::class!(PDFView)) {
        return Some(objc2::Message::retain(view));
    }
    view.subviews().iter().find_map(|child| pdf_view(&child))
}

struct State {
    screens: std::collections::HashMap<String, WorkspaceScreen>,
    current_evidence: Option<Value>,
    target: Retained<Delegate>,
    workspace_rows: Vec<(String, Retained<NSView>, Retained<NSButton>)>,
    workspace_rename: Option<(String, String, Retained<NSTextField>)>,
    _window: Retained<NSWindow>,
    root: Retained<NSView>,
    left: Retained<NSView>,
    left_shell: Retained<NSView>,
    right_shell: Retained<NSView>,
    right_content: Retained<NSView>,
    right: Retained<NSView>,
    center: Retained<NSView>,
    workspace_welcome: Retained<NSView>,
    chat: chat_panel::Panel,
    chat_mode: bool,
    right_tabs: [Retained<NSButton>; 2],
    right_tab_group: Retained<NSView>,
    right_tab_selection: Retained<NSView>,
    sidebar_handles: [Retained<NSButton>; 2],
    sidebar_open: [bool; 2],
    left_width: f64,
    left_drag_offset: f64,
    left_drag_pending: bool,
    left_hide_veil: Retained<NSView>,
    left_hide_arrow: Retained<NSView>,
    left_resizer: Retained<sidebar_resize::Divider>,
    shortcuts: Option<Retained<NSView>>,
    right_width: f64,
    right_drag_offset: f64,
    right_drag_pending: bool,
    right_hide_veil: Retained<NSView>,
    right_hide_arrow: Retained<NSView>,
    right_resizer: Retained<sidebar_resize::Divider>,
    sidebar_openness: [f64; 2],
    sidebar_motion: Option<sidebar_layout::Motion>,
    sidebar_timer: Option<Retained<NSTimer>>,
    header: Retained<NSTextField>,
    analysis_help: Retained<NSTextField>,
    editing_new_title: bool,
    blank_title: String,
    title_divider: Retained<NSView>,
    subtitle: Retained<NSTextField>,
    status: Retained<NSTextField>,
    ai_spinner: Retained<NSProgressIndicator>,
    diagram: Option<diagram_canvas::Canvas>,
    save_button: Retained<NSButton>,
    delete_button: Retained<NSButton>,
    map_button: Retained<NSButton>,
    editor_scroll: Retained<NSScrollView>,
    text: Retained<NSTextView>,
    preview: Option<Retained<NSView>>,
    preview_pending: Option<(pptx_preview::Handle, String)>,
    preview_rendered: bool,
    preview_editable: bool,
    annotation_conversion: Option<(pptx_preview::Handle, Value, bool)>,
    annotation_publish: Option<mpsc::Receiver<Result<Value, String>>>,
    analysis_waiting: Option<Value>,
    deferred_tree: Option<document_tree::Event>,
    deferred_action: Option<Action>,
    last_pdf_status: String,
    share_waiting: Option<String>,
    share_picker: Option<Retained<NSSharingServicePicker>>,
    preview_locator: Option<Value>,
    map: Option<Retained<NSView>>,
    search: Option<Retained<NSTextField>>,
    search_clear: Option<Retained<NSButton>>,
    file_search: Option<Retained<NSSearchField>>,
    last_file_query: String,
    last_query: String,
    nodes_open: bool,
    node_limit: usize,
    expanded_link: String,
    neighbor_limit: usize,
    desc_field: Option<Retained<NSTextField>>,
    desc_hint: Option<Retained<NSTextField>>,
    desc_concept: String,
    desc_plain: String,
    map_preferences: std::collections::HashMap<String, Value>,
    relation: Option<Retained<NSPopUpButton>>,
    cleanup_review: Option<(String,mpsc::Receiver<Result<Value,String>>)>,
    graph_data: Value,
    selection: Value,
    workspace: Option<String>,
    document: Option<Value>,
    last_text: String,
    last_selection: NSRange,
    last_editor_focus: bool,
    dirty: bool,
    edited_at: Instant,
    actions: Vec<Option<Action>>,
    open_folders: HashSet<String>,
    document_tree: Option<Retained<document_tree::DocumentTree>>,
    tree_refresh_pending: bool,
    job: Option<String>,
    last_job_poll: Instant,
    last_dimensions: NSSize,
    provider: String,
    snapshot: bool,
    connection_job: Option<String>,
    /// 마지막 `provider.status` 결과. CLI를 실행하므로 백그라운드에서만 읽는다.
    provider_info: Option<Value>,
    connection_installing: bool,
    connection_node: bool,
    provider_probe: Option<mpsc::Receiver<Value>>,
    provider_probe_again: bool,
    connect_button: Option<Retained<NSButton>>,
    connect_help: Option<Retained<NSTextField>>,
    pending: Option<Pending>,
    /// 열린 과목의 마지막 과목 정리 실행 기록.
    run: Option<Value>,
    organize_label: Option<Retained<NSTextField>>,
    /// 이 과목의 정리 제안과 뒤에서 도는 정리 상태.
    suggestions: Value,
    suggestions_for: Option<String>,
    last_suggestion_poll: Instant,
    tab_dot: Retained<NSView>,
    /// 검색바 옆 ✦ 버튼과 그 버튼에서 여는 정리 제안 창.
    suggest_button: Option<Retained<NSButton>>,
    suggest_open: bool,
    /// 삭제를 한 번 더 묻고 있는 제안 ID.
    suggest_confirm: Option<String>,
    /// 창 바깥 클릭을 받는 막과 창 자체. 다시 그릴 때 지운다.
    suggest_layer: Vec<Retained<NSView>>,
    unit_focus: String,
}
fn document_base(doc: &Value) -> Option<&std::path::Path> {
    doc["absolute_path"]
        .as_str()
        .and_then(|p| std::path::Path::new(p).parent())
        .or_else(|| doc["image_base"].as_str().map(std::path::Path::new))
}
fn import_image(bytes: &[u8], extension: &str) -> Result<(String, std::path::PathBuf), String> {
    let (workspace, folder, base) = STATE.with(|state| {
        let state = state
            .try_borrow()
            .map_err(|_| "편집이 끝난 뒤 다시 붙여넣어 주세요".to_owned())?;
        let s = state
            .as_ref()
            .ok_or_else(|| "노트를 먼저 열어 주세요".to_owned())?;
        let workspace = s
            .workspace
            .clone()
            .ok_or_else(|| "과목을 먼저 선택해 주세요".to_owned())?;
        let folder = s
            .document
            .as_ref()
            .and_then(|d| d["parent"].as_str())
            .unwrap_or("")
            .to_owned();
        let base = s
            .document
            .as_ref()
            .and_then(document_base)
            .map(std::path::Path::to_path_buf);
        Ok::<_, String>((workspace, folder, base))
    })?;
    let mut input = tempfile::Builder::new()
        .suffix(&format!(".{extension}"))
        .tempfile()
        .map_err(|e| e.to_string())?;
    input.write_all(bytes).map_err(|e| e.to_string())?;
    let result = rpc(
        json!({"op":"note.image.import","workspace_id":workspace,"folder":folder,"source":input.path()}),
    )?;
    Ok((
        strv(&result, "destination"),
        base.unwrap_or_else(|| std::path::PathBuf::from(strv(&result, "image_base"))),
    ))
}
/// 왼쪽 워크스페이스 행 간격. 행 높이 40pt + 여백 6pt.
const WORKSPACE_ROW_STEP: f64 = 46.;
fn rpc(value: Value) -> Result<Value, String> {
    rpc_inner(value, None)
}
fn rpc_inner(
    value: Value,
    pid: Option<(Arc<AtomicI32>, Arc<AtomicBool>)>,
) -> Result<Value, String> {
    let exe = std::env::var_os("STOLOGY_CORE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .with_file_name("stology-core")
        });
    let mut cmd = Command::new(exe);
    cmd.arg("--request")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if pid.is_some() {
        unsafe {
            cmd.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("로컬 저장 엔진을 실행할 수 없습니다: {e}"))?;
    if let Some((p, c)) = &pid {
        p.store(child.id() as i32, Ordering::SeqCst);
        if c.load(Ordering::SeqCst) {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGTERM);
            }
        }
    }
    child
        .stdin
        .take()
        .unwrap()
        .write_all(value.to_string().as_bytes())
        .map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if let Some((p, _)) = &pid {
        p.store(0, Ordering::SeqCst);
    }
    let result: Value = serde_json::from_slice(&out.stdout).map_err(|_| {
        format!(
            "저장 엔진 응답 오류: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    })?;
    if result["ok"].as_bool() == Some(true) {
        Ok(result["result"].clone())
    } else {
        Err(result["error"]
            .as_str()
            .unwrap_or("처리에 실패했습니다")
            .to_owned())
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn strv(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().unwrap()
}
unsafe fn view(frame: NSRect) -> Retained<NSView> {
    NSView::initWithFrame(NSView::alloc(mtm()), frame)
}
unsafe fn text_label(text: &str, frame: NSRect, size: f64, bold: bool) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm());
    label.setFrame(frame);
    let font = if bold {
        font::bold(size)
    } else {
        font::regular(size)
    };
    label.setFont(Some(&font));
    label.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.09, 0.10, 0.16, 1.,
    )));
    label
}
/// A wrapped label hung below `y` (its top edge); `y` moves down past it.
unsafe fn wrapped(
    parent: &NSView,
    text: &str,
    x: f64,
    y: &mut f64,
    width: f64,
    size: f64,
    bold: bool,
    lines: isize,
) -> Retained<NSTextField> {
    let label = text_label(text, rect(x, 0., width, size + 6.), size, bold);
    label.setMaximumNumberOfLines(lines);
    label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    let wanted = label
        .cell()
        .map(|cell| cell.cellSizeForBounds(rect(0., 0., width, 10000.)).height)
        .unwrap_or(size + 6.);
    let height = wanted.ceil().clamp(size + 6., lines as f64 * (size + 6.));
    *y -= height;
    label.setFrame(rect(x, *y, width, height));
    *y -= 5.;
    parent.addSubview(&label);
    label
}
const ORGANIZE_STRIP: f64 = 76.;
/// 뷰 아래에서 스크롤 뷰를 찾는다.
unsafe fn find_scroll(v: &NSView) -> Option<Retained<NSScrollView>> {
    for child in v.subviews().iter() {
        if child.isKindOfClass(NSScrollView::class()) {
            return Some(Retained::cast_unchecked(child));
        }
        if let Some(found) = find_scroll(&child) {
            return Some(found);
        }
    }
    None
}
fn organize_entries(tree: &Value) -> impl Iterator<Item = &Value> {
    tree["entries"].as_array().into_iter().flatten().filter(|e| {
        e["missing"] != true && matches!(e["kind"].as_str(), Some("md" | "pdf" | "pptx" | "png" | "jpg" | "jpeg" | "heic"))
    })
}

/// 정리 제안 창의 너비와 최대 높이.
const SUGGEST_POP_WIDTH: f64 = 300.;
const SUGGEST_POP_MAX: f64 = 640.;
fn suggestion_note(status: &Value) -> Option<String> {
    status["error"].as_str().filter(|e| !e.is_empty()).map(|e| format!("정리하지 못했어요 · {e}")).or_else(|| status["notice"].as_str().map(str::to_owned))
}
/// 제안의 종류 꼬리표: 내가 요청한 초안, AI가 알아서 만든 초안, 아직 초안이 아닌 후보.
fn suggestion_chip(s: &Value) -> &'static str {
    match (s["state"].as_str(), s["automatic"] == true) {
        (Some("draft"), false) => "초안",
        (Some("draft"), true) => "AI 초안",
        _ => "정리 전",
    }
}
/// 꼬리표 뒤의 설명. 버튼 이름 대신 사용자가 한 일과 AI가 한 일을 말한다.
fn suggestion_text(s: &Value) -> String {
    let times = match s["repeats"].as_i64().unwrap_or(2) {
        2 => "두 번".to_owned(),
        3 => "세 번".to_owned(),
        4 => "네 번".to_owned(),
        n => format!("{n}번"),
    };
    let what = match s["signal"].as_str().unwrap_or("") {
        "marked" => "표시해 둔 답변".to_owned(),
        "repeated" => format!("{times} 물어본 내용"),
        "topic" => "이어서 물어본 내용".to_owned(),
        "copied" => "복사해 둔 답변".to_owned(),
        _ => "과목 개념을 물어본 답변".to_owned(),
    };
    match suggestion_chip(s) {
        "초안" => "AI 답변을 정리해 달라고 해서 만든 노트예요".to_owned(),
        "AI 초안" if s["signal"] == "repeated" || s["signal"] == "topic" => format!("{what}이라 AI가 노트로 만들었어요"),
        "AI 초안" => format!("{what}을 AI가 노트로 만들었어요"),
        _ => format!("{what}이에요. 노트로 남길까요?"),
    }
}
fn suggest_color(r: f64, g: f64, b: f64, a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}
/// 둥근 바탕을 칠한다. 정리 제안 창의 꼬리표·안내 상자·구분선이 쓴다.
unsafe fn rounded(v: &NSView, fill: &NSColor, radius: f64) {
    v.setWantsLayer(true);
    let l: Retained<AnyObject> = msg_send![v, layer];
    let _: () = msg_send![&l, setBackgroundColor: &*fill.CGColor()];
    let _: () = msg_send![&l, setCornerRadius: radius];
}
/// 위에서 아래로 쌓는(뒤집힌) 부모에 여러 줄 글을 놓는다. 높이는 글에 맞춘다.
unsafe fn flow_label(parent: &NSView, text: &str, x: f64, y: f64, width: f64, size: f64, bold: bool, lines: isize) -> Retained<NSTextField> {
    let label = text_label(text, rect(x, y, width, size + 6.), size, bold);
    label.setMaximumNumberOfLines(lines);
    label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    let height = label.cell().map(|c| c.cellSizeForBounds(rect(0., 0., width, 10000.)).height.ceil()).unwrap_or(size + 6.);
    label.setFrame(rect(x, y, width, height.min(lines as f64 * (size + 6.))));
    parent.addSubview(&label);
    label
}
/// 첫 줄만 들여 써서 앞에 놓인 꼬리표와 겹치지 않게 한다.
unsafe fn first_line_indent(label: &NSTextField, indent: f64) {
    let text: Retained<NSMutableAttributedString> = msg_send![&label.attributedStringValue(), mutableCopy];
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setFirstLineHeadIndent(indent);
    paragraph.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
    text.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, NSRange::new(0, text.length()));
    label.setAttributedStringValue(&text);
}
/// 초안의 제목과 내용을 고친다. `(제목, 내용, 바로 등록)`.
unsafe fn release_actions(v: &NSView, actions: &mut Vec<Option<Action>>) {
    let is_control: bool = msg_send![v,isKindOfClass:NSControl::class()];
    if is_control {
        if let Some(menu) = v.menu() {
            for item in menu.itemArray().iter() {
                if item.action() == Some(sel!(action:)) && item.tag() >= 3 {
                    if let Some(slot) = actions.get_mut(item.tag() as usize) { *slot = None; }
                }
            }
        }
        let tag: isize = msg_send![v, tag];
        if tag >= 3 {
            if let Some(a) = actions.get_mut(tag as usize) {
                *a = None;
            }
        }
    }
    for child in v.subviews().iter() {
        release_actions(&child, actions);
    }
}
unsafe fn clear(v: &NSView, actions: &mut Vec<Option<Action>>) {
    for x in v.subviews().iter() {
        release_actions(&x, actions);
        x.removeFromSuperview();
    }
}
unsafe fn open_tree_item_in_finder(path: &std::path::Path) {
    let Ok(path) = std::path::absolute(path) else {
        alert("Finder를 열 수 없습니다", "항목의 저장 경로를 확인해 주세요.");
        return;
    };
    let file_type = std::fs::symlink_metadata(&path).ok().map(|entry| entry.file_type());
    let finder = NSWorkspace::sharedWorkspace();
    let path = NSString::from_str(&path.to_string_lossy());
    let opened = match file_type {
        Some(kind) if kind.is_dir() => finder.selectFile_inFileViewerRootedAtPath(None, &path),
        Some(kind) if kind.is_file() => {
            finder.selectFile_inFileViewerRootedAtPath(Some(&path), &NSString::from_str(""))
        }
        _ => {
            alert("Finder를 열 수 없습니다", "항목의 저장 위치를 찾을 수 없습니다.");
            return;
        }
    };
    if !opened {
        alert("Finder를 열 수 없습니다", "Finder에서 항목의 저장 위치를 열지 못했습니다.");
    }
}
unsafe fn alert(title: &str, message: &str) {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str(title));
    a.setInformativeText(&NSString::from_str(message));
    a.addButtonWithTitle(&NSString::from_str("확인"));
    a.runModal();
}
unsafe fn confirm(title: &str, message: &str) -> bool {
    confirm_action(title, message, "휴지통으로 이동")
}
unsafe fn confirm_action(title: &str, message: &str, accept: &str) -> bool {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str(title));
    a.setInformativeText(&NSString::from_str(message));
    a.addButtonWithTitle(&NSString::from_str(accept));
    a.addButtonWithTitle(&NSString::from_str("취소"));
    a.runModal() == 1000
}
unsafe fn input(title: &str, message: &str, initial: &str, accept: &str) -> Option<String> {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str(title));
    a.setInformativeText(&NSString::from_str(message));
    let f = NSTextField::initWithFrame(NSTextField::alloc(mtm()), rect(0., 0., 360., 28.));
    f.setFont(Some(&font::regular(13.)));
    f.setStringValue(&NSString::from_str(initial));
    a.setAccessoryView(Some(&f));
    a.addButtonWithTitle(&NSString::from_str(accept));
    a.addButtonWithTitle(&NSString::from_str("취소"));
    if a.runModal() == 1000 {
        Some(f.stringValue().to_string())
    } else {
        None
    }
}
pub(crate) unsafe fn workspace_context_will_open(tag: isize) {
    STATE.with(|state| if let Ok(mut state) = state.try_borrow_mut() {
        if let Some(s) = state.as_mut() {
            let clicked = s.actions.get(tag as usize).and_then(|a| a.as_ref()).and_then(|a| match a { Action::Workspace(id) => Some(id.clone()), _ => None });
            if s.workspace_rename.as_ref().is_some_and(|(id, _, _)| clicked.as_ref().is_some_and(|other| other != id)) {
                WORKSPACE_COMMAND.with(|c| c.set(0));
                s.finish_workspace_rename(true, false);
            }
        }
    });
}

impl State {
    fn begin_async(&mut self, request: Value) {
        let (tx, receiver) = mpsc::channel();
        let pid = Arc::new(AtomicI32::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let p = pid.clone();
        let c = cancelled.clone();
        std::thread::spawn(move || {
            let result = rpc_inner(request, Some((p.clone(), c.clone())));
            if c.load(Ordering::SeqCst) {
                if let Ok(j) = &result {
                    if let Some(id) = j["id"].as_str() {
                        let _ = rpc(json!({"op":"analysis.cancel","id":id}));
                    }
                }
            }
            let _ = tx.send(result);
            p.store(0, Ordering::SeqCst);
        });
        self.pending = Some(Pending {
            receiver,
            pid,
            cancelled,
        });
    }
    fn register(&mut self, action: Action) -> isize {
        if let Some(i) = self.actions.iter().position(Option::is_none) {
            self.actions[i] = Some(action);
            i as isize
        } else {
            let i = self.actions.len();
            self.actions.push(Some(action));
            i as isize
        }
    }
    unsafe fn button(&mut self, title: &str, frame: NSRect, action: Action) -> Retained<NSButton> {
        let hover_only = matches!(action, Action::Save | Action::Delete);
        let b = if hover_only {
            let button = theme::hover_glass_button(frame);
            button.setTitle(&NSString::from_str(title));
            button.setTarget(Some(&self.target));
            button.setAction(Some(sel!(action:)));
            button
        } else {
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str(title), Some(&self.target), Some(sel!(action:)), mtm(),
            )
        };
        b.setFrame(frame);
        let primary = matches!(action, Action::Import | Action::NewWorkspace);
        b.setTag(self.register(action));
        theme::button(&b, primary);
        if hover_only { b.setBordered(false); }
        b
    }
    /// 판 없는 글자 버튼 하나. 오른쪽 끝이 `right`이고 폭을 돌려준다.
    unsafe fn link_button(&mut self, parent: &NSView, title: &str, right: f64, y: f64, action: Action, ink: &NSColor, _tip: Option<&str>) -> f64 {
        let b = NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(&self.target), Some(sel!(action:)), mtm());
        b.setBordered(false);
        let text = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &NSString::from_str(title));
        let range = NSRange::new(0, text.length());
        text.addAttribute_value_range(NSForegroundColorAttributeName, ink, range);
        text.addAttribute_value_range(NSFontAttributeName, &font::weighted(11., 0.3), range);
        b.setAttributedTitle(&text);
        let w = b.attributedTitle().size().width.ceil() + 8.;
        b.setFrame(rect(right - w, y, w, 20.));
        b.setTag(self.register(action));

        parent.addSubview(&b);
        w
    }
    /// 글자 버튼들을 오른쪽 아래로 모아 세운다. 목록의 왼쪽부터 차례로 놓인다.
    unsafe fn link_row(&mut self, parent: &NSView, items: Vec<(&str, Action, Retained<NSColor>, Option<&str>)>, width: f64, y: f64) {
        let mut right = width;
        for (title, action, ink, tip) in items.into_iter().rev() {
            right -= self.link_button(parent, title, right, y, action, &ink, tip) + 4.;
        }
    }
    /// 입체 유리 판 + 투명 버튼을 parent에 함께 추가한다. 판은 버튼 바로 아래 형제 뷰다.
    unsafe fn glass(
        &mut self,
        parent: &NSView,
        title: &str,
        frame: NSRect,
        action: Action,
        kind: theme::Glass,
        icon: Option<&str>,
    ) -> Retained<NSButton> {
        let b = NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(&self.target),
            Some(sel!(action:)),
            mtm(),
        );
        // 버튼은 유리 셸의 내용 뷰 안에 둔다. 판 없는 버튼은 안쪽 여백이 없으므로 좌우를 띄운다.
        let (shell, inner) = theme::glass_shell(frame, kind);
        let pad = if frame.size.width > 60. { 12. } else { 0. };
        b.setFrame(rect(
            pad,
            0.,
            frame.size.width - pad * 2.,
            frame.size.height,
        ));
        b.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        b.setTag(self.register(action));
        theme::glass_button(&b, kind, icon, 13.);
        inner.addSubview(&b);
        parent.addSubview(&shell);
        b
    }
    /// 판 없는 사이드바 행(아이콘+글자). 선택된 행은 반투명 인디고 바탕·밝은 테두리·왼쪽 강조 막대.
    unsafe fn row(
        &mut self,
        parent: &NSView,
        title: &str,
        frame: NSRect,
        action: Action,
        icon: &str,
        selected: bool,
    ) -> Retained<NSButton> {
        let draft_row = matches!(&action, Action::Open(document) if document["kind"] == "draft");
        if selected && draft_row {
            parent.addSubview(&theme::document_selection(frame));
        } else if selected {
            parent.addSubview(&theme::selection_bg(frame));
            parent.addSubview(&theme::accent_bar(rect(
                3.,
                frame.origin.y + 10.,
                3.,
                frame.size.height - 20.,
            )));
        }
        let b = if draft_row || matches!(action, Action::Workspace(_)) {
            let row: Retained<document_tree::WorkspaceRow> = msg_send![document_tree::WorkspaceRow::alloc(mtm()),initWithFrame:frame];
            row.setTarget(Some(&self.target));
            row.setAction(Some(sel!(action:)));
            row.into_super()
        } else {
            NSButton::buttonWithTitle_target_action(&NSString::from_str(title),Some(&self.target),Some(sel!(action:)),mtm())
        };
        b.setFrame(rect(
            frame.origin.x + 12.,
            frame.origin.y,
            frame.size.width - 20.,
            frame.size.height,
        ));
        b.setTag(self.register(action));
        b.setBordered(false);
        // 아이콘과 글자 사이 숨 쉴 간격. 화면 읽기용 이름은 원래 제목으로 둔다.
        b.setTitle(&NSString::from_str(&format!("  {title}")));
        let _: () = msg_send![&b, setAccessibilityLabel: &*NSString::from_str(title)];
        b.setAlignment(NSTextAlignment::Left);
        b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let ink = if selected {
            theme::indigo()
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.16, 0.17, 0.27, 1.)
        };
        theme::row_title(&b, &ink, if selected { 0.3 } else { 0.0 });
        let tint = if selected {
            theme::indigo()
        } else {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.36, 0.38, 0.52, 1.)
        };
        b.setContentTintColor(Some(&tint));
        theme::set_icon(&b, Some(icon), 14.);
        b.setImageHugsTitle(true);
        if selected {
            let _: () = msg_send![&b, setAccessibilityValue: &*NSString::from_str("선택됨")];
        }
        parent.addSubview(&b);
        b
    }
    /// 아이콘 + 굵은 제목으로 된 섹션 머리말.
    unsafe fn section(
        &self,
        parent: &NSView,
        icon: &str,
        title: &str,
        x: f64,
        y: f64,
        w: f64,
        size: f64,
    ) -> Retained<NSTextField> {
        let side = size + 8.;
        // 작은 글자는 프레임 위쪽에 붙어 그려지므로 아이콘도 그만큼 올려 시각 중심을 맞춘다.
        let lift = (14. - size).max(0.) * 1.5;
        if let Some(v) = theme::icon_view(icon, rect(x, y - 2. + lift, side, side)) {
            parent.addSubview(&v);
        }
        let label = text_label(
            title,
            rect(
                x + side + 6.,
                y + (side - size - 6.) / 2.,
                w - side - 6.,
                size + 6.,
            ),
            size,
            true,
        );
        parent.addSubview(&label);
        label
    }
    unsafe fn layout_status(&self, width: f64) {
        let x = if self.ai_spinner.isHidden() { 26. } else { 46. };
        let height = self.status.frame().size.height;
        let y = if self.diagram.is_some() {
            let line_bottom = height - self.status.firstBaselineOffsetFromTop()
                + self.status.font().map_or(0., |font| font.descender());
            (TITLE_DIVIDER_INSET - line_bottom).round()
        } else { 18. };
        self.status.setFrame(rect(x, y, (width - x - 194.).max(0.), height));
        self.ai_spinner.setFrameOrigin(NSPoint::new(26., y + 4.));
    }
    unsafe fn update_ai_spinner(&self) {
        let active = self.workspace.is_some() && (self.chat.busy() || self.job.is_some()
            || self.cleanup_review.is_some()
            || (self.suggestions_for == self.workspace && self.suggestions["running"].as_i64().unwrap_or(0) > 0));
        if active == self.ai_spinner.isHidden() {
            if active {
                self.ai_spinner.setHidden(false);
                self.ai_spinner.startAnimation(None);
            } else {
                self.ai_spinner.stopAnimation(None);
                self.ai_spinner.setHidden(true);
            }
            self.layout_status(self.center.bounds().size.width);
        }
    }
    unsafe fn set_status(&self, text: &str) {
        self.status.setStringValue(&NSString::from_str(text));
    }
    /// Show editing instructions when the sentence field receives focus.
    unsafe fn concept_desc_began(&mut self) {
        if let Some(hint) = &self.desc_hint {
            hint.setStringValue(&NSString::from_str("Enter 저장 · Esc 취소"));
        }

    }
    unsafe fn save_description(&mut self, id: &str, text: &str) {
        let Some(ws) = self.workspace.clone() else { return; };
        match rpc(json!({"op":"ontology.concept.describe","workspace_id":ws,"concept_id":id,"definition":text})) {
            Ok(_) => {
                self.reload_graph();
                self.refresh_right();
                if let Some(hint) = &self.desc_hint {
                    hint.setStringValue(&NSString::from_str(if text.trim().is_empty() { "원래 문장으로 되돌렸습니다." } else { "저장했습니다." }));
                }
            }
            Err(e) => self.fail(&e),
        }
    }
    /// Enter, Esc, or leaving the sentence field.
    unsafe fn poll_description(&mut self) {
        let command = DESC_COMMAND.with(|c| c.replace(0));
        if command == 0 { return; }
        let Some(field) = self.desc_field.clone() else { return; };
        let id = self.desc_concept.clone();
        if command == 1 {
            let typed = field.currentEditor().map(|e| e.string().to_string()).unwrap_or_else(|| field.stringValue().to_string());
            let typed = typed.trim().to_owned();
            if typed.chars().count() > 200 {
                self.set_status("설명은 200자 이내로 적어 주세요");
                return;
            }
            // Writing the original sentence again is the same as taking the edit back.
            let text = if typed == self.desc_plain { String::new() } else { typed };
            if text.is_empty() && self.desc_plain.is_empty() { return; }
            let unchanged = text.is_empty();
            if unchanged {
                field.abortEditing();
                self.refresh_right();
                if let Some(hint) = &self.desc_hint { hint.setStringValue(&NSString::from_str("바뀐 내용이 없어요.")); }
            } else {
                self.save_description(&id, &text);
            }
        } else {
            field.abortEditing();
            self.refresh_right();
            if let Some(hint) = &self.desc_hint {
                hint.setStringValue(&NSString::from_str(if command == -1 { "취소했습니다." } else { "저장하지 않고 닫았습니다." }));
            }
        }
        DESC_COMMAND.with(|c| c.set(0));
    }
    unsafe fn fail(&self, e: &str) {
        self.set_status("저장 상태를 확인해 주세요");
        alert("작업을 완료하지 못했습니다", e);
    }
    unsafe fn refresh(&mut self) {
        self.load_run();
        self.refresh_left();
        self.refresh_right();
        self.update_header();
    }
    /// 열린 과목의 마지막 과목 정리를 읽는다. 창을 다시 연 뒤에도 돌고 있는 실행은 다시 따라간다.
    fn load_run(&mut self) {
        self.run = self
            .workspace
            .as_ref()
            .and_then(|ws| rpc(json!({"op":"organize.status","workspace_id":ws})).ok())
            .map(|v| v["run"].clone())
            .filter(Value::is_object);
        if self.job.is_none() {
            if let Some(job) = self.run.as_ref().filter(|r| r["state"] == "running").and_then(|r| r["job_id"].as_str()) {
                self.job = Some(job.to_owned());
            }
        }
    }
    fn workspace_name(&self) -> String {
        let id = self.workspace.clone().unwrap_or_default();
        rpc(json!({"op":"workspace.list"}))
            .ok()
            .and_then(|v| v["workspaces"].as_array().and_then(|a| a.iter().find(|w| w["id"] == id.as_str()).map(|w| strv(w, "name"))))
            .unwrap_or_default()
    }
    /// 과목 정리 확인 창을 연다. 대상 해시가 지금 화면의 내용과 같도록 먼저 저장한다.
    unsafe fn open_organize(&mut self, reread: Option<(String, String)>) {
        let Some(ws) = self.workspace.clone() else {
            return;
        };
        if !self.flush_body() || !self.finish_new_title(true) {
            return;
        }
        pdf_session::request_flush_all();
        organize_panel::open(&ws, &self.workspace_name(), reread);
    }
    unsafe fn reload_graph(&mut self) {
        let Some(ws) = self.workspace.clone() else {
            return;
        };
        if let Ok(g) = rpc(json!({"op":"graph.get","workspace_id":ws})) {
            self.graph_data = g.clone();
            if self.map.is_some() {
                // A reload after hiding a link or undoing a merge stays on the concept being read.
                let open = self.selection["concept"]["id"].as_str().map(str::to_owned);
                graph::update(g);
                if !self.unit_focus.is_empty() {
                    graph::focus_unit(&self.unit_focus);
                }
                if let Some(id) = open {
                    graph::select(&id);
                }
                self.selection = graph::selection().unwrap_or(Value::Null);
                self.refresh_relation_options();
            }
        }
    }
    /// 정리 제안을 다시 읽는다. 읽을 때마다 기다리던 정리 묶음이 차례를 얻는다.
    unsafe fn poll_suggestions(&mut self) {
        self.last_suggestion_poll = Instant::now();
        let status = self
            .workspace
            .as_ref()
            .and_then(|ws| rpc(json!({"op":"distill.status","workspace_id":ws})).ok())
            .unwrap_or(Value::Null);
        let was_working = self.suggestions["working"].as_i64().unwrap_or(0);
        let changed = status != self.suggestions || self.suggestions_for != self.workspace;
        self.suggestions = status;
        self.suggestions_for = self.workspace.clone();
        if changed {
            if was_working > self.suggestions["working"].as_i64().unwrap_or(0) {
                self.chat.sync_notes();
            }
            if self.map.is_none() {
                self.refresh_right();
            }
        }
        self.update_tab_dot();
    }
    /// 정리 제안 창을 닫을 때 그 안의 제안과 좁히기 안내를 본 것으로 친다.
    /// 열자마자 표시하면 열린 창에서 안내가 바로 사라진다.
    unsafe fn mark_suggestions_seen(&mut self) {
        let Some(ws) = self.suggestions_for.clone() else { return };
        if self.suggestions["unseen"] != true && self.suggestions["notice"].is_null() {
            return;
        }
        if rpc(json!({"op":"distill.seen","workspace_id":ws})).is_ok() {
            self.suggestions["unseen"] = json!(false);
            for s in self.suggestions["suggestions"].as_array_mut().into_iter().flatten() {
                s["seen"] = json!(true);
            }
            if self.suggestions["notice"].as_str().is_some_and(|n| n.starts_with("자동으로")) {
                self.suggestions["notice"] = Value::Null;
            }
        }
        self.update_tab_dot();
    }
    unsafe fn update_tab_dot(&self) {
        let dot = self.suggestions["unseen"] == true && self.chat_mode;
        self.tab_dot.setHidden(!dot);
        let _: () = msg_send![&self.right_tabs[0], setAccessibilityLabel: &*NSString::from_str(if dot { "자료 · 새 정리 제안 있음" } else { "자료" })];
    }
    /// 파일 검색 옆 ✦ 버튼. 제안 개수는 아이콘 오른쪽 위의 작은 원으로 보인다.
    unsafe fn render_suggest_button(&mut self, h: f64, w: f64) {
        let right = self.right.clone();
        let count = self.suggestions["suggestions"].as_array().map(Vec::len).unwrap_or(0);
        let working = self.suggestions["working"].as_i64().unwrap_or(0) > 0;
        let frame = rect(w - 46., h - 88., 30., 28.);
        let kind = if self.suggest_open { theme::Glass::Selected } else { theme::Glass::Clear };
        let b = self.glass(&right, "", frame, Action::SuggestToggle, kind, Some("sparkle"));
        if count == 0 && !working && suggestion_note(&self.suggestions).is_none() {
            b.setContentTintColor(Some(&NSColor::tertiaryLabelColor()));
        }
        let label = match (count, working) {
            (0, true) => "정리 제안 · 초안을 만드는 중".to_owned(),
            (0, false) => "정리 제안 없음".to_owned(),
            (n, _) => format!("정리 제안 {n}개"),
        };
        let _: () = msg_send![&b, setAccessibilityLabel: &*NSString::from_str(&label)];
        if count > 0 {
            let text = if count > 9 { "9+".to_owned() } else { count.to_string() };
            let bw = if count > 9 { 20. } else { 15. };
            let badge = text_label(&text, rect(frame.origin.x + frame.size.width - bw + 5., frame.origin.y + frame.size.height - 10., bw, 15.), 9., true);
            badge.setAlignment(NSTextAlignment::Center);
            badge.setTextColor(Some(&suggest_color(0.42, 0.43, 0.50, 1.)));
            let title: Retained<NSMutableAttributedString> = msg_send![&badge.attributedStringValue(), mutableCopy];
            title.addAttribute_value_range(NSBaselineOffsetAttributeName, &NSNumber::numberWithDouble(-2.), NSRange::new(0, title.length()));
            badge.setAttributedStringValue(&title);
            rounded(&badge, &NSColor::whiteColor(), 7.5);
            let _: () = msg_send![&badge, setAccessibilityElement: false];
            right.addSubview(&badge);
        }
        self.suggest_button = Some(b);
    }
    /// ✦ 버튼 아래에 뜨는 정리 제안 창. 등록하기 전에는 자료에 들어가지 않는다.
    unsafe fn render_suggest_pop(&mut self) {
        let was_open = !self.suggest_layer.is_empty();
        let mut previous = NSPoint::new(0., 0.);
        for v in std::mem::take(&mut self.suggest_layer) {
            if let Some(scroll) = find_scroll(&v) {
                previous = scroll.contentView().bounds().origin;
            }
            release_actions(&v, &mut self.actions);
            v.removeFromSuperview();
        }
        let showing = self.suggest_open && !self.chat_mode && self.map.is_none() && self.workspace.is_some();
        let (Some(anchor), Some(content)) = (self.suggest_button.clone().filter(|_| showing), self._window.contentView()) else {
            self.suggest_open = false;
            self.suggest_confirm = None;
            if was_open {
                self.mark_suggestions_seen();
            }
            return;
        };
        let at = anchor.convertRect_toView(anchor.bounds(), Some(&content));
        // 창 바깥을 누르면 닫는다.
        let catcher = NSButton::buttonWithTitle_target_action(&NSString::from_str(""), Some(&self.target), Some(sel!(action:)), mtm());
        catcher.setFrame(content.bounds());
        catcher.setBordered(false);
        catcher.setTransparent(true);
        catcher.setTag(self.register(Action::SuggestClose));
        catcher.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        let _: () = msg_send![&catcher, setAccessibilityElement: false];
        content.addSubview(&catcher);
        self.suggest_layer.push(Retained::into_super(Retained::into_super(catcher)));

        let status = self.suggestions.clone();
        let list = status["suggestions"].as_array().cloned().unwrap_or_default();
        let width = SUGGEST_POP_WIDTH - 26.;
        let body = chat_messages::flipped();
        let mut y = 0.;
        let small = |text: &str, x: f64, y: &mut f64, w: f64| {
            let label = flow_label(&body, text, x, *y, w, 10.5, false, 4);
            label.setTextColor(Some(&NSColor::secondaryLabelColor()));
            *y += label.frame().size.height + 6.;
        };
        if status["working"].as_i64().unwrap_or(0) > 0 {
            small("AI가 노트 초안을 만드는 중이에요…", 0., &mut y, width);
        }
        if let Some(note) = suggestion_note(&status) {
            small(&note, 0., &mut y, width);
        }
        if status["intro_seen"] != true {
            let text = flow_label(&body, "AI와 나눈 대화에서 다시 볼 만한 답을 노트 초안으로 모아 두는 곳이에요. 등록하면 자료에 노트로 들어가고, 그 전에는 지도에 반영되지 않아요.", 0., y, width, 10.5, false, 5);
            text.setTextColor(Some(&suggest_color(0.36, 0.38, 0.47, 1.)));
            let bottom = y + text.frame().size.height + 4.;
            self.link_button(&body, "알겠어요", width, bottom, Action::SuggestIntroSeen, &theme::indigo(), None);
            y = bottom + 28.;
        }
        if list.is_empty() {
            small("새 정리 제안이 없어요. AI와 대화하다 다시 볼 만한 답이 생기면 여기 모여요.", 0., &mut y, width);
        }
        let danger = suggest_color(0.77, 0.28, 0.23, 1.);
        let mut confirm_at = None;
        for (i, s) in list.iter().enumerate() {
            if i > 0 {
                body.addSubview(&theme::soft_divider(rect(0., y, width, 1.), true));
                y += 10.;
            }
            let id = strv(s, "id");
            let draft = s["state"] == "draft";
            let name = text_label(&strv(s, "title"), rect(0., y, width, 17.), 12., true);
            name.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            body.addSubview(&name);
            y += 21.;
            let chip_text = suggestion_chip(s);
            let (ink, fill) = match chip_text {
                "초안" => (suggest_color(0.30, 0.27, 0.78, 1.), suggest_color(0.92, 0.92, 0.98, 1.)),
                "AI 초안" => (suggest_color(0.18, 0.43, 0.70, 1.), suggest_color(0.91, 0.95, 1., 1.)),
                _ => (suggest_color(0.43, 0.45, 0.53, 1.), suggest_color(0.95, 0.95, 0.97, 1.)),
            };
            let chip = text_label(chip_text, rect(0., y + 1., 10., 15.), 9.5, true);
            chip.setTextColor(Some(&ink));
            chip.setAlignment(NSTextAlignment::Center);
            let chip_w = chip.intrinsicContentSize().width.ceil() + 10.;
            chip.setFrame(rect(0., y + 1., chip_w, 15.));
            rounded(&chip, &fill, 5.);
            let meta = flow_label(&body, &suggestion_text(s), 0., y, width, 10.5, false, 3);
            meta.setTextColor(Some(&NSColor::secondaryLabelColor()));
            first_line_indent(&meta, chip_w + 5.);
            let meta_h = meta.cell().map(|c| c.cellSizeForBounds(rect(0., 0., width, 10000.)).height.ceil()).unwrap_or(16.);
            meta.setFrame(rect(0., y, width, meta_h));
            body.addSubview(&chip);
            y += meta_h.max(17.) + 7.;
            let buttons: Vec<(&str, &'static str)> = if draft {
                vec![("등록", "register"), ("수정", "edit")]
            } else {
                vec![("초안 만들기", "draft")]
            };
            let mut items: Vec<(&str, Action, Retained<NSColor>, Option<&str>)> = buttons.into_iter().map(|(title, kind)| {
                let tip = match kind {
                    "register" => "‘학습도우미 정리’ 폴더에 노트로 저장해요. 다음 과목 정리 때 지도에 들어가요.",
                    "edit" => "등록하기 전에 제목과 내용을 고쳐요.",
                    _ => "이 질문과 답을 노트 초안으로 정리해요. 대화 내용이 AI에 한 번 더 전송돼요.",
                };
                let ink = if kind == "register" || kind == "draft" { theme::indigo() } else { NSColor::secondaryLabelColor() };
                (title, Action::Suggest(kind, id.clone()), ink, Some(tip))
            }).collect();
            items.push(("삭제", Action::Suggest("ask_dismiss", id.clone()), danger.clone(), Some(if draft { "이 초안과 보관해 둔 대화 사본을 지워요." } else { "이 제안을 지워요. 노트로 만들지 않아요." })));
            self.link_row(&body, items, width, y);
            y += 22.;
            if self.suggest_confirm.as_deref() == Some(id.as_str()) {
                let text = flow_label(&body, &format!("이 {}을 삭제할까요? 보관해 둔 대화 사본도 함께 지워요. 되돌릴 수 없어요.", if draft { "초안" } else { "제안" }), 0., y + 4., width, 10.5, false, 3);
                text.setTextColor(Some(&suggest_color(0.48, 0.23, 0.19, 1.)));
                let row = y + 4. + text.frame().size.height + 2.;
                self.link_row(&body, vec![
                    ("취소", Action::Suggest("cancel", id.clone()), NSColor::secondaryLabelColor(), None),
                    ("삭제", Action::Suggest("dismiss", id.clone()), danger.clone(), None),
                ], width, row);
                confirm_at = Some(rect(0., y, width, row + 22. - y));
                y = row + 22.;
            }
            y += 12.;
        }
        let body_h = (y - 6.).max(30.);
        body.setFrame(rect(0., 0., width, body_h));
        let room = (at.origin.y - 20.).max(160.);
        let header_h = 28.;
        let panel_h = (body_h + 32. + header_h).min(SUGGEST_POP_MAX).min(room);
        let panel_x = (at.origin.x + at.size.width + 6. - SUGGEST_POP_WIDTH).max(8.);
        // 가독성을 위해 흰 바탕 위에 은은한 라벤더 틴트를 얹는다. 따로 구분되는 상자나 테두리는 없다.
        let (outer, glass) = theme::panel(rect(panel_x, at.origin.y - 9. - panel_h, SUGGEST_POP_WIDTH, panel_h));
        glass.setWantsLayer(true);
        let gl: Retained<AnyObject> = msg_send![&glass, layer];
        let _: () = msg_send![&gl, setBackgroundColor: &*NSColor::whiteColor().CGColor()];
        let _: () = msg_send![&gl, setCornerRadius: 22f64];
        let _: () = msg_send![&gl, setCornerCurve: &*NSString::from_str("continuous")];
        let _: () = msg_send![&gl, setMasksToBounds: true];
        let wash = view(rect(0., 0., SUGGEST_POP_WIDTH, panel_h));
        rounded(&wash, &suggest_color(0.78, 0.73, 0.97, 0.20), 0.);
        wash.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        glass.addSubview(&wash);
        outer.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin | NSAutoresizingMaskOptions::ViewMinYMargin);
        let header_y = panel_h - 13. - header_h;
        if let Some(image) = theme::icon("sparkle", 13.) {
            let icon = NSImageView::imageViewWithImage(&image, mtm());
            icon.setFrame(rect(13., header_y + 6., 16., 16.));
            icon.setContentTintColor(Some(&theme::indigo()));
            glass.addSubview(&icon);
        }
        let button_w = 78.;
        let button_h = 24.;
        let button_x = SUGGEST_POP_WIDTH - 13. - button_w;
        let label_x = 34.;
        let head = text_label("", NSRect::ZERO, 12.5, true);
        let title = NSMutableAttributedString::initWithString(
            NSMutableAttributedString::alloc(),
            &NSString::from_str(&format!("정리 제안  {}개", list.len())),
        );
        let heading = NSRange::new(0, NSString::from_str("정리 제안").length());
        let count = NSRange::new(heading.length, title.length() - heading.length);
        title.addAttribute_value_range(NSFontAttributeName, &font::bold(12.5), heading);
        title.addAttribute_value_range(NSFontAttributeName, &font::regular(11.), count);
        title.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::secondaryLabelColor(), count);
        head.setAttributedStringValue(&title);
        let label_h = head.intrinsicContentSize().height.ceil();
        head.setFrame(rect(label_x, header_y + (header_h - label_h) / 2., button_x - label_x - 8., label_h));
        glass.addSubview(&head);

        let has_materials = self.workspace.as_ref()
            .and_then(|ws| rpc(json!({"op":"tree","workspace_id":ws})).ok())
            .is_some_and(|tree| organize_entries(&tree).next().is_some());
        let resumable = matches!(self.visible_organize_run().and_then(|run| run["state"].as_str()), Some("paused" | "limit" | "failed"));
        let organize = self.glass(
            &glass,
            "전체 정리",
            rect(button_x, header_y + (header_h - button_h) / 2., button_w, button_h),
            Action::Organize,
            theme::Glass::Selected,
            None,
        );
        // 이미 캡슐에 여백이 있으므로 버튼 셀에 캡슐 전체 폭을 준다.
        organize.setFrame(rect(0., 0., button_w, button_h));
        if self.job.is_some() || self.pending.is_some() || resumable || !has_materials {
            organize.setEnabled(false);
            organize.setAlphaValue(0.45);
        }

        // 넘칠 때만 겹쳐 뜨는 스크롤 막대를 오른쪽 여백 위에 둔다.
        let scroll_h = header_y - 19.;
        let overflow = body_h > scroll_h;
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(13., 13., width + if overflow { 11. } else { 0. }, scroll_h));
        scroll.setHasVerticalScroller(overflow);
        scroll.setHasHorizontalScroller(false);
        scroll.setScrollerStyle(NSScrollerStyle::Overlay);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&body));
        glass.addSubview(&scroll);
        // 다시 그려도 보던 자리를 지키고, 삭제 확인이 열리면 그 상자가 보이게 한다.
        body.scrollPoint(previous);
        if let Some(r) = confirm_at {
            body.scrollRectToVisible(r);
        }
        let _: () = msg_send![&outer, setAccessibilityRole: &*NSString::from_str("AXGroup")];
        let _: () = msg_send![&outer, setAccessibilityLabel: &*NSString::from_str("정리 제안")];
        content.addSubview(&outer);
        self.suggest_layer.push(outer);
    }
    fn visible_organize_run(&self) -> Option<&Value> {
        self.run.as_ref().filter(|r| r["single"] != true && r["seen"] != true)
    }
    /// 자료 보관함의 과목 정리 줄: 진행과 멈춤, 이어서 정리, 결과 요약.
    unsafe fn render_organize_strip(&mut self, tree: &Value, heading: &NSTextField, top: f64, w: f64) -> f64 {
        let right = self.right.clone();
        let run = self.visible_organize_run().unwrap_or(&Value::Null);
        let entries: Vec<_> = organize_entries(tree).collect();
        let fresh = entries.iter().filter(|e| e["analysis_state"] == "unanalysed").count();
        let changed = entries.iter().filter(|e| e["analysis_state"] == "stale").count();
        if fresh > 0 {
            let mut title_frame = heading.frame();
            title_frame.size.width = heading.intrinsicContentSize().width.ceil() + 2.;
            heading.setFrame(title_frame);
            let x = title_frame.origin.x + title_frame.size.width + 6.;
            let label = text_label(
                &format!("새 자료 {fresh}개"),
                rect(x, title_frame.origin.y, w - x - 16., title_frame.size.height),
                11., false,
            );
            let mut frame = label.frame();
            frame.size.height = label.intrinsicContentSize().height.ceil();
            frame.origin.y += (title_frame.size.height - frame.size.height) / 2.;
            label.setFrame(frame);
            label.setTextColor(Some(&NSColor::secondaryLabelColor()));
            right.addSubview(&label);
        }
        let (text, buttons): (String, Vec<(&str, Action, Option<&str>, bool)>) = match run["state"].as_str() {
            Some("running") => (strv(&run["progress"], "label"), vec![("멈춤", Action::OrganizePause, Some("stop"), false)]),
            Some(state @ ("paused" | "limit" | "failed")) => (
                match state {
                    "limit" => "구독 사용 한도에 닿아 멈췄어요. 읽은 부분은 보관해 두었어요.".to_owned(),
                    "failed" => format!("정리를 마치지 못했어요. 이전 지도는 그대로예요. {}", strv(&run, "error")),
                    _ => format!("멈췄어요 · {}", strv(&run["progress"], "label")),
                },
                vec![("이어서 정리", Action::OrganizeResume, Some("sparkle"), true), ("닫기", Action::OrganizeDismiss, None, false)],
            ),
            Some("succeeded") => {
                let result = &run["result"];
                let skipped = result["failed"].as_array().map(Vec::len).unwrap_or(0);
                let low = result["low_quality"].as_array().map(Vec::len).unwrap_or(0);
                let mut text = strv(result, "text");
                if skipped > 0 {
                    text.push_str(&format!(" · 건너뛴 자료 {skipped}개"));
                }
                if low > 0 {
                    text.push_str(&format!(" · 정리 품질 낮음 {low}개"));
                }
                // Materials added after the last run can be organised without closing its summary first.
                if fresh + changed > 0 {
                    let summary = if changed > 0 {
                        format!("정리 필요 {changed}개 · 지난 정리: {text}")
                    } else {
                        format!("지난 정리: {text}")
                    };
                    (summary, vec![("어떻게 정리했나요", Action::OrganizeHow, None, false)])
                } else {
                    (text, vec![("어떻게 정리했나요", Action::OrganizeHow, None, false), ("닫기", Action::OrganizeDismiss, None, false)])
                }
            }
            _ => (
                if self.job.is_some() {
                    "진행 중인 작업이 끝나면 정리할 수 있어요".to_owned()
                } else if entries.is_empty() {
                    "자료를 넣으면 과목 전체를 한 번에 정리할 수 있어요".to_owned()
                } else if fresh + changed == 0 {
                    "모든 자료가 지도에 반영되어 있어요".to_owned()
                } else if changed > 0 {
                    format!("정리 필요 {changed}개")
                } else {
                    String::new()
                },
                Vec::new(),
            ),
        };
        let height = if buttons.is_empty() {
            if text.is_empty() { return 0.; }
            40.
        } else {
            ORGANIZE_STRIP
        };
        let y = top - height;
        let count = buttons.len().max(1) as f64;
        let width = (w - 32. - 6. * (count - 1.)) / count;
        let occupied = self.job.is_some() || self.pending.is_some();
        for (i, (title, action, icon, primary)) in buttons.into_iter().enumerate() {
            let blocked = match action {
                Action::OrganizeResume => occupied,
                _ => false,
            };
            let b = self.glass(
                &right,
                title,
                rect(16. + i as f64 * (width + 6.), y + 40., width, 30.),
                action,
                if primary { theme::Glass::Primary } else { theme::Glass::Clear },
                icon,
            );
            if blocked {
                b.setEnabled(false);
                b.setAlphaValue(0.45);
            }
        }
        let label = text_label(&text, rect(18., y + 2., w - 36., 32.), 11., false);
        label.setMaximumNumberOfLines(2);
        label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        label.setTextColor(Some(&NSColor::secondaryLabelColor()));
        right.addSubview(&label);
        self.organize_label = Some(label);
        height
    }
    unsafe fn finish_workspace_rename(&mut self, commit: bool, refresh: bool) {
        let Some((id, old, field)) = self.workspace_rename.take() else { return; };
        let name = field.currentEditor().map(|e| e.string().to_string()).unwrap_or_else(|| field.stringValue().to_string());
        field.setDelegate(None);
        field.abortEditing();
        field.removeFromSuperview();
        let mut shown = old.clone();
        if commit && name != old {
            match rpc(json!({"op":"workspace.rename","workspace_id":id,"name":name})) {
                Ok(_) => shown = name,
                Err(e) => self.fail(&e),
            }
        }
        if !refresh {
            for action in self.actions.iter_mut().flatten() {
                match action {
                    Action::RenameWorkspace(key, title) | Action::DeleteWorkspace(key, title) if key == &id => *title = shown.clone(),
                    _ => {}
                }
            }
        }
        if refresh { self.refresh_left(); }
        else if let Some((_, _, button)) = self.workspace_rows.iter().find(|(key, _, _)| key == &id) {
            button.setTitle(&NSString::from_str(&format!("  {shown}")));
            let selected = self.workspace.as_deref() == Some(id.as_str());
            let ink = if selected { theme::soft_selection_ink() } else { NSColor::colorWithSRGBRed_green_blue_alpha(0.16, 0.17, 0.27, 1.) };
            theme::row_title(button, &ink, if selected { 0.3 } else { 0. });
        }
    }
    unsafe fn refresh_left(&mut self) {
        self.refresh_left_with_motion(false);
    }
    unsafe fn refresh_left_with_motion(&mut self, animate_selection: bool) {
        if self.workspace_rename.is_some() { return; }
        self.workspace_rows.clear();
        clear(&self.left, &mut self.actions);
        let h = self.left.frame().size.height;
        let left = self.left.clone();
        let lw = left.frame().size.width;
        left.addSubview(&text_label("워크스페이스", rect(24., h - 46., lw - 70., 24.), 13., true));
        let add = self.row(
            &left,
            "워크스페이스 추가",
            rect(12., h - 96., lw - 24., 42.),
            Action::NewWorkspace,
            "plus",
            false,
        );
        add.setContentTintColor(Some(&theme::indigo()));
        theme::row_title(&add, &theme::indigo(), 0.3);
        if let Ok(r) = rpc(json!({"op":"workspace.list"})) {
            let rows = r["workspaces"].as_array().cloned().unwrap_or_default();
            left.addSubview(&theme::soft_divider(rect(14., h - 105., lw - 28., 1.), true));
            let top = h - 110.;
            let area = (top - 150.).max(80.);
            let ch = (rows.len() as f64 * WORKSPACE_ROW_STEP + 10.).max(area);
            let list = view(rect(0., 0., lw, ch));
            let selected_index = rows
                .iter()
                .position(|w| self.workspace.as_deref() == Some(strv(w, "id").as_str()));
            for (i, w) in rows.iter().enumerate() {
                let id = strv(w, "id");
                let selected = selected_index == Some(i);
                // 46pt 간격에 40pt 행을 두어 워크스페이스 사이 여백 6pt를 유지한다.
                let frame = rect(12., ch - 40. - i as f64 * WORKSPACE_ROW_STEP, lw - 24., 40.);
                if selected {
                    let selection = theme::soft_selection(frame);
                    list.addSubview(&selection);
                    if animate_selection { theme::float_selection(&selection); }
                }
                let row_container = view(frame);
                list.addSubview(&row_container);
                let b = self.row(
                    &row_container,
                    &strv(w, "name"),
                    row_container.bounds(),
                    Action::Workspace(id.clone()),
                    if selected { "book-fill" } else { "book" },
                    false,
                );
                self.workspace_rows.push((id.clone(), row_container.clone(), b.clone()));
                let menu = NSMenu::new(mtm());
                for (label, icon, action) in [
                    ("이름 변경", "pencil", Action::RenameWorkspace(id.clone(), strv(w,"name"))),
                    ("삭제", "trash", Action::DeleteWorkspace(id, strv(w,"name"))),
                ] {
                    let title = NSString::from_str(label);
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm()), &title, Some(sel!(action:)), &NSString::from_str(""));
                    item.setTarget(Some(&self.target));
                    item.setTag(self.register(action));
                    item.setImage(theme::icon(icon, 13.).as_deref());
                    if icon == "trash" {
                        document_tree::style_delete_item(&item, &title);
                    }
                    menu.addItem(&item);
                }
                b.setMenu(Some(&menu));
                if selected {
                    if animate_selection { theme::float_selection(&b); }
                    theme::row_title(&b, &theme::soft_selection_ink(), 0.3);
                    b.setContentTintColor(Some(&theme::soft_selection_icon()));
                    let _: () =
                        msg_send![&b, setAccessibilityValue: &*NSString::from_str("선택됨")];
                }
            }
            let scroll =
                NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., 150., lw, area));
            scroll.setHasVerticalScroller(true);
            crate::scrollbar::style(&scroll);
            scroll.setAutohidesScrollers(true);
            scroll.setDrawsBackground(false);
            scroll.setDocumentView(Some(&list));
            list.scrollPoint(NSPoint::new(0., ch));
            left.addSubview(&scroll);
        }
        left.addSubview(&theme::soft_divider(rect(14., 142., lw - 28., 1.), true));
        self.row(
            &left,
            "AI 설정",
            rect(12., 100., lw - 24., 38.),
            Action::AiSettings,
            "sparkle",
            false,
        );
        let connect = self.row(
            &left,
            "구독 계정 연결",
            rect(12., 62., lw - 24., 38.),
            Action::Connect,
            "key",
            false,
        );
        hand_cursor::install(&connect);
        // 헬퍼텍스트는 행 제목 글자 시작점에 맞춘다.
        let help = text_label("", rect(46., 48., lw - 58., 16.), 11., false);
        help.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        left.addSubview(&help);
        self.connect_button = Some(connect);
        self.connect_help = Some(help);
        self.update_provider_row();
        let footer = text_label("내 Mac에 보관됩니다", rect(24., 22., 175., 18.), 10., false);
        footer.setTextColor(Some(&NSColor::secondaryLabelColor()));
        self.left.addSubview(&footer);
        // Keep header rows attached to the top and the list flexible while
        // resizing, without fetching workspaces or recreating controls.
        for child in self.left.subviews().iter() {
            child.setAutoresizingMask(if child.isKindOfClass(NSScrollView::class()) {
                NSAutoresizingMaskOptions::ViewHeightSizable
            } else if child.frame().origin.y > 142. {
                NSAutoresizingMaskOptions::ViewMinYMargin
            } else {
                NSAutoresizingMaskOptions::empty()
            });
        }
    }
    unsafe fn refresh_right(&mut self) {
        // Keep AppKit's field editor (including IME composition) alive during
        // resize and background updates. Apply the deferred layout after editing.
        if self.map.is_none()
            && self.document_tree.as_ref().is_some_and(|tree| {
                tree.renaming_workspace()
                    .as_deref()
                    .is_some_and(|workspace| self.workspace.as_deref() == Some(workspace))
            })
        {
            self.tree_refresh_pending = true;
            return;
        }
        self.tree_refresh_pending = false;
        self.organize_label = None;
        let tree_scroll = self
            .document_tree
            .as_ref()
            .map(|tree| tree.scroll_position());
        self.document_tree = None;
        // Keep the search field and its IME editor alive during sidebar refresh.
        for child in self.right.subviews().iter() {
            if self.file_search.as_ref().is_some_and(|field| {
                Retained::as_ptr(field).cast::<AnyObject>()
                    == Retained::as_ptr(&child).cast::<AnyObject>()
            }) {
                continue;
            }
            release_actions(&child, &mut self.actions);
            child.removeFromSuperview();
        }
        if let Some(field) = &self.file_search {
            field.setHidden(self.map.is_some());
        }
        let h = self.right.frame().size.height;
        let w = self.right.frame().size.width;
        let right = self.right.clone();

        if self.map.is_some() {
            self.section(
                &right,
                "network",
                "개념과 원본 근거",
                16.,
                h - 46.,
                w - 68.,
                14.,
            );
            self.render_graph_side();
            self.suggest_button = None;
            self.render_suggest_pop();
            return;
        }
        let heading = self.section(&right, "archive", "자료 보관함", 16., h - 46., w - 68., 14.);
        let field = self.file_search.get_or_insert_with(|| {
            let field = NSSearchField::initWithFrame(
                NSSearchField::alloc(mtm()),
                rect(16., h - 90., w - 68., 32.),
            );
            field.setPlaceholderString(Some(&NSString::from_str("파일 검색")));
            field.setFont(Some(&font::regular(13.)));
            field.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
            let _: () = msg_send![&field, setAccessibilityLabel: &*NSString::from_str("파일 검색")];
            right.addSubview(&field);
            field
        });
        field.setFrame(rect(16., h - 90., w - 68., 32.));
        field.setEnabled(self.workspace.is_some());
        self.suggest_button = None;
        if self.workspace.is_some() {
            self.render_suggest_button(h, w);
        }
        self.render_suggest_pop();
        if let Some(ws) = self.workspace.clone() {
            let tree = rpc(json!({"op":"tree","workspace_id":ws})).unwrap_or(json!({}));
            if let Some(document) = self.document.as_mut() {
                if let Some(current) = tree["entries"].as_array().and_then(|entries| entries.iter().find(|entry| entry["id"] == document["id"])) {
                    document["analysis_state"] = current["analysis_state"].clone();
                }
            }
            self.update_analysis_help();
            let area_h = (h * 0.53).max(180.);
            let strip_h = self.render_organize_strip(&tree, &heading, h - 104., w);
            let selected = self.document.as_ref().and_then(|d| d["id"].as_str());
            let (scroll, controller) = document_tree::build(
                &tree,
                selected,
                &self.open_folders,
                rect(10., h - 104. - area_h, w - 20., (area_h - strip_h).max(60.)),
            );
            controller.set_analysis_busy(self.analysis_busy());
            controller.filter(&self.last_file_query, &self.open_folders);
            if let Some(position) = tree_scroll {
                controller.restore_scroll(position);
            }
            self.document_tree = Some(controller);
            self.right.addSubview(&scroll);
            let sep_y = h - 114. - area_h;
            self.right
                .addSubview(&theme::soft_divider(rect(14., sep_y, w - 28., 1.), true));
            self.section(&right, "pencil", "작성 중", 16., sep_y - 32., w - 32., 12.);
            if let Ok(ds) = rpc(json!({"op":"draft.list","workspace_id":ws})) {
                let drafts = ds["drafts"].as_array().cloned().unwrap_or_default();
                let area = (sep_y - 50.).max(30.);
                let ch = (drafts.len() as f64 * 40. + 6.).max(area);
                let list = view(rect(0., 0., w - 24., ch));
                for (i, d) in drafts.iter().enumerate() {
                    let title = d["title"].as_str().unwrap_or("제목 없음");
                    let container = view(rect(3., ch - 37. - i as f64 * 40., w - 26., 32.));
                    list.addSubview(&container);
                    let row = self.row(
                        &container,
                        title,
                        container.bounds(),
                        Action::Open(d.clone()),
                        "doc",
                        self.document
                            .as_ref()
                            .is_some_and(|current| current["id"] == d["id"]),
                    );
                    let menu = NSMenu::new(mtm());
                    let title = NSString::from_str("삭제");
                    let item = NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm()), &title, Some(sel!(action:)), &NSString::from_str(""));
                    item.setTarget(Some(&self.target));
                    item.setTag(self.register(Action::DeleteDocument(d.clone())));
                    document_tree::style_delete_item(&item, &title);
                    menu.addItem(&item);
                    row.setMenu(Some(&menu));
                }
                let scroll = NSScrollView::initWithFrame(
                    NSScrollView::alloc(mtm()),
                    rect(10., 10., w - 20., area),
                );
                scroll.setHasVerticalScroller(true);
                crate::scrollbar::style(&scroll);
                scroll.setAutohidesScrollers(true);
                scroll.setDrawsBackground(false);
                scroll.setDocumentView(Some(&list));
                list.scrollPoint(NSPoint::new(0., ch));
                self.right.addSubview(&scroll);
            }
        }
    }
    unsafe fn resize_right_contents(&self, height: f64) {
        let old_height = self.right.bounds().size.height;
        if height == old_height { return; }
        let old_area = (old_height * 0.53).max(180.);
        let area = (height * 0.53).max(180.);
        let old_separator = old_height - 114. - old_area;
        let separator = height - 114. - area;
        for child in self.right.subviews().iter() {
            let mut frame = child.frame();
            if child.isKindOfClass(NSScrollView::class()) {
                if self.map.is_some() {
                    frame.size.height += height - old_height;
                } else if frame.origin.y <= 10. {
                    frame.size.height = (separator - 50.).max(30.);
                } else {
                    frame.origin.y = height - 104. - area;
                    frame.size.height += area - old_area;
                }
            } else if self.map.is_none() && frame.origin.y <= old_separator {
                frame.origin.y += separator - old_separator;
            } else {
                frame.origin.y += height - old_height;
            }
            child.setFrame(frame);
        }
    }
    unsafe fn update_analysis_help(&self) {
        let visible = self.diagram.is_none() && self.map.is_none() && !self.snapshot && self.workspace.is_some() && self.editing_suggestion().is_none();
        let draft = !self.document.as_ref().is_some_and(|d| d["path"].as_str().is_some_and(|p| !p.is_empty()));
        self.analysis_help.setHidden(!visible);
        let state = self.document.as_ref().map(|d| strv(d, "analysis_state")).unwrap_or_default();
        let unsaved = self.dirty || pdf_session::has_pending();
        let complete = state == "analysed" && !unsaved;
        // A material that was read and then changed needs organising again.
        let outdated = state == "stale" || (state == "analysed" && unsaved);
        self.analysis_help.setStringValue(&NSString::from_str(if draft { "작성중" } else if complete { "분석완료" } else if outdated { "정리 필요" } else { "분석 미완료" }));
        let color = if draft { NSColor::systemYellowColor() } else if complete { NSColor::systemGreenColor() } else if outdated { NSColor::systemOrangeColor() } else { NSColor::secondaryLabelColor() };
        self.analysis_help.setTextColor(Some(&color));
        let size = self.center.bounds().size;
        // Round up the label bounds and leave room for the text cell's trailing inset.
        // An exact fractional intrinsic width clips the final Korean glyph.
        let help_width = if visible { self.analysis_help.intrinsicContentSize().width.ceil() + 4. } else { 0. };
        let title_width = (size.width - 210. - if visible { help_width + 8. } else { 0. }).max(0.);
        self.header.setFrame(rect(28., size.height - 61., title_width, 38.));
        let diagram = self.diagram.is_some();
        let left = if diagram { self.header.frame().origin.x } else { TITLE_DIVIDER_INSET };
        let lift = if diagram { self.subtitle.frame().size.height } else { 0. };
        theme::resize_soft_divider(&self.title_divider, rect(
            left, size.height - 91. + lift, (size.width - left - TITLE_DIVIDER_INSET).max(0.), 1.,
        ));
        self.analysis_help.setFrame(rect((size.width - 182. - help_width).max(28.), size.height - 49., help_width, 18.));
    }
    fn editing_suggestion(&self) -> Option<&Value> {
        self.document.as_ref().filter(|doc| doc["kind"] == "suggestion")
    }
    unsafe fn update_header(&mut self) {
        let needs_workspace = self.workspace.is_none();
        let diagram_available = self.can_draw() && self.diagram.is_none();
        editor::set_menu_diagram_action(&self.text, diagram_available.then_some(create_diagram_from_menu));
        self.workspace_welcome.setHidden(!needs_workspace);
        self.header.setHidden(needs_workspace);
        self.title_divider.setHidden(needs_workspace || self.map.is_some());
        self.subtitle.setHidden(needs_workspace);
        self.status.setHidden(needs_workspace);
        self.save_button.setHidden(needs_workspace);
        let suggestion = self.editing_suggestion().is_some() && self.map.is_none();
        self.delete_button.setHidden(needs_workspace || suggestion);
        self.save_button.setTitle(&NSString::from_str(if suggestion { "등록" } else { "저장" }));
        self.save_button.setKeyEquivalent(&NSString::from_str(if suggestion { "" } else { "s" }));
        self.map_button.setHidden(needs_workspace);
        self.editor_scroll.setHidden(needs_workspace || self.preview.is_some() || self.map.is_some());
        let title = if let Some(canvas) = &self.diagram {
            canvas.title.clone()
        } else if self.map.is_some() {
            "지식지도".to_string()
        } else {
            self.document
                .as_ref()
                .map(|d| strv(d, "title"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| self.blank_title.clone())
        };
        let label = if self.diagram.is_some() { "다이어그램 제목" } else { "노트 제목" };
        self.header.setAccessibilityLabel(Some(&NSString::from_str(label)));
        if !self.editing_new_title {
            self.header.setPlaceholderString(Some(&NSString::from_str("제목 없음")));
            self.header.setStringValue(&NSString::from_str(if title == "제목 없음" { "" } else { &title }));
        }
        let can_edit = self.workspace.is_some() && self.map.is_none() && !self.snapshot;
        self.header.setEditable(can_edit);
        self.header.setSelectable(can_edit);
        self.update_analysis_help();
        self.layout_status(self.center.bounds().size.width);
        self.subtitle
            .setStringValue(&NSString::from_str(if self.map.is_some() {
                "원본에서 발견한 개념과 관계"
            } else if self.snapshot {
                "분석 당시 원본 · 읽기 전용 · 수정은 자료 보관함에서"
            } else if suggestion {
                "정리 제안 초안 · 변경 사항 자동 저장 · 등록하면 자료에 추가돼요"
            } else if self
                .document
                .as_ref()
                .map(|d| strv(d, "kind") == "md")
                .unwrap_or(false)
            {
                "이후 변경은 자동으로 저장됩니다"
            } else {
                "생각을 적어보세요 · 초안은 자동 보관됩니다"
            }));
        if self.preview.is_some() && self.map.is_none() && !self.snapshot {
            let subtitle = if self.preview_editable {
                "PDF 필기 · 변경 사항은 같은 파일에 자동 저장됩니다".to_owned()
            } else {
                format!(
                    "{} 원본 미리보기 · 읽기 전용",
                    self.document
                        .as_ref()
                        .map(|d| strv(d, "kind").to_uppercase())
                        .unwrap_or_default()
                )
            };
            self.subtitle.setStringValue(&NSString::from_str(&subtitle));
        }
        if self.preview_rendered && self.map.is_none() {
            self.subtitle.setStringValue(&NSString::from_str(
                "PPTX 미리보기 · 설치된 LibreOffice로 렌더링 · 원본 보관됨",
            ));
        }
        if self.preview_pending.is_some() && self.map.is_none() {
            self.subtitle.setStringValue(&NSString::from_str(
                "PPTX 미리보기 준비 중 · 설치된 LibreOffice로 렌더링",
            ));
        }
        if self.map.is_none() {
            if let Some(loc) = self
                .preview_locator
                .as_ref()
                .filter(|l| l["kind"] == "slide")
            {
                self.subtitle.setStringValue(&NSString::from_str(&format!(
                    "근거 슬라이드 {} · {}",
                    loc["index"],
                    self.subtitle.stringValue()
                )));
            }
        }
        if self.map.is_none() && !self.snapshot {
            if let Some(doc) = self.document.as_ref().filter(|d| d["kind"] == "md") {
                let parent = strv(doc, "parent");
                self.subtitle.setStringValue(&NSString::from_str(&format!(
                    "과목 루트{} · 변경 사항 자동 저장",
                    if parent.is_empty() {
                        String::new()
                    } else {
                        format!(" / {parent}")
                    }
                )));
            }
        }
        let content = !self.text.string().to_string().trim().is_empty();
        let saved_note = self.document.as_ref().is_some_and(|d| d["kind"] == "md");
        let writable = !self.snapshot
            && self.workspace.is_some()
            && self.map.is_none()
            && self.preview.is_none();
        let writable_pdf = !self.snapshot
            && self.map.is_none()
            && self.document.as_ref().is_some_and(|d| d["kind"] == "pdf");
        self.save_button
            .setEnabled(writable_pdf || (writable && (content || saved_note)));
        self.save_button
            .setAlphaValue(if writable_pdf || (writable && (content || saved_note)) {
                1.
            } else {
                0.45
            });
        self.delete_button.setEnabled(
            !self.snapshot
                && self.map.is_none()
                && (content || self.document.is_some() || self.preview.is_some()),
        );
        self.map_button
            .setTitle(&NSString::from_str(if self.map.is_some() {
                "문서로 돌아가기 ↙"
            } else {
                "지식지도 펼치기 ↗"
            }));
        if self.diagram.is_some(){
            self.subtitle.setHidden(true);
            self.save_button.setTitle(&NSString::from_str("완료"));self.save_button.setEnabled(true);
            self.save_button.setAlphaValue(1.);self.save_button.setKeyEquivalent(&NSString::new());
            self.delete_button.setTitle(&NSString::from_str("취소"));self.delete_button.setEnabled(true);
            self.editor_scroll.setHidden(true);self.map_button.setHidden(true);
        }else{self.delete_button.setTitle(&NSString::from_str("삭제"));}
    }
    fn can_draw(&self) -> bool {
        self.workspace.is_some()
            && self.map.is_none()
            && self.preview.is_none()
            && !self.snapshot
            && self
                .document
                .as_ref()
                .is_none_or(|d| matches!(d["kind"].as_str(), Some("md" | "draft")))
    }
    unsafe fn start_diagram(&mut self, link: Option<&str>, from_insert_menu: bool) {
        if !self.can_draw() || self.diagram.is_some() || !self.flush() {
            return;
        }
        let mut data = stology_core::diagram_svg::Diagram::default();
        let mut title = stology_core::document_title("").to_owned();
        let mut existing = None;
        if let Some(link) = link {
            let folder = self
                .document
                .as_ref()
                .and_then(|d| d["parent"].as_str())
                .unwrap_or("");
            let prefix = format!(
                "{}.diagram/",
                "../".repeat(std::path::Path::new(folder).components().count())
            );
            let Some(id) = link
                .strip_prefix(&prefix)
                .and_then(|f| f.strip_suffix(".svg"))
            else {
                return;
            };
            match rpc(json!({"op":"note.diagram.read","workspace_id":self.workspace,"id":id})) {
                Ok(result) => match serde_json::from_value(result["diagram"].clone()) {
                    Ok(d) => {
                        data = d;
                        title = stology_core::document_title(result["title"].as_str().unwrap_or("")).to_owned();
                        existing = Some((id.to_owned(), strv(&result, "hash")));
                    }
                    Err(e) => {
                        self.fail(&e.to_string());
                        return;
                    }
                },
                Err(e) => {
                    self.set_status(&e);
                    return;
                }
            }
        }
        let mut canvas =
            diagram_canvas::Canvas::new(data, title, msg_send![&*self.text, selectedRange], existing);
        canvas.insert_on_caret_line = from_insert_menu;
        self.center.addSubview(&canvas.root);
        self.diagram = Some(canvas);
        self.text.setEditable(false);
        self.update_header();
        self.layout_diagram();
        self.diagram.as_ref().unwrap().focus();
        self.set_status("도구를 고르고 캔버스에서 드래그하세요");
    }
    fn layout_diagram(&self) {
        let Some(canvas) = &self.diagram else { return };
        let size = self.center.bounds().size;
        let divider = self.title_divider.frame();
        canvas.layout(rect(0., 0., size.width, (divider.origin.y - 1.).max(1.)), divider);
    }
    unsafe fn close_diagram(&mut self) {
        if let Some(c) = self.diagram.take() {
            c.root.removeFromSuperview();
        }
        self.text.setEditable(true);
        self.update_header();
        self._window.makeFirstResponder(Some(&self.text));
    }
    unsafe fn discard_diagram(&mut self) -> bool {
        if self.diagram.is_none() { return true; }
        self.finish_new_title(true);
        if self.diagram.as_ref().is_some_and(|c| c.changed())
            && !confirm_action(
                "다이어그램 편집을 끝낼까요?",
                "저장하지 않은 그림 변경 사항이 사라집니다.",
                "변경 버리기",
            )
        {
            return false;
        }
        self.close_diagram();
        self.set_status("노트로 돌아왔습니다");
        true
    }
    unsafe fn finish_diagram(&mut self) {
        self.finish_new_title(true);
        let Some(c) = &self.diagram else { return };
        let data = c.document();
        if !data.has_content() {
            self.close_diagram();
            self.set_status("빈 다이어그램은 저장하지 않았습니다");
            return;
        }
        let selection = c.selection;
        let insert_on_caret_line = c.insert_on_caret_line;
        let existing = c.existing.clone();
        let folder = self
            .document
            .as_ref()
            .and_then(|d| d["parent"].as_str())
            .unwrap_or("");
        let req = json!({"op":"note.diagram.save","workspace_id":self.workspace,"folder":folder,"diagram":data,"title":c.title,
            "id":existing.as_ref().map(|e|&e.0),"expected_hash":existing.as_ref().map(|e|&e.1)});
        match rpc(req) {
            Err(e) => self.fail(&e),
            Ok(result) => {
                self.close_diagram();
                let base = self
                    .document
                    .as_ref()
                    .and_then(document_base)
                    .unwrap_or_else(|| {
                        std::path::Path::new(result["image_base"].as_str().unwrap())
                    });
                if existing.is_none() {
                    editor::insert_diagram(
                        &self.text,
                        selection,
                        insert_on_caret_line,
                        strv(&result, "destination").as_str(),
                        base,
                        editor::diagram_paint::content_bounds(&data).size.width.clamp(32., 520.),
                    );
                } else {
                    editor::style_with_base(&self.text, Some(base));
                }
                self.capture_change();
                self.set_status("다이어그램을 저장했습니다");
            }
        }
    }
    unsafe fn remember_screen(&mut self) {
        let Some(id) = self.workspace.clone() else { return };
        let pdf_page = self.preview.as_ref().and_then(|p| pdf_view(p)).and_then(|pdf| {
            let page: Option<Retained<AnyObject>> = msg_send![&*pdf, currentPage];
            let doc: Option<Retained<AnyObject>> = msg_send![&*pdf, document];
            let (page, doc) = (page?, doc?);
            Some(msg_send![&*doc, indexForPage: &*page])
        });
        let mut document = self.document.clone();
        if let Some(d) = document.as_mut().filter(|d| d["kind"] == "suggestion") {
            d["body"] = json!(self.last_text);
        }
        self.screens.insert(id, WorkspaceScreen {
            blank_title: self.blank_title.clone(),
            file_query: self.last_file_query.clone(),
            document,
            evidence: self.snapshot.then(|| self.current_evidence.clone()).flatten(),
            selection: NSTextInputClient::selectedRange(&*self.text),
            scroll: self.editor_scroll.contentView().bounds().origin,
            map_open: self.map.is_some(),
            folders: self.open_folders.clone(),
            pdf_page,
        });
    }

    unsafe fn restore_screen(&mut self) {
        self.show_blank();
        let screen = self.workspace.as_ref().and_then(|id| self.screens.remove(id));
        self.last_file_query = screen.as_ref().map(|s| s.file_query.clone()).unwrap_or_default();
        if let Some(field) = &self.file_search {
            field.setStringValue(&NSString::from_str(&self.last_file_query));
        }
        let Some(screen) = screen else { return };
        self.blank_title = screen.blank_title;
        self.open_folders = screen.folders;
        if let Some(e) = screen.evidence {
            self.evidence(e);
        } else if let Some(d) = screen.document {
            self.open_doc(d);
        }
        let length = self.text.string().length();
        let start = screen.selection.location.min(length);
        self.text.setSelectedRange(NSRange::new(start, screen.selection.length.min(length - start)));
        self.text.scrollPoint(screen.scroll);
        if let Some((pdf, index)) = self.preview.as_ref().and_then(|p| pdf_view(p)).zip(screen.pdf_page) {
            let doc: Option<Retained<AnyObject>> = msg_send![&*pdf, document];
            if let Some(doc) = doc {
                let count: usize = msg_send![&*doc, pageCount];
                if index < count {
                    let page: Option<Retained<AnyObject>> = msg_send![&*doc, pageAtIndex: index];
                    if let Some(page) = page { let _: () = msg_send![&*pdf, goToPage: &*page]; }
                }
            }
        }
        if screen.map_open { self.open_map(); }
    }

    unsafe fn show_blank(&mut self) {
        self.snapshot = false;
        self.document = None;
        self.blank_title = "제목 없음".into();
        self.last_text.clear();
        self.dirty = false;
        self.text.setString(&NSString::from_str(""));
        editor::style(&self.text);
        self.clear_undo();
        self.text.setEditable(self.workspace.is_some());
        self.hide_preview();
        self.update_header();
        self.set_status("새 노트 · 입력하면 초안으로 자동 보관됩니다");
    }
    unsafe fn clear_undo(&self) {
        let undo: Option<Retained<AnyObject>> = msg_send![&self.text, undoManager];
        if let Some(u) = undo {
            let _: () = msg_send![&u, removeAllActions];
        }
    }
    unsafe fn start_pptx_preview(&mut self, path: &str) {
        let placeholder = view(self.editor_scroll.frame());
        let label=text_label("PPTX 미리보기를 준비하고 있습니다.\n\n이 Mac에 설치된 LibreOffice로 슬라이드를 렌더링합니다.\n원본은 그대로 보관되며 앱에 LibreOffice를 포함하지 않습니다.\n다른 자료를 선택하면 준비를 취소합니다.",rect(28.,placeholder.frame().size.height-200.,placeholder.frame().size.width-56.,160.),14.,false);
        label.setMaximumNumberOfLines(9);
        label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        placeholder.addSubview(&label);
        self.editor_scroll.setHidden(true);
        self.center.addSubview(&placeholder);
        self.preview = Some(placeholder);
        self.preview_pending = Some((pptx_preview::start(path), path.to_string()));
        self.set_status("PPTX 렌더링 중 · 원본 보존");
    }
    unsafe fn hide_preview(&mut self) {
        if let Some((p, _)) = self.preview_pending.take() {
            p.cancel();
        }
        self.preview_rendered = false;
        self.preview_editable = false;
        self.preview_locator = None;
        self.last_pdf_status.clear();
        if let Some(p) = self.preview.take() {
            editor::close_preview(&p);
            p.removeFromSuperview();
        }
        self.editor_scroll.setHidden(false);
    }
    unsafe fn capture_change(&mut self) {
        if self.snapshot || self.map.is_some() || self.preview.is_some() {
            return;
        }
        let marked: bool = msg_send![&self.text, hasMarkedText];
        if marked {
            return;
        }
        let current = self.text.string().to_string();
        if current != self.last_text {
            self.last_text = current;
            self.dirty = true;
            self.edited_at = Instant::now();
            editor::style_with_base(&self.text, self.document.as_ref().and_then(document_base));
            self.set_status("작성 중…");
            self.update_header();
        }
    }
    unsafe fn flush(&mut self) -> bool {
        if !self.discard_diagram(){return false;}
        self.poll_pdf_sessions();
        pdf_session::request_flush_all();
        match pdf_session::flush_state() {
            Ok(false) => {
                self.set_status("PDF 저장 중… · 저장이 끝나면 다시 시도해 주세요");
                return false;
            }
            Err(e) => {
                self.set_status(&format!("PDF 저장 실패 · {e}"));
                return false;
            }
            Ok(true) => {}
        }
        if !self.flush_body() {
            return false;
        }
        self.finish_new_title(true)
    }
    unsafe fn poll_pdf_sessions(&mut self) {
        let events = pdf_session::poll();
        let mut changed = false;
        let mut resolved = Vec::new();
        for event in events {
            changed = true;
            if event.resolved {
                resolved.push((event.document_id.clone(), event.export_path.clone()));
                continue;
            }
            if let Some(doc) = self
                .document
                .as_mut()
                .filter(|d| !self.snapshot && d["id"].as_str() == Some(event.document_id.as_str()))
            {
                doc["version"] = json!(event.version);
                doc["hash"] = json!(event.hash);
                doc["absolute_path"] = json!(event.path);
            }
        }
        if let Some(doc) = self
            .document
            .as_ref()
            .filter(|d| !self.snapshot && d["kind"] == "pdf")
        {
            if let Some(status) = pdf_session::status(&strv(doc, "id")) {
                if status != self.last_pdf_status {
                    self.set_status(&if status.starts_with("저장 실패") {
                        format!("{status} · 파일 메뉴 > PDF 저장 문제 해결")
                    } else {
                        status.clone()
                    });
                    self.last_pdf_status = status;
                }
            }
        }
        if changed {
            self.refresh_right();
        }
        for (id, export_path) in resolved {
            if self
                .document
                .as_ref()
                .is_some_and(|d| d["id"].as_str() == Some(id.as_str()))
            {
                match rpc(json!({"op":"note.read","id":id})) {
                    Ok(doc) => {
                        self.open_doc(doc);
                    }
                    Err(e) => self.fail(&e),
                }
            }
            self.set_status(
                &export_path
                    .map(|path| format!("내 필기를 별도 PDF로 저장했습니다 · {path}"))
                    .unwrap_or_else(|| "미반영 필기를 버리고 현재 PDF를 다시 열었습니다".into()),
            );
        }
    }
    unsafe fn cancel_background_work(&mut self) {
        self.analysis_waiting = None;
        self.deferred_action = None;
        self.deferred_tree = None;
        // Only the step in progress is cancelled; finished steps stay for 이어서 정리.
        let _ = rpc(json!({"op":"analysis.cancel_active"}));
        if let Some(job) = &self.connection_job {
            let _ = rpc(json!({"op":"provider.job.cancel","id":job}));
        }
        if let Some(p) = &self.pending {
            p.cancel();
        }
        if let Some((p, _)) = self.preview_pending.take() {
            p.cancel();
        }
        if let Some((p, _, _)) = self.annotation_conversion.take() {
            p.cancel();
        }
    }
    unsafe fn finish_new_title(&mut self, commit: bool) -> bool {
        if !self.editing_new_title {
            return true;
        }
        let title = self
            .header
            .currentEditor()
            .map(|e| e.string().to_string())
            .unwrap_or_else(|| self.header.stringValue().to_string());
        let title = stology_core::document_title(&title).to_owned();
        let commit = TITLE_COMMAND.with(|pending| pending.replace(0) != -1) && commit;
        if let Some(canvas) = &mut self.diagram {
            if commit { canvas.title = title; }
            self.editing_new_title = false;
            self.header.abortEditing();
            self.update_header();
            return true;
        }
        let Some(doc) = self.document.clone() else {
            if commit { self.blank_title = title; }
            self.editing_new_title = false;
            self.header.abortEditing();
            self.update_header();
            return true;
        };
        if commit && title != strv(&doc, "title") {
            let request = if doc["kind"] == "suggestion" {
                json!({"op":"distill.update","id":doc["id"],"title":title,"body":self.last_text})
            } else {
                json!({"op":if doc["kind"] == "draft" { "draft.rename" } else { "note.rename" },"id":doc["id"],"title":title,"expected_version":doc["version"]})
            };
            match rpc(request) {
                Ok(updated) if doc["kind"] == "suggestion" => {
                    if let Some(document) = self.document.as_mut() {
                        document["title"] = updated["title"].clone();
                    }
                }
                Ok(updated) => {
                    self.document = Some(updated);
                }
                Err(error) => {
                    self.set_status(&format!(
                        "제목 수정 실패 · {error} · 다른 이름을 입력하거나 Esc로 취소하세요"
                    ));
                    self.header.setStringValue(&NSString::from_str(&title));
                    self.header.selectText(None);
                    return false;
                }
            }
        }
        self.editing_new_title = false;
        self.header.abortEditing();
        self.header.setEditable(false);
        self.header.setSelectable(false);
        self.set_status(if commit {
            "노트 제목을 저장했습니다 · 변경 사항은 자동 저장됩니다"
        } else {
            "기본 제목을 유지했습니다 · 변경 사항은 자동 저장됩니다"
        });
        self.update_header();
        self.refresh_right();
        true
    }
    unsafe fn flush_body(&mut self) -> bool {
        if editor::table_cell_is_editing(&self.text) {
            self._window.endEditingFor(None);
        }
        let marked: bool = msg_send![&self.text, hasMarkedText];
        if marked {
            let _: () = msg_send![&self.text, unmarkText];
        }
        self.capture_change();
        if !self.dirty {
            return true;
        }
        if self.workspace.is_none() {
            return false;
        }
        let body = self.last_text.clone();
        let ws = self.workspace.clone().unwrap();
        if body.trim().is_empty() && self.document.is_none() {
            self.dirty = false;
            return true;
        }
        if let Some(doc) = self.editing_suggestion() {
            if body.trim().is_empty() {
                self.set_status("저장 실패 · 초안 내용을 입력해 주세요");
                return false;
            }
            match rpc(json!({"op":"distill.update","id":doc["id"],"title":doc["title"],"body":body})) {
                Ok(_) => {
                    self.dirty = false;
                    self.set_status("정리 제안 초안에 저장됨");
                    return true;
                }
                Err(e) => {
                    self.set_status(&format!("저장 실패 · {e}"));
                    return false;
                }
            }
        }
        let request = if let Some(d) = &self.document {
            let kind = strv(d, "kind");
            json!({"op":if kind=="draft"{"draft.update"}else{"note.update"},"workspace_id":ws,"id":d["id"],"body":body,"expected_version":d["version"]})
        } else {
            json!({"op":"draft.create","workspace_id":ws,"body":body})
        };
        let refresh_drafts = self.document.as_ref().is_none_or(|d| d["kind"] == "draft");
        let state_before = self.document.as_ref().map(|d| strv(d, "analysis_state"));
        match rpc(request) {
            Ok(mut d) => {
                // 자료 트리의 상태와 과목 정리 줄의 `정리 필요` 수는 이 노트의 상태를 센다.
                let state_turned = d["id"].is_string() && state_before.as_deref() != Some(d["analysis_state"].as_str().unwrap_or(""));
                if d["id"].is_string() {
                    if self.document.is_none() && self.blank_title != "제목 없음" {
                        match rpc(json!({"op":"draft.rename","id":d["id"],"title":self.blank_title,"expected_version":d["version"]})) {
                            Ok(renamed) => d = renamed,
                            Err(e) => { self.document = Some(d); self.set_status(&e); return false; }
                        }
                    }
                    self.document = Some(d);
                } else if body.trim().is_empty() {
                    self.document = None;
                }
                self.dirty = false;
                self.set_status("이 Mac에 저장됨");
                self.update_header();
                if refresh_drafts || state_turned {
                    self.refresh_right();
                }
                true
            }
            Err(e) => {
                self.set_status(&format!("저장 실패 · {e}"));
                false
            }
        }
    }
    unsafe fn open_doc(&mut self, d: Value) -> bool {
        if !self.flush() {
            return false;
        }
        if self.map.is_some() {
            self.close_map();
        }
        let id = strv(&d, "id");
        let kind = strv(&d, "kind");
        let data = if kind == "suggestion" {
            Ok(d.clone())
        } else if kind == "draft" {
            rpc(json!({"op":"draft.get","id":id}))
        } else {
            rpc(json!({"op":"note.read","id":id}))
        };
        // A restart conflict must remain reachable: metadata identifies the
        // working file without accepting its new hash. The PDF worker opens
        // rescue mode and keeps normal publication blocked until resolution.
        let recovery_error = data
            .as_ref()
            .err()
            .cloned()
            .filter(|_| kind == "pdf" && d["absolute_path"].is_string());
        let data = if recovery_error.is_some() {
            Ok(d.clone())
        } else {
            data
        };
        match data {
            Ok(doc) => {
                self.snapshot = false;
                self.hide_preview();
                self.document = Some(doc.clone());
                self.set_status("이 Mac에 보관된 자료");
                let kind = strv(&doc, "kind");
                if kind == "pptx" {
                    self.set_status("PPTX · 아이콘만 표시되면 시스템 미리보기 미지원입니다. PDF로 변환해 가져오세요.");
                }
                if kind == "md" || kind == "draft" || doc["body"].is_string() {
                    let body = strv(&doc, "body");
                    self.text.setString(&NSString::from_str(&body));
                    self.clear_undo();
                    self.last_text = body;
                    self.text.setEditable(true);
                    editor::style_with_base(
                        &self.text,
                        self.document.as_ref().and_then(document_base),
                    );
                } else {
                    self.text.setEditable(false);
                    self.text.setString(&NSString::from_str(""));
                    self.last_text.clear();
                    self.clear_undo();
                    editor::style(&self.text);
                    let path = strv(&doc, "absolute_path");
                    if kind == "pptx" && pptx_preview::available() {
                        self.start_pptx_preview(&path);
                    } else {
                        let preview = if kind == "pdf" {
                            match editor::editable_pdf(&path, self.editor_scroll.frame(), &id) {
                                Ok(view) => {
                                    self.preview_editable = true;
                                    Ok(view)
                                }
                                Err(error) => {
                                    self.set_status(&format!("읽기 전용 PDF · {error}"));
                                    editor::preview(&path, self.editor_scroll.frame(), None)
                                }
                            }
                        } else {
                            editor::preview(&path, self.editor_scroll.frame(), None)
                        };
                        match preview {
                            Ok(p) => {
                                self.editor_scroll.setHidden(true);
                                self.center.addSubview(&p);
                                self.preview = Some(p);
                            }
                            Err(e) => {
                                let p = view(self.editor_scroll.frame());
                                let label = text_label(
                                    &format!(
                                    "원본을 표시할 수 없습니다\n\n{e}\n\n보관된 파일은 유지됩니다."
                                ),
                                    rect(
                                        28.,
                                        p.frame().size.height - 180.,
                                        p.frame().size.width - 56.,
                                        140.,
                                    ),
                                    14.,
                                    false,
                                );
                                label.setMaximumNumberOfLines(8);
                                label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                                p.addSubview(&label);
                                self.editor_scroll.setHidden(true);
                                self.center.addSubview(&p);
                                self.preview = Some(p);
                                self.set_status("원본 미리보기 실패 · 보관 파일 유지");
                            }
                        }
                    }
                }
                self.document = Some(doc);
                self.dirty = false;
                self.sync_tree_selection();
                self.update_header();
                if let Some(error) = recovery_error {
                    self.set_status(&format!(
                        "PDF 복구 확인 필요 · {error} · 파일 메뉴 > PDF 저장 문제 해결"
                    ));
                }
                true
            }
            Err(e) => {
                self.fail(&e);
                false
            }
        }
    }
    unsafe fn sync_tree_selection(&self) {
        if let Some(tree) = &self.document_tree {
            tree.select_document(self.document.as_ref().and_then(|d| d["id"].as_str()));
        }
    }
    unsafe fn handle_tree_events(&mut self) {
        let mut events = Vec::new();
        if let Some(event) = self.deferred_tree.take() {
            events.push(event);
        }
        events.extend(document_tree::drain());
        for event in events {
            let requires_save = !matches!(
                &event,
                document_tree::Event::ToggleFolder(_, _)
                    | document_tree::Event::OpenFinder(_)
                    | document_tree::Event::Analyze(_)
                    | document_tree::Event::Reread(_)
                    | document_tree::Event::Annotate(_)
                    | document_tree::Event::Reconvert(_)
            );
            if requires_save && pdf_session::has_pending() {
                pdf_session::request_flush_all();
                match pdf_session::flush_state() {
                    Ok(false) => {
                        self.deferred_tree = Some(event);
                        self.set_status("PDF 저장 후 요청한 작업을 계속합니다…");
                        continue;
                    }
                    Err(e) => {
                        self.fail(&e);
                        continue;
                    }
                    Ok(true) => {}
                }
            }
            match event {
                document_tree::Event::ToggleFolder(path, expanded) => {
                    if expanded {
                        self.open_folders.insert(path);
                    } else {
                        self.open_folders.remove(&path);
                    }
                }
                document_tree::Event::Open(doc) => {
                    if self.workspace.as_deref() != doc["workspace_id"].as_str() {
                        continue;
                    }
                    if !self.snapshot
                        && self.document.as_ref().is_some_and(|d| d["id"] == doc["id"])
                    {
                        continue;
                    }
                    let had_draft = self.document.as_ref().is_some_and(|d| d["kind"] == "draft");
                    self.open_doc(doc);
                    self.sync_tree_selection();
                    if had_draft {
                        self.refresh_right();
                    }
                }
                document_tree::Event::OpenFinder(path) => open_tree_item_in_finder(&path),
                document_tree::Event::Analyze(doc) => self.analyze_document(doc),
                document_tree::Event::Reread(doc) => {
                    if self.workspace.as_deref() == doc["workspace_id"].as_str() && !self.analysis_busy() {
                        self.open_organize(Some((strv(&doc, "id"), strv(&doc, "title"))));
                    }
                }
                document_tree::Event::Annotate(doc) => self.annotate_pptx(doc, false),
                document_tree::Event::Reconvert(doc) => self.annotate_pptx(doc, true),
                document_tree::Event::Delete(doc) => self.delete_document(doc),
                document_tree::Event::Rename(doc) => self.begin_tree_rename(doc),
                document_tree::Event::CommitRename(doc, title) => {
                    self.relocate_document(doc, None, Some(title))
                }
                document_tree::Event::NewNote(parent) => self.create_tree_note(&parent),
                document_tree::Event::RenameFolder(path) => self.rename_tree_folder(&path),
                document_tree::Event::NewFolder(parent) => self.create_tree_folder(&parent),
                document_tree::Event::CopyFiles(paths, folder) => {
                    self.copy_tree_files(paths, &folder)
                }
                document_tree::Event::Move(doc, folder) => {
                    self.relocate_document(doc, Some(folder), None)
                }
            }
        }
    }
    unsafe fn copy_tree_files(&mut self, paths: Vec<String>, folder: &str) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if !self.flush() {
            return;
        }
        let mut copied = 0;
        let mut errors = Vec::new();
        for path in paths {
            match rpc(
                json!({"op":"material.copy","workspace_id":workspace,"path":path,"folder":folder}),
            ) {
                Ok(_) => copied += 1,
                Err(error) => {
                    let name = std::path::Path::new(&path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    errors.push(format!("{name}: {error}"));
                }
            }
        }
        if copied > 0 {
            self.last_file_query.clear();
            if let Some(field) = &self.file_search {
                field.setStringValue(&NSString::from_str(""));
            }
            let parts: Vec<_> = folder.split('/').filter(|p| !p.is_empty()).collect();
            for i in 1..=parts.len() {
                self.open_folders.insert(parts[..i].join("/"));
            }
            self.refresh_right();
        }
        let destination = if folder.is_empty() {
            "과목 루트"
        } else {
            folder
        };
        let summary = format!("파일 {copied}개를 {destination}에 복사했습니다");
        if errors.is_empty() {
            self.set_status(&summary);
        } else {
            self.fail(&format!(
                "{summary}\n복사하지 못한 파일 {}개\n{}",
                errors.len(),
                errors.join("\n")
            ));
        }
    }
    unsafe fn create_tree_note(&mut self, parent: &str) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if !self.flush() {
            return;
        }
        match rpc(json!({"op":"note.create","workspace_id":workspace,"folder":parent})) {
            Ok(doc) => {
                // Reveal the destination even if a previous search hid it.
                self.last_file_query.clear();
                if let Some(field) = &self.file_search {
                    field.setStringValue(&NSString::from_str(""));
                }
                let parts: Vec<_> = parent.split('/').filter(|p| !p.is_empty()).collect();
                for i in 1..=parts.len() {
                    self.open_folders.insert(parts[..i].join("/"));
                }
                if self.open_doc(doc) {
                    self.refresh_right();
                    self.sync_tree_selection();
                    self.editing_new_title = true;
                    self.header.setEditable(true);
                    self.header.setSelectable(true);
                    self.header.selectText(None);
                    self.set_status("폴더에 새 노트를 만들었습니다 · 제목 입력 후 Enter로 작성 · Esc로 기본 제목 유지");
                }
            }
            Err(error) => self.fail(&error),
        }
    }
    unsafe fn rename_tree_folder(&mut self, old: &str) {
        let Some(workspace) = self.workspace.clone() else { return };
        if !self.flush() { return }
        let initial = old.rsplit('/').next().unwrap_or(old);
        let Some(name) = input("폴더 이름 변경", "새 폴더 이름을 입력하세요.", initial, "변경") else { return };
        match rpc(json!({"op":"folder.rename","workspace_id":workspace,"path":old,"name":name})) {
            Ok(folder) => {
                let path = strv(&folder, "path");
                let prefix = format!("{old}/");
                self.open_folders = self.open_folders.iter().map(|entry| {
                    if entry == old { path.clone() }
                    else if let Some(rest) = entry.strip_prefix(&prefix) { format!("{path}/{rest}") }
                    else { entry.clone() }
                }).collect();
                if let Some(doc) = self.document.clone().filter(|d| !self.snapshot && strv(d,"path").starts_with(&prefix)) {
                    self.open_doc(doc);
                }
                self.refresh_right();
                if let Some(tree) = &self.document_tree { tree.reveal_folder(&path); }
                self.set_status("폴더 이름을 변경했습니다");
            }
            Err(error) => self.fail(&error),
        }
    }
    unsafe fn create_tree_folder(&mut self, parent: &str) {
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if self.map.is_some() {
            return;
        }
        if !self.flush() {
            self.fail("현재 편집 내용을 저장하지 못했습니다. 내용을 보존한 뒤 다시 시도해 주세요.");
            return;
        }
        let destination = if parent.is_empty() {
            "과목 루트"
        } else {
            parent
        };
        let explanation = format!("‘{destination}’ 안에 만들 폴더 이름을 입력하세요.");
        let prompt = NSAlert::new(mtm());
        prompt.setMessageText(&NSString::from_str("새 폴더"));
        prompt.setInformativeText(&NSString::from_str(&explanation));
        let field = NSTextField::initWithFrame(NSTextField::alloc(mtm()), rect(0., 0., 360., 28.));
        field.setFont(Some(&font::regular(13.)));
        let _: () = msg_send![&field, setAccessibilityLabel: &*NSString::from_str("폴더 이름")];
        prompt.setAccessoryView(Some(&field));
        prompt.addButtonWithTitle(&NSString::from_str("만들기"));
        prompt.addButtonWithTitle(&NSString::from_str("취소"));
        loop {
            prompt.window().setInitialFirstResponder(Some(&field));
            field.selectText(None);
            if prompt.runModal() != 1000 {
                return;
            }
            match rpc(
                json!({"op":"folder.create","workspace_id":workspace,"parent":parent,"name":field.stringValue().to_string()}),
            ) {
                Ok(folder) => {
                    let path = strv(&folder, "path");
                    let parts: Vec<_> = path.split('/').collect();
                    for i in 1..=parts.len() {
                        self.open_folders.insert(parts[..i].join("/"));
                    }
                    self.refresh_right();
                    if let Some(tree) = &self.document_tree {
                        tree.reveal_folder(&path);
                    }
                    self.set_status(&format!("‘{}’ 폴더를 만들었습니다", strv(&folder, "name")));
                    return;
                }
                Err(error) => {
                    prompt.setInformativeText(&NSString::from_str(&format!(
                        "{explanation}\n\n{error}"
                    )));
                }
            }
        }
    }
    unsafe fn begin_tree_rename(&mut self, doc: Value) {
        if self.workspace.as_deref() != doc["workspace_id"].as_str() || self.map.is_some() {
            return;
        }
        // Save before focusing the row: saving a draft can rebuild the tree.
        if !self.flush() {
            self.set_status("현재 편집 내용을 저장하지 못해 제목 수정을 시작하지 못했습니다.");
            return;
        }
        if let Some(tree) = &self.document_tree {
            tree.begin_rename(&doc, None, None);
        }
    }
    unsafe fn file_change_error(&self, doc: &Value, title: Option<&str>, error: &str) {
        if let Some(title) = title {
            self.set_status(&format!("제목 수정 실패 · {error}"));
            if let Some(tree) = &self.document_tree {
                tree.begin_rename(doc, Some(title), Some(error));
            }
        } else {
            self.fail(error);
        }
    }
    unsafe fn relocate_document(
        &mut self,
        doc: Value,
        folder: Option<String>,
        rename_title: Option<String>,
    ) {
        // A context click targets its own row, independently of the open document.
        if self.workspace.as_deref() != doc["workspace_id"].as_str() {
            return;
        }
        if !self.flush() {
            self.file_change_error(
                &doc,
                rename_title.as_deref(),
                "현재 편집 내용을 저장하지 못해 파일 변경을 중단했습니다.",
            );
            if rename_title.is_none() {
                self.sync_tree_selection();
            }
            return;
        }
        let id = strv(&doc, "id");
        let current = match rpc(json!({"op":"note.read","id":id})) {
            Ok(doc) => doc,
            Err(e) => {
                self.file_change_error(&doc, rename_title.as_deref(), &e);
                return;
            }
        };
        if !self.snapshot
            && self.document.as_ref().is_some_and(|opened| {
                opened["id"] == current["id"] && opened["hash"] != current["hash"]
            })
        {
            // note.read can discover an external edit. Never pair its new hash/version
            // with the old editor text: the next autosave would overwrite that edit.
            self.file_change_error(&doc, rename_title.as_deref(), "원본이 외부에서 변경되었습니다. 자료를 다시 연 뒤 제목이나 위치를 변경해 주세요. 현재 편집 내용은 유지됩니다.");
            return;
        }
        let request = if let Some(ref folder) = folder {
            json!({"op":"note.move","id":id,"folder":folder,"expected_version":current["version"]})
        } else if let Some(ref title) = rename_title {
            json!({"op":"note.rename","id":id,"title":title,"expected_version":current["version"]})
        } else {
            return;
        };
        match rpc(request) {
            Ok(updated) => {
                let parent = strv(&updated, "parent");
                let parts: Vec<_> = parent.split('/').filter(|p| !p.is_empty()).collect();
                for i in 1..=parts.len() {
                    self.open_folders.insert(parts[..i].join("/"));
                }
                if !self.snapshot
                    && self
                        .document
                        .as_ref()
                        .is_some_and(|d| d["id"] == updated["id"])
                {
                    self.document = Some(updated.clone());
                    if updated["kind"] == "md" {
                        let rewritten = strv(&updated, "body");
                        if rewritten != self.last_text {
                            let caret: NSRange = msg_send![&self.text, selectedRange];
                            self.text.setString(&NSString::from_str(&rewritten));
                            self.text.setSelectedRange(NSRange::new(
                                caret.location.min(rewritten.encode_utf16().count()),
                                0,
                            ));
                            self.last_text = rewritten;
                            self.dirty = false;
                            self.clear_undo();
                        }
                        // Resolve images from the new document folder.
                        editor::style_with_base(
                            &self.text,
                            updated["absolute_path"]
                                .as_str()
                                .and_then(|p| std::path::Path::new(p).parent()),
                        );
                    } else {
                        pdf_session::forget(&strv(&updated, "id"));
                        self.open_doc(updated);
                    }
                }
                self.refresh_right();
                self.update_header();
                self.sync_tree_selection();
                self.set_status(if folder.is_some() {
                    "자료의 보관 위치를 변경했습니다"
                } else {
                    "자료 제목을 수정했습니다"
                });
            }
            Err(e) => {
                // A failed response may follow a completed physical move. Reading
                // replays its journal; reconcile metadata before the next autosave.
                if !self.snapshot
                    && self
                        .document
                        .as_ref()
                        .is_some_and(|d| d["id"] == current["id"])
                {
                    if let Ok(recovered) = rpc(json!({"op":"note.read","id":current["id"]})) {
                        if self
                            .document
                            .as_ref()
                            .is_some_and(|d| d["hash"] == recovered["hash"])
                        {
                            self.document = Some(recovered.clone());
                            if recovered["kind"] == "md" {
                                editor::style_with_base(
                                    &self.text,
                                    recovered["absolute_path"]
                                        .as_str()
                                        .and_then(|p| std::path::Path::new(p).parent()),
                                );
                            } else {
                                pdf_session::forget(&strv(&recovered, "id"));
                                self.open_doc(recovered);
                            }
                        }
                    }
                }
                self.refresh_right();
                self.update_header();
                self.file_change_error(&doc, rename_title.as_deref(), &e);
            }
        }
    }
    unsafe fn share_pdf(&mut self) {
        let Some(doc) = self
            .document
            .as_ref()
            .filter(|d| !self.snapshot && d["kind"] == "pdf")
        else {
            self.set_status("공유할 PDF를 먼저 열어 주세요");
            return;
        };
        self.share_waiting = Some(strv(doc, "id"));
        pdf_session::request_flush_all();
        self.set_status("최신 필기 저장 후 PDF를 공유합니다…");
        self.poll_pdf_share();
    }
    unsafe fn poll_pdf_share(&mut self) {
        let Some(id) = self.share_waiting.clone() else {
            return;
        };
        match pdf_session::flush_state() {
            Ok(false) => return,
            Err(e) => {
                self.share_waiting = None;
                self.fail(&format!("저장 실패로 공유를 중단했습니다: {e}"));
                return;
            }
            Ok(true) => {}
        }
        self.share_waiting = None;
        let doc = match rpc(json!({"op":"note.read","id":id})) {
            Ok(doc) => doc,
            Err(e) => {
                self.fail(&e);
                return;
            }
        };
        let item: Retained<AnyObject> = Retained::cast_unchecked(NSURL::fileURLWithPath(
            &NSString::from_str(&strv(&doc, "absolute_path")),
        ));
        let items = NSArray::from_retained_slice(&[item]);
        let picker = NSSharingServicePicker::initWithItems(NSSharingServicePicker::alloc(), &items);
        picker.showRelativeToRect_ofView_preferredEdge(
            self.save_button.bounds(),
            &self.save_button,
            NSRectEdge::MinY,
        );
        self.share_picker = Some(picker);
        self.set_status("저장된 PDF 파일을 공유합니다");
    }
    unsafe fn resolve_pdf_save(&mut self) {
        let Some(doc) = self
            .document
            .clone()
            .filter(|d| !self.snapshot && d["kind"] == "pdf")
        else {
            self.set_status("저장 문제가 있는 PDF를 먼저 열어 주세요");
            return;
        };
        let id = strv(&doc, "id");
        let Some(error) = pdf_session::status(&id).filter(|s| s.starts_with("저장 실패"))
        else {
            self.set_status("이 PDF에 저장 오류가 없습니다");
            return;
        };
        let prompt = NSAlert::new(mtm());
        prompt.setMessageText(&NSString::from_str("PDF 저장 문제 해결"));
        prompt.setInformativeText(&NSString::from_str(&format!("{error}\n\n현재 파일은 보존됩니다. 내 필기를 별도 PDF로 저장하거나, 미반영 필기를 버리고 현재 파일을 다시 열 수 있습니다.")));
        prompt.addButtonWithTitle(&NSString::from_str("내 필기를 별도 PDF로 저장"));
        prompt.addButtonWithTitle(&NSString::from_str("버리고 다시 열기"));
        prompt.addButtonWithTitle(&NSString::from_str("취소"));
        let result = match prompt.runModal() {
            1000 => {
                let panel = NSSavePanel::savePanel(mtm());
                panel.setTitle(Some(&NSString::from_str("내 필기를 별도 PDF로 저장")));
                panel.setNameFieldStringValue(&NSString::from_str(&format!(
                    "{} (필기 복구).pdf",
                    strv(&doc, "title")
                )));
                if panel.runModal() != 1 {
                    return;
                }
                let Some(path) = panel
                    .URL()
                    .and_then(|url| url.path())
                    .map(|p| p.to_string())
                else {
                    return;
                };
                pdf_session::request_export(&id, &path)
            }
            1001 => {
                if !confirm_action("미반영 필기를 버리시겠습니까?", "PDF에 아직 반영하지 못한 내 필기는 삭제됩니다. 외부에서 변경된 현재 파일은 그대로 다시 엽니다.", "미반영 필기 버리기") { return; }
                pdf_session::discard_recovery(&id)
            }
            _ => return,
        };
        match result {
            Ok(()) => self.set_status("PDF 복구 작업 중…"),
            Err(e) => self.fail(&e),
        }
    }
    unsafe fn save(&mut self) {
        if self.diagram.is_some(){return}
        if !self.snapshot && self.document.as_ref().is_some_and(|d| d["kind"] == "pdf") {
            pdf_session::request_flush_all();
            self.poll_pdf_sessions();
            match pdf_session::flush_state() {
                Ok(true) => self.set_status("PDF에 저장됨"),
                Ok(false) => self.set_status("PDF 저장 중…"),
                Err(e) => self.set_status(&format!("PDF 저장 실패 · {e}")),
            }
            return;
        }
        if self.editing_suggestion().is_some() && self.map.is_none() {
            if self.flush() {
                self.set_status("정리 제안 초안에 저장됨");
            }
            return;
        }
        if self.snapshot
            || self.map.is_some()
            || self.preview.is_some()
            || (self.text.string().to_string().trim().is_empty()
                && !self.document.as_ref().is_some_and(|d| d["kind"] == "md"))
        {
            return;
        }
        if !self.flush() {
            alert(
                "먼저 초안 보관을 완료해야 합니다",
                "현재 내용은 편집기에 유지됩니다. 저장 위치와 디스크 상태를 확인하세요.",
            );
            return;
        }
        if let Some(d) = self.document.clone() {
            if strv(&d, "kind") != "draft" {
                return;
            }
            let ws = self.workspace.clone().unwrap();
            let folders = rpc(json!({"op":"tree","workspace_id":ws})).unwrap_or(json!({}));
            let suggested = d["title"].as_str().unwrap_or("제목 없음");
            let Some((title, folder)) = save_dialog::choose(&ws, suggested, &folders) else {
                self.refresh_right();
                return;
            };
            match rpc(json!({"op":"note.save","draft_id":d["id"],"title":title,"folder":folder})) {
                Ok(saved) => {
                    if saved["conflict"] == true {
                        alert(
                            "같은 이름의 노트가 있습니다",
                            &format!(
                                "기존 파일은 보존했습니다. ‘{}’ 제목으로 다시 저장해 주세요.",
                                strv(&saved, "suggested_title")
                            ),
                        );
                        return;
                    }
                    let rewritten = strv(&saved, "body");
                    if rewritten != self.last_text {
                        self.text.setString(&NSString::from_str(&rewritten));
                        self.last_text = rewritten;
                        self.clear_undo();
                    }
                    self.document = Some(saved);
                    editor::style_with_base(
                        &self.text,
                        self.document.as_ref().and_then(document_base),
                    );
                    let parts: Vec<_> = folder.split('/').filter(|p| !p.is_empty()).collect();
                    for i in 1..=parts.len() {
                        self.open_folders.insert(parts[..i].join("/"));
                    }
                    self.set_status("노트를 저장했습니다");
                    self.refresh();
                }
                Err(e) => self.fail(&e),
            }
        }
    }
    unsafe fn delete(&mut self) {
        if self.snapshot || self.editing_suggestion().is_some() {
            return;
        }
        if let Some(doc) = self.document.clone() { self.delete_document(doc); }
    }
    unsafe fn delete_document(&mut self, mut doc: Value) {
        let current = self.document.as_ref().is_some_and(|open| open["id"] == doc["id"]);
        if current {
            if !self.flush() { return; }
            if let Some(open) = &self.document { doc["version"] = open["version"].clone(); }
        }
        let title = strv(&doc, "title");
        if !confirm(
            &format!(
                "‘{}’ 삭제",
                if title.is_empty() {
                    "작성 중인 초안"
                } else {
                    &title
                }
            ),
            "선택한 항목을 휴지통으로 이동합니다. 이 자료만의 지도 근거도 제거됩니다.",
        ) {
            return;
        }
        let op = if strv(&doc, "kind") == "draft" {
            "draft.delete"
        } else {
            "material.delete"
        };
        match rpc(json!({"op":op,"id":doc["id"],"expected_version":doc["version"]})) {
            Ok(_) => {
                pdf_session::forget(&strv(&doc, "id"));
                if current { self.dirty = false; self.show_blank(); }
                self.refresh_right();
            }
            Err(e) => self.fail(&e),
        }
    }
    unsafe fn connect(&mut self) {
        let state = rpc(json!({"op":"provider.status"})).unwrap_or(json!({}));
        let (action, state) = provider_dialog::show(&state);
        if matches!(state["setup_job"]["state"].as_str(), Some("queued" | "running")) {
            self.connection_job = state["setup_job"]["id"].as_str().map(str::to_owned);
            self.connection_installing = true;
            self.connection_node = true;
            self.set_status("Node.js 백그라운드 설치 중");
        }
        if self.connection_node && !matches!(state["setup_job"]["state"].as_str(), Some("queued" | "running")) {
            self.connection_job = None; self.connection_node = false;
        }
        // 연결한 공급자는 모두 채팅에서 쓴다. 자료 분석 공급자가 아직 없을 때만
        // 방금 연결을 시작한 공급자나 이미 연결된 첫 공급자로 정한다.
        if self.provider.is_empty() {
            let pick = action.as_ref().map(|(p, _)| p.clone()).or_else(|| provider_dialog::first_connected(&state));
            if let Some(p) = pick {
                self.provider = p;
                let _ = rpc(json!({"op":"provider.set","provider":self.provider}));
            }
        }
        if let Some((provider, act)) = action {
            let install = act == provider_dialog::Action::Install;
            match rpc(json!({"op":if install {"provider.install"} else {"provider.login"},"provider":provider})) {
                Ok(v) => {
                    self.connection_job = v["id"].as_str().map(str::to_string);
                    self.connection_installing = install;
                    self.connection_node = false;
                    self.set_status(if install { "공식 CLI 설치 진행 중" } else { "공식 도구 연결 진행 중 · 브라우저에서 완료하세요" });
                }
                Err(e) => self.fail(&e),
            }
        }
        self.update_provider_row();
        self.probe_provider();
    }
    /// `provider.status`를 백그라운드에서 다시 읽는다. 이미 읽는 중이면 끝난 뒤 한 번 더 읽는다.
    fn probe_provider(&mut self) {
        if self.provider_probe.is_some() {
            self.provider_probe_again = true;
            return;
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(rpc(json!({"op":"provider.status"})).unwrap_or(Value::Null));
        });
        self.provider_probe = Some(rx);
    }
    fn poll_provider_probe(&mut self) {
        let Some(rx) = &self.provider_probe else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(v) => Some(v),
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        self.provider_probe = None;
        if let Some(v) = result.filter(|v| v["providers"].is_array()) {
            self.provider_info = Some(v);
        }
        if std::mem::take(&mut self.provider_probe_again) {
            self.probe_provider();
        }
        unsafe { self.update_provider_row() };
    }
    /// 선택한 공급자의 설치·로그인 상태를 (배지 심볼, 배지 색, 헬퍼텍스트)로 바꾼다.
    /// 헬퍼텍스트는 좁은 사이드바에 맞춰 짧게, 자세한 안내는 툴팁으로 준다.
    fn provider_view(&self) -> (&'static str, ProviderTone, String, String) {
        let (cli, plan) = if self.provider == "antigravity" {
            ("Antigravity CLI", "Google 계정")
        } else if self.provider == "claude" {
            ("Claude Code CLI", "Claude")
        } else {
            ("Codex CLI", "ChatGPT")
        };
        if self.connection_job.is_some() {
            return (
                "person.crop.circle.badge.clock",
                ProviderTone::Busy,
                if self.connection_installing {
                    if self.connection_node { "Node.js 설치 중…".into() } else { "CLI 설치 중…".into() }
                } else {
                    "브라우저에서 로그인 중…".into()
                },
                if self.connection_installing {
                    if self.connection_node { "Node.js를 백그라운드에서 설치하고 있어요".into() } else { format!("{cli}를 설치하고 있어요") }
                } else {
                    "브라우저에서 로그인을 마쳐 주세요".into()
                },
            );
        }
        let connected = self.provider_info.as_ref().map(provider_dialog::connected_names).unwrap_or_default();
        if !connected.is_empty() {
            let names = connected.join(" · ");
            return (
                "person.crop.circle.badge.checkmark",
                ProviderTone::Ok,
                format!("{names} 연결됨"),
                format!("{names} 연결됨 · 눌러서 구독 연결을 관리하세요"),
            );
        }
        if self.provider.is_empty() {
            return (
                "person.crop.circle.badge.questionmark",
                ProviderTone::Idle,
                "구독을 선택해 주세요".into(),
                "눌러서 분석에 쓸 구독을 선택하세요".into(),
            );
        }
        let entry = self.provider_info.as_ref().and_then(|v| {
            v["providers"]
                .as_array()?
                .iter()
                .find(|p| p["provider"] == self.provider.as_str())
                .cloned()
        });
        match entry {
            None => (
                "person.crop.circle.badge.checkmark",
                ProviderTone::Idle,
                "연결 상태 확인 중…".into(),
                "공식 CLI의 설치·로그인 상태를 확인하고 있어요".into(),
            ),
            Some(p) if p["installed"] != true => (
                "person.crop.circle.badge.xmark",
                ProviderTone::Error,
                "CLI 설치가 필요해요".into(),
                format!("{cli}가 없습니다 · 눌러서 설치하세요"),
            ),
            Some(p) if p["logged_in"] != true => (
                "person.crop.circle.badge.exclamationmark",
                ProviderTone::Warn,
                "로그인이 필요해요".into(),
                format!("{} 로그인이 필요합니다 · 눌러서 로그인하세요", if self.provider=="antigravity" {plan.to_owned()} else {format!("{plan} 구독")}),
            ),
            Some(_) => (
                "person.crop.circle.badge.checkmark",
                ProviderTone::Ok,
                format!("{} 연결됨", if self.provider=="antigravity" {plan.to_owned()} else {format!("{plan} 구독")}),
                format!("{cli} · {}으로 연결되어 있어요", if self.provider=="antigravity" {plan.to_owned()} else {format!("{plan} 구독")}),
            ),
        }
    }
    unsafe fn update_provider_row(&mut self) {
        let (symbol, tone, text, detail) = self.provider_view();
        let detail = NSString::from_str(&detail);
        let color = tone.color();
        if let Some(b) = &self.connect_button {
            let body = NSColor::colorWithSRGBRed_green_blue_alpha(0.36, 0.38, 0.52, 1.);
            if let Some(image) = theme::palette_icon(symbol, 14., &color, &body) {
                b.setImage(Some(&image));
            }
            let _: () = msg_send![&**b, setAccessibilityValue: &*detail];
        }
        if let Some(help) = &self.connect_help {
            help.setStringValue(&NSString::from_str(&text));
            help.setTextColor(Some(&color));
            help.setAlphaValue(1.);
        }
    }
    /// 연결 중에는 헬퍼텍스트를 천천히 깜빡인다.
    unsafe fn pulse_provider_row(&self) {
        if self.connection_job.is_none() {
            return;
        }
        if let Some(help) = &self.connect_help {
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.);
            help.setAlphaValue(0.675 + 0.325 * (t * std::f64::consts::TAU / 1.4).cos());
        }
    }
    /// 분석과 과목 정리는 한 번에 하나만 돈다. 채팅은 실행의 단계 사이에 끼어들 수 있어 여기에 넣지 않는다.
    fn analysis_busy(&self) -> bool {
        self.job.is_some() || self.pending.is_some() || self.analysis_waiting.is_some()
    }
    unsafe fn analyze_document(&mut self, doc: Value) {
        if self.analysis_busy() || self.workspace.as_deref() != doc["workspace_id"].as_str() {
            return;
        }
        if !self.flush_body() || !self.finish_new_title(true) {
            return;
        }
        let current = match rpc(json!({"op":"note.read","id":doc["id"]})) {
            Ok(doc) => doc,
            Err(e) => {
                self.fail(&e);
                return;
            }
        };
        if current["analysis_state"] == "analysed" && !pdf_session::has_pending() {
            self.set_status("현재 파일의 분석이 완료되어 있습니다");
            return;
        }
        if self.provider.is_empty() {
            self.connect();
            if self.provider.is_empty() {
                return;
            }
        }
        if !confirm_action("자료 분석", &format!("‘{}’의 원문과 이미지가 {} 공급자에게 전송됩니다. 필기를 PDF에 저장한 뒤 현재 파일을 분석합니다.", strv(&current,"title"), self.provider), "분석 시작") { return; }
        pdf_session::request_flush_all();
        self.analysis_waiting = Some(current);
        self.set_status("최신 필기 저장 후 분석을 시작합니다…");
        self.refresh_right();
        self.start_waiting_analysis();
    }
    unsafe fn start_waiting_analysis(&mut self) {
        if self.analysis_waiting.is_none() {
            return;
        }
        match pdf_session::flush_state() {
            Ok(false) => return,
            Err(e) => {
                self.analysis_waiting = None;
                self.fail(&format!("PDF 저장 실패로 분석을 시작하지 않았습니다: {e}"));
                self.refresh_right();
                return;
            }
            Ok(true) => {}
        }
        let doc = self.analysis_waiting.take().unwrap();
        match rpc(json!({"op":"note.read","id":doc["id"]})) {
            Ok(current) => {
                self.begin_async(json!({"op":"analysis.begin","workspace_id":current["workspace_id"],"material_id":current["id"],"expected_hash":current["hash"],"provider":self.provider}));
                self.set_status("자료 추출 중… · 필기와 편집을 계속할 수 있습니다");
            }
            Err(e) => self.fail(&e),
        }
        self.refresh_right();
    }
    unsafe fn annotate_pptx(&mut self, doc: Value, force_new: bool) {
        if doc["kind"] != "pptx" || self.workspace.as_deref() != doc["workspace_id"].as_str() {
            return;
        }
        if self.annotation_conversion.is_some() || self.annotation_publish.is_some() {
            self.set_status("PDF를 준비하고 있습니다…");
            return;
        }
        if !force_new {
            match rpc(json!({"op":"pdf.generated.get","source_id":doc["id"]})) {
                Ok(link) if link["document"].is_object() => {
                    self.dispatch_action(Action::Open(link["document"].clone()));
                    return;
                }
                Err(e) => {
                    self.fail(&e);
                    return;
                }
                _ => {}
            }
        } else if !confirm_action(
            "새 PDF로 변환",
            "현재 PPTX로 새 PDF를 만듭니다. 기존 필기 PDF는 그대로 남습니다.",
            "새 PDF 만들기",
        ) {
            return;
        }
        if !pptx_preview::available() {
            self.fail("PPTX 필기에는 설치된 LibreOffice가 필요합니다. PowerPoint에서 PDF로 내보낸 뒤 가져와 주세요.");
            return;
        }
        match rpc(json!({"op":"note.read","id":doc["id"]})) {
            Ok(current) => {
                self.annotation_conversion = Some((
                    pptx_preview::start(&strv(&current, "absolute_path")),
                    current,
                    force_new,
                ));
                self.set_status("필기용 PDF를 준비합니다… · 원본 PPTX는 유지됩니다");
            }
            Err(e) => self.fail(&e),
        }
    }
    unsafe fn poll_annotation_conversion(&mut self) {
        let result = self
            .annotation_conversion
            .as_ref()
            .and_then(|(h, _, _)| h.try_result());
        if let Some(result) = result {
            let (_, doc, force_new) = self.annotation_conversion.take().unwrap();
            match result {
                Ok(path) => {
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send(rpc(json!({"op":"pdf.generated.register","source_id":doc["id"],"source_hash":doc["hash"],"pdf_path":path,"force_new":force_new})));
                    });
                    self.annotation_publish = Some(rx);
                    self.set_status("같은 폴더에 필기용 PDF를 보관합니다…");
                }
                Err(e) => self.fail(&format!("필기용 PDF 생성 실패 · {e}")),
            }
        }
        let result = self
            .annotation_publish
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(value) => Some(value),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("PDF 등록 작업이 종료되었습니다".into()))
                }
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            self.annotation_publish = None;
            match result {
                Ok(doc) => {
                    if !PDF_QUIT_PENDING.with(Cell::get)
                        && self.workspace.as_deref() == doc["workspace_id"].as_str()
                    {
                        self.dispatch_action(Action::Open(doc));
                        self.refresh_right();
                    }
                    self.set_status("필기용 PDF를 만들었습니다 · 분석은 파일 메뉴에서 요청하세요");
                }
                Err(e) => self.fail(&format!("필기용 PDF 보관 실패 · {e}")),
            }
        }
    }
    unsafe fn import(&mut self) {
        if !self.flush() {
            return;
        }
        let Some(ws) = self.workspace.clone() else {
            return;
        };
        let panel = NSOpenPanel::openPanel(mtm());
        panel.setCanChooseFiles(true);
        panel.setCanChooseDirectories(false);
        panel.setAllowsMultipleSelection(false);
        panel.setTitle(Some(&NSString::from_str("보관할 자료 선택")));
        panel.setMessage(Some(&NSString::from_str(
            "PDF, Markdown, PNG, JPEG, HEIC, PPTX · 기존 노트를 선택하면 중복 보관하지 않습니다.",
        )));
        let folderbox = view(rect(0., 0., 440., 54.));
        folderbox.addSubview(&text_label(
            "보관 폴더",
            rect(0., 33., 440., 18.),
            11.,
            true,
        ));
        let folders = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm()),
            rect(0., 0., 440., 28.),
            false,
        );
        folders.setFont(Some(&font::regular(13.)));
        folders.addItemWithTitle(&NSString::from_str("과목 루트"));
        let mut paths = vec![String::new()];
        if let Ok(tree) = rpc(json!({"op":"tree","workspace_id":ws})) {
            if let Some(fs) = tree["folders"].as_array() {
                for f in fs {
                    let p = strv(f, "path");
                    folders.addItemWithTitle(&NSString::from_str(&p));
                    paths.push(p);
                }
            }
        }
        folderbox.addSubview(&folders);
        panel.setAccessoryView(Some(&folderbox));
        panel.setAccessoryViewDisclosed(true);
        if panel.runModal() != 1 {
            return;
        }
        let Some(url) = panel.URL() else {
            return;
        };
        let Some(path) = url.path() else {
            return;
        };
        let folder = paths
            .get(folders.indexOfSelectedItem() as usize)
            .cloned()
            .unwrap_or_default();
        match rpc(
            json!({"op":"material.import","workspace_id":ws,"path":path.to_string(),"folder":folder}),
        ) {
            Ok(d) => {
                let parts: Vec<_> = folder.split('/').filter(|p| !p.is_empty()).collect();
                for i in 1..=parts.len() {
                    self.open_folders.insert(parts[..i].join("/"));
                }
                self.refresh_right();
                self.open_doc(d);
                self.set_status(
                    "자료를 보관했습니다 · 파일을 우클릭하여 분석을 요청할 수 있습니다",
                );
            }
            Err(e) => self.fail(&e),
        }
    }
    unsafe fn open_map(&mut self) {
        if !self.flush() {
            return;
        }
        let Some(ws) = self.workspace.clone() else {
            return;
        };
        match rpc(json!({"op":"graph.get","workspace_id":ws})) {
            Ok(g) => self.graph_data = g,
            Err(e) => {
                self.fail(&e);
                return;
            }
        }
        self.editor_scroll.setHidden(true);
        if let Some(p) = &self.preview {
            p.setHidden(true);
        }
        let map = graph::create(
            rect(
                0.,
                58.,
                self.center.frame().size.width,
                self.center.frame().size.height - 158.,
            ),
            self.graph_data.clone(),
        );
        self.center.addSubview(&map);
        self.map = Some(map);
        self.selection = Value::Null;
        self.unit_focus.clear();
        self.set_status("개념을 선택해 연결된 원본 근거를 확인하세요");
        self.add_map_controls();
        self.last_query.clear();
        self.nodes_open = true;
        self.node_limit = 40;
        self.neighbor_limit = 8;
        if let Some(pref) = self
            .workspace
            .as_ref()
            .and_then(|ws| self.map_preferences.get(ws))
            .cloned()
        {
            self.nodes_open = pref["nodes_open"].as_bool().unwrap_or(true);
            self.node_limit = pref["node_limit"].as_u64().unwrap_or(40) as usize;
            if let Some(q) = pref["query"].as_str() {
                if let Some(f) = &self.search {
                    f.setStringValue(&NSString::from_str(q));
                }
                self.last_query = q.to_string();
                graph::search(q);
            }
            if let Some(id) = pref["selected"].as_str() {
                graph::select(id);
            }
            if let Some(r) = pref["relation"].as_str() {
                graph::filter(r);
                if let Some(p) = &self.relation {
                    if let Some(index) = self.graph_data["relations"]
                        .as_array()
                        .and_then(|rs| rs.iter().position(|x| x["id"] == r))
                    {
                        p.selectItemAtIndex(index as isize + 1);
                    }
                }
            }
            if !pref["view"].is_null() {
                graph::restore_view(&pref["view"]);
            }
        }
        self.selection = graph::selection().unwrap_or(Value::Null);
        graph::take_node_selection(); // Restoring a saved view is not a new selection.
        self.refresh_relation_options();
        self.refresh_right();
        self.update_header();
        graph::focus_input();
    }
    unsafe fn add_map_controls(&mut self) {
        let combo = NSComboBox::initWithFrame(
            NSComboBox::alloc(mtm()),
            rect(24., self.center.frame().size.height - 139., 210., 28.),
        );
        combo.setFont(Some(&font::regular(13.)));
        combo.setCompletes(true);
        combo.setUsesDataSource(false);
        combo.setHasVerticalScroller(true);
        combo.setNumberOfVisibleItems(8);
        if let Some(cs) = self.graph_data["concepts"].as_array() {
            for c in cs {
                combo.addItemWithObjectValue(&NSString::from_str(&strv(c, "name")));
            }
        }
        let search: Retained<NSTextField> = combo.into_super();
        search.setPlaceholderString(Some(&NSString::from_str("개념 검색")));
        search.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
        search.setTarget(Some(&self.target));
        search.setAction(Some(sel!(action:)));
        search.setTag(self.register(Action::Search));
        self.center.addSubview(&search);
        self.search = Some(search);
        let clear = self.button(
            "×",
            rect(238., self.center.frame().size.height - 139., 26., 28.),
            Action::ClearSearch,
        );
        self.center.addSubview(&clear);
        self.search_clear = Some(clear);
        let p = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm()),
            rect(270., self.center.frame().size.height - 139., (self.center.frame().size.width - 294.).min(166.), 28.),
            false,
        );
        p.setFont(Some(&font::regular(13.)));
        p.addItemWithTitle(&NSString::from_str("모든 관계"));
        if let Some(rs) = self.graph_data["relations"].as_array() {
            for r in rs {
                p.addItemWithTitle(&NSString::from_str(&strv(r, "label")));
            }
        }
        p.setTarget(Some(&self.target));
        p.setAction(Some(sel!(action:)));
        p.setTag(self.register(Action::Relation));
        self.center.addSubview(&p);
        self.relation = Some(p);
        self.refresh_relation_options();
    }
    unsafe fn refresh_relation_options(&self) {
        if let Some(p) = &self.relation {
            let old = p.indexOfSelectedItem();
            p.removeAllItems();
            p.addItemWithTitle(&NSString::from_str("모든 관계"));
            let selected = self.selection["concept"]["id"].as_str().unwrap_or("");
            if let Some(rs) = self.graph_data["relations"].as_array() {
                for (i, r) in rs.iter().enumerate() {
                    let count = if selected.is_empty() {
                        self.graph_data["assertions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|a| a["relation"] == r["id"])
                            .count()
                    } else {
                        self.graph_data["assertions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|a| a["relation"] == r["id"])
                            .filter_map(|a| {
                                if a["subject"] == selected {
                                    a["object"].as_str()
                                } else if a["object"] == selected {
                                    a["subject"].as_str()
                                } else {
                                    None
                                }
                            })
                            .collect::<HashSet<_>>()
                            .len()
                    };
                    p.addItemWithTitle(&NSString::from_str(&format!(
                        "{} · {}",
                        strv(r, "label"),
                        count
                    )));

                }
            }
            p.selectItemAtIndex(old.max(0).min(p.numberOfItems() - 1));
        }
    }
    unsafe fn close_map(&mut self) {
        if let Some(ws) = self.workspace.clone() {
            let selected = graph::selection()
                .and_then(|v| v["concept"]["id"].as_str().map(str::to_string))
                .unwrap_or_default();
            let relation = self
                .relation
                .as_ref()
                .map(|p| p.indexOfSelectedItem())
                .filter(|i| *i > 0)
                .and_then(|i| {
                    self.graph_data["relations"]
                        .as_array()
                        .and_then(|rs| rs.get(i as usize - 1))
                })
                .map(|r| strv(r, "id"))
                .unwrap_or_default();
            self.map_preferences.insert(ws,json!({"selected":selected,"relation":relation,"query":self.last_query,"nodes_open":self.nodes_open,"node_limit":self.node_limit,"view":graph::snapshot_view()}));
        }
        if let Some(v) = self.map.take() {
            v.removeFromSuperview();
        }
        graph::dispose();
        if let Some(v) = self.search.take() {
            release_actions(&v, &mut self.actions);
            v.removeFromSuperview();
        }
        if let Some(v) = self.search_clear.take() {
            release_actions(&v, &mut self.actions);
            v.removeFromSuperview();
        }
        if let Some(v) = self.relation.take() {
            release_actions(&v, &mut self.actions);
            v.removeFromSuperview();
        }
        self.editor_scroll.setHidden(self.preview.is_some());
        if let Some(v) = &self.preview {
            v.setHidden(false);
        }
        self.refresh_right();
        self.update_header();
        if self.preview.is_none() {
            self._window.makeFirstResponder(Some(&self.text));
        }
        self.set_status(if self.snapshot {
            "분석 당시 원본 · 읽기 전용"
        } else {
            "이 Mac에 보관됨"
        });
    }
    unsafe fn render_graph_side(&mut self) {
        let h = self.right.frame().size.height;
        let w = self.right.frame().size.width;
        let right = self.right.clone();
        let import = self.glass(
            &right,
            "",
            rect(w - 48., h - 51., 28., 28.),
            Action::Import,
            theme::Glass::Primary,
            Some("import"),
        );
        import.setImageHugsTitle(false);
        import.setAlignment(NSTextAlignment::Center);
        import.setAccessibilityLabel(Some(&NSString::from_str("자료 추가하기")));
        let subject = self.graph_data["layer"] == "subject";
        let mut top = h - 62.;
        let reviewing = self.cleanup_review.is_some();
        let cleanup = self.glass(&right, "", rect(w - 84., h - 51., 28., 28.),
            Action::CleanupReview, theme::Glass::Clear, Some("broom"));
        cleanup.setImageHugsTitle(false);
        cleanup.setAlignment(NSTextAlignment::Center);
        let label = NSString::from_str(if reviewing { "AI 삭제 후보 검토 중" } else { "AI 삭제 후보 검토" });
        cleanup.setAccessibilityLabel(Some(&label));
        cleanup.setEnabled(!reviewing && self.graph_data["concepts"].as_array().is_some_and(|c| !c.is_empty()));
        if self.job.is_some() || self.pending.is_some() {
            top -= 38.;
            let organizing = self.run.as_ref().is_some_and(|r| r["state"] == "running" && r["single"] != true);
            self.glass(
                &right,
                if organizing { "정리 멈춤" } else { "진행 중인 분석 취소" },
                rect(16., top, w - 32., 30.),
                if organizing { Action::OrganizePause } else { Action::Cancel },
                theme::Glass::Clear,
                Some("stop"),
            );
        }
        let area = (top - 16.).max(100.);
        // Rows hang downward from zero and are lifted once the height is known.
        let list = view(rect(0., 0., w - 14., area));
        let inner = w - 32.;
        let mut y = -8.;
        let selected_id = self.selection["concept"]["id"].as_str().unwrap_or("").to_string();
        let mut concepts = graph_controls::sorted(&self.graph_data, &selected_id);
        if !self.unit_focus.is_empty() {
            concepts.retain(|c| c["unit"] == self.unit_focus.as_str());
        }
        // 대화에서 등록한 노트는 내 질문에서 나온 자료로 보여 준다.
        let place_title = |data: &Value, material: &Value| {
            data["sources"]
                .as_array()
                .and_then(|ss| ss.iter().find(|src| &src["material_id"] == material))
                .map(|src| format!("{}{}", if src["from_chat"] == true { "내 질문에서 · " } else { "" }, strv(src, "title")))
                .unwrap_or_default()
        };
        let source_title = |data: &Value, material: &Value| {
            data["sources"]
                .as_array()
                .and_then(|ss| ss.iter().find(|src| &src["material_id"] == material))
                .map(|src| strv(src, "title"))
                .unwrap_or_default()
        };
        if !self.selection.is_null() {
            let selected = self.selection.clone();
            let link = selected["kind"] == "assertion";
            let object = if link {
                selected["assertion"].clone()
            } else if selected["kind"] == "concept" {
                selected["concept"].clone()
            } else {
                selected.clone()
            };
            let sid = strv(&object, "id");
            let concept = self.graph_data["concepts"]
                .as_array()
                .and_then(|cs| cs.iter().find(|c| c["id"] == sid.as_str()))
                .cloned()
                .unwrap_or(Value::Null);
            let inferred = object["basis"] == "inferred";
            let (title, body) = if link {
                let condition = object["condition"].as_str().filter(|s| !s.is_empty());
                if subject {
                    (
                        strv(&object, "sentence"),
                        format!(
                            "{}{}",
                            if inferred {
                                format!("자료에 적혀 있지 않지만 정리하면서 이은 연결이에요.\n이유: {}", strv(&object, "reason"))
                            } else {
                                "자료에 적힌 연결이에요.".to_owned()
                            },
                            condition.map(|c| format!("\n조건: {c}")).unwrap_or_default()
                        ),
                    )
                } else {
                    let relation = self.graph_data["relations"]
                        .as_array()
                        .and_then(|rs| rs.iter().find(|r| r["id"] == object["relation"]));
                    let name = |id: &Value| {
                        self.graph_data["concepts"]
                            .as_array()
                            .and_then(|cs| cs.iter().find(|c| &c["id"] == id))
                            .map(|c| strv(c, "name"))
                            .unwrap_or_else(|| id.as_str().unwrap_or("").into())
                    };
                    (
                        format!("{} → {}", name(&object["subject"]), name(&object["object"])),
                        format!(
                            "{}{}\n{}\n조건: {}",
                            if object["negated"] == true { "부정 · " } else { "" },
                            relation.map(|r| strv(r, "label")).unwrap_or_default(),
                            relation.map(|r| strv(r, "definition")).unwrap_or_default(),
                            condition.unwrap_or("없음")
                        ),
                    )
                }
            } else {
                (
                    object["name"].as_str().or(object["label"].as_str()).unwrap_or("선택한 개념").to_string(),
                    concept["definition"].as_str().or(object["definition"].as_str()).unwrap_or("연결된 원본 근거").to_string(),
                )
            };
            wrapped(&list, &title, 20., &mut y, w - 40., 16., true, 3);
            if subject && !link {
                let unit = self.graph_data["units"]
                    .as_array()
                    .and_then(|us| us.iter().find(|u| u["id"] == concept["unit"]))
                    .map(|u| strv(u, "name"))
                    .unwrap_or_default();
                let meta = [strv(&concept, "kind_label"), unit].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
                if !meta.is_empty() {
                    wrapped(&list, &meta, 20., &mut y, w - 40., 10.5, false, 1).setTextColor(Some(&NSColor::secondaryLabelColor()));
                }
            }
            self.desc_field = None;
            self.desc_hint = None;
            if subject && !link {
                let edited = concept["edited"] == true;
                let field = wrapped(&list, &body, 20., &mut y, w - 40., 11., false, 8);
                field.setEditable(true);
                field.setSelectable(true);
                field.setBezeled(false);
                field.setDrawsBackground(false);
                field.setIdentifier(Some(&NSString::from_str("concept-desc")));
                field.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                if edited {
                    let marker = wrapped(&list, "(수정됨)", 20., &mut y, w - 40., 10.5, false, 1);
                    marker.setTextColor(Some(&NSColor::tertiaryLabelColor()));
                }
                y += 3.;
                y -= 15.;
                let hint = text_label("문장을 클릭해 고쳐 보세요.", rect(20., y, w - 150., 14.), 10.5, false);
                hint.setTextColor(Some(&NSColor::tertiaryLabelColor()));
                list.addSubview(&hint);
                if edited {
                    // 작은 글씨 링크. 올리면 돌아갈 원문이 말풍선으로 보인다.
                    let b = NSButton::buttonWithTitle_target_action(&NSString::from_str("↺ 원래 문장으로"), Some(&self.target), Some(sel!(action:)), mtm());
                    b.setBordered(false);
                    b.setAlignment(NSTextAlignment::Right);
                    b.setFrame(rect(w - 16. - 110., y - 1., 110., 16.));
                    b.setTag(self.register(Action::RestoreDescription(sid.clone())));
                    let title = NSMutableAttributedString::initWithString(NSMutableAttributedString::alloc(), &b.title());
                    let range = NSRange::new(0, title.length());
                    title.addAttribute_value_range(NSForegroundColorAttributeName, &NSColor::colorWithSRGBRed_green_blue_alpha(0.30, 0.27, 0.78, 1.), range);
                    title.addAttribute_value_range(NSFontAttributeName, &font::regular(10.5), range);
                    b.setAttributedTitle(&title);
                    list.addSubview(&b);
                }
                y -= 4.;
                self.desc_field = Some(field);
                self.desc_hint = Some(hint);
                self.desc_concept = sid.clone();
                self.desc_plain = body.clone();
            } else {
                wrapped(&list, &body, 20., &mut y, w - 40., 11., false, 8).setSelectable(true);
            }
            y -= 6.;
            if subject && link && inferred {
                y -= 28.;
                self.glass(&list, "이 연결 숨기기", rect(16., y, inner, 28.), Action::HideLink(object.clone(), true), theme::Glass::Clear, None);
                y -= 8.;
            }
            if concept["low_quality"] == true {
                wrapped(&list, "정리 품질 낮음 · 이 개념이 나온 자료는 원문과 맞지 않는 내용이 많았어요.", 20., &mut y, w - 40., 11., false, 3)
                    .setTextColor(Some(&NSColor::systemOrangeColor()));
                if let Some(material) = concept["materials"][0].as_str() {
                    let title = source_title(&self.graph_data, &concept["materials"][0]);
                    y -= 28.;
                    self.glass(&list, "꼼꼼하게 다시 정리", rect(16., y, inner, 28.), Action::Reread(material.to_owned(), title), theme::Glass::Clear, Some("sparkle"));
                    y -= 8.;
                }
            }
            if subject && !link {
                let touches = |l: &&Value| l["subject"] == sid.as_str() || l["object"] == sid.as_str();
                let links: Vec<Value> = self.graph_data["assertions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(touches)
                    .cloned()
                    .collect();
                let hidden: Vec<Value> = self.graph_data["hidden"].as_array().into_iter().flatten().filter(touches).cloned().collect();
                if !links.is_empty() {
                    list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                    y -= 42.;
                    self.section(&list, "link", &format!("연결 {}개", links.len()), 16., y, inner, 11.);
                    y -= 6.;
                    for l in links.iter().take(self.neighbor_limit) {
                        let other = if l["subject"] == sid.as_str() { strv(l, "object") } else { strv(l, "subject") };
                        // 같은 두 개념이 조건만 달리 여러 번 이어질 수 있어 조건을 함께 보여 준다.
                        let sentence = match l["condition"].as_str().filter(|c| !c.is_empty()) {
                            Some(condition) => format!("{} (조건: {condition})", strv(l, "sentence")),
                            None => strv(l, "sentence"),
                        };
                        let dashed = l["basis"] == "inferred";
                        let id = strv(l, "id");
                        let expanded = self.expanded_link == id;
                        let height = if expanded {
                            let measure = text_label(&sentence, rect(0.,0.,inner-49.,28.), 12.5, false);
                            measure.setMaximumNumberOfLines(0);
                            measure.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                            measure.cell().map(|c| c.cellSizeForBounds(rect(0.,0.,inner-49.,100000.)).height.ceil()+10.).unwrap_or(28.).max(28.)
                        } else {28.};
                        y -= height;
                        let b = self.glass(&list, &sentence, rect(16., y, inner, height), Action::Node(other), theme::Glass::Clear, Some(if dashed { "link" } else { "orb" }));
                        b.setAlignment(NSTextAlignment::Left);
                        if expanded {
                            b.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                            if let Some(cell) = b.cell() {cell.setWraps(true);}
                        } else {
                            if b.intrinsicContentSize().width > b.frame().size.width {
                                self.actions[b.tag() as usize] = Some(Action::ExpandLink(id));
                            }
                            b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                        }
                        let menu = NSMenu::new(mtm());
                        let item = NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm()), &NSString::from_str("이 연결 영구 삭제…"),Some(sel!(action:)), &NSString::from_str(""));
                        item.setTarget(Some(&self.target));
                        item.setTag(self.register(Action::DeleteLink(l.clone())));
                        menu.addItem(&item);
                        b.setMenu(Some(&menu));
                        y -= 3.;
                        if dashed {
                            wrapped(&list, &format!("정리하며 이음 · {}", strv(l, "reason")), 30., &mut y, w - 50., 10.5, false, 3)
                                .setTextColor(Some(&NSColor::secondaryLabelColor()));
                        }
                    }
                    if links.len() > 8 {
                        y -= 28.;
                        self.glass(
                            &list,
                            if self.neighbor_limit == 8 { "연결 더 보기" } else { "연결 접기" },
                            rect(16., y, inner, 28.),
                            Action::MoreNeighbors,
                            theme::Glass::Clear,
                            None,
                        );
                        y -= 6.;
                    }
                }
                if !hidden.is_empty() {
                    list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                    y -= 42.;
                    self.section(&list, "deselect", &format!("숨긴 연결 {}개", hidden.len()), 16., y, inner, 11.);
                    y -= 6.;
                    for l in &hidden {
                        wrapped(&list, &strv(l, "sentence"), 20., &mut y, w - 40., 11., false, 2).setTextColor(Some(&NSColor::secondaryLabelColor()));
                        y -= 24.;
                        self.glass(&list, "다시 보이기", rect(w - 16. - 110., y, 110., 24.), Action::HideLink(l.clone(), false), theme::Glass::Clear, None);
                        y -= 8.;
                    }
                }
                let merged = concept["merged"].as_array().cloned().unwrap_or_default();
                if !merged.is_empty() {
                    list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                    y -= 42.;
                    self.section(&list, "tag", "하나로 합친 이름", 16., y, inner, 11.);
                    y -= 6.;
                    for m in &merged {
                        wrapped(&list, &strv(m, "name"), 20., &mut y, w - 40., 11.5, true, 1);
                        wrapped(&list, &strv(m, "definition"), 20., &mut y, w - 40., 10.5, false, 2).setTextColor(Some(&NSColor::secondaryLabelColor()));
                        y -= 24.;
                        let b = self.glass(&list, "따로 떼기", rect(w - 16. - 110., y, 110., 24.), Action::Unmerge(strv(m, "id")), theme::Glass::Clear, None);
                        y -= 8.;
                    }
                }
            } else if !link {
                let neighbors = graph_controls::neighbors(&self.graph_data, &sid);
                if !neighbors.is_empty() {
                    list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                    y -= 42.;
                    self.section(&list, "link", "연결된 개념", 16., y, inner, 11.);
                    y -= 6.;
                    for n in neighbors.iter().take(self.neighbor_limit) {
                        y -= 28.;
                        let b = self.glass(&list, &strv(n, "name"), rect(16., y, inner, 28.), Action::Node(strv(n, "id")), theme::Glass::Clear, Some("orb"));
                        b.setAlignment(NSTextAlignment::Left);
                        b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                        y -= 3.;
                    }
                    if neighbors.len() > 8 {
                        y -= 28.;
                        self.glass(
                            &list,
                            if self.neighbor_limit == 8 { "연결된 개념 더 보기" } else { "연결된 개념 접기" },
                            rect(16., y, inner, 28.),
                            Action::MoreNeighbors,
                            theme::Glass::Clear,
                            None,
                        );
                        y -= 6.;
                    }
                }
            }
            // 배운 곳: 과목 층의 개념은 자료별 위치를, 그 밖에는 연결의 근거를 보여 준다.
            let mut places: Vec<Value> = if subject && !link {
                concept["learned"].as_array().cloned().unwrap_or_default()
            } else if subject {
                object["evidence"].as_array().cloned().unwrap_or_default()
            } else {
                self.graph_data["assertions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|a| [&a["subject"], &a["object"], &a["id"], &a["relation"]].iter().any(|v| v.as_str() == Some(sid.as_str())))
                    .flat_map(|a| a["evidence"].as_array().cloned().unwrap_or_default())
                    .collect()
            };
            // 수업 자료에서 배운 곳이 먼저, 내 질문에서 나온 노트가 뒤에 온다.
            let from_chat = |e: &Value| self.graph_data["sources"].as_array().is_some_and(|ss| ss.iter().any(|src| src["material_id"] == e["material_id"] && src["from_chat"] == true));
            places.sort_by_key(from_chat);
            if !places.is_empty() {
                list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                y -= 42.;
                self.section(&list, "quote", &format!("배운 곳 {}곳", places.len()), 16., y, inner, 11.);
                y -= 6.;
            }
            let shown = 12;
            let more = places.len().saturating_sub(shown);
            for e in places.into_iter().take(shown) {
                let loc = &e["locator"];
                let title = format!(
                    "{} · {} {}{}",
                    place_title(&self.graph_data, &e["material_id"]),
                    loc["kind"].as_str().unwrap_or("원본"),
                    loc["index"],
                    if e["stale"] == true { " · 구버전" } else { "" }
                );
                let quote = e["quote"].as_str().unwrap_or("이미지 영역 근거").to_string();
                y -= 28.;
                let b = self.glass(&list, &title, rect(16., y, inner, 28.), Action::Evidence(e), theme::Glass::Clear, Some("quote"));
                b.setAlignment(NSTextAlignment::Left);
                b.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
                y -= 3.;
                wrapped(&list, &quote, 20., &mut y, w - 44., 11., false, 4).setSelectable(true);
                y -= 4.;
            }
            if more > 0 {
                wrapped(&list, &format!("외 {more}곳"), 20., &mut y, w - 40., 10.5, false, 1).setTextColor(Some(&NSColor::secondaryLabelColor()));
            }
            y -= 10.;
        }
        if subject {
            let units = self.graph_data["units"].as_array().cloned().unwrap_or_default();
            if !units.is_empty() {
                list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
                y -= 42.;
                self.section(&list, "list", &format!("단원 {}개", units.len()), 16., y, inner, 11.);
                y -= 6.;
                for u in &units {
                    let id = strv(u, "id");
                    let focused = self.unit_focus == id;
                    y -= 28.;
                    let b = self.glass(
                        &list,
                        &format!("{} · 개념 {}", strv(u, "name"), u["concepts"].as_u64().unwrap_or(0)),
                        rect(16., y, inner, 28.),
                        Action::Unit(id),
                        if focused { theme::Glass::Selected } else { theme::Glass::Clear },
                        Some("orb"),
                    );
                    let [r, g, bl] = graph::unit_color(u["position"].as_u64().unwrap_or(0) as usize);
                    b.setContentTintColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(r, g, bl, 1.)));
                    b.setAlignment(NSTextAlignment::Left);
                    b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
                    if focused {
                        let _: () = msg_send![&b, setAccessibilityValue: &*NSString::from_str("선택됨")];
                    }
                    y -= 3.;
                }
                y -= 8.;
            }
        }
        list.addSubview(&theme::soft_divider(rect(14., y - 10., w - 28., 1.), true));
        y -= 42.;
        self.section(&list, "tag", &format!("개념 {}개", concepts.len()), 16., y, inner, 11.);
        y -= 6.;
        if concepts.is_empty() {
            wrapped(&list, "자료를 추가하면\n개념과 관계가 여기에 펼쳐집니다.", 20., &mut y, w - 40., 12., false, 3);
        }
        for c in concepts.iter().take(self.node_limit) {
            let selected = strv(c, "id") == selected_id;
            let label = format!(
                "{} · 연결 {}",
                strv(c, "name"),
                graph_controls::degree(&self.graph_data, &strv(c, "id"))
            );
            y -= 28.;
            let b = if selected {
                // 선택한 개념은 옅은 인디고 유리 판으로 강조한다.
                self.glass(&list, &label, rect(16., y, inner, 28.), Action::Node(strv(c, "id")), theme::Glass::Selected, Some("orb"))
            } else {
                let b = self.button(&label, rect(16., y, inner, 28.), Action::Node(strv(c, "id")));
                b.setBordered(false);
                theme::set_icon(&b, Some("orb"), 8.);
                list.addSubview(&b);
                b
            };
            b.setAlignment(NSTextAlignment::Left);
            b.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            if selected {
                let _: () = msg_send![&b, setAccessibilityValue: &*NSString::from_str("선택됨")];
            }
            y -= 2.;
        }
        if concepts.len() > self.node_limit {
            y -= 28.;
            self.glass(&list, "노드 더 보기", rect(16., y, inner, 28.), Action::MoreNodes, theme::Glass::Clear, None);
        }
        let ch = (16. - y).max(area);
        list.setFrame(rect(0., 0., w - 14., ch));
        for child in list.subviews().iter() {
            let mut frame = child.frame();
            frame.origin.y += ch;
            child.setFrame(frame);
        }
        let scroll =
            NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., 10., w, area));
        scroll.setHasVerticalScroller(true);
        crate::scrollbar::style(&scroll);
        scroll.setAutohidesScrollers(true);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&list));
        list.scrollPoint(NSPoint::new(0., ch));
        self.right.addSubview(&scroll);
    }
    unsafe fn evidence(&mut self, e: Value) {
        if !self.flush() {
            return;
        }
        let id = strv(&e, "material_id");
        let source = match rpc(json!({"op":"source.get","id":id})) {
            Ok(s) => s,
            Err(err) => {
                self.fail(&err);
                return;
            }
        };
        let path = e["immutable_path"]
            .as_str()
            .or(source["immutable_path"].as_str())
            .unwrap_or("")
            .to_string();
        if self.map.is_some() {
            self.close_map();
        }
        self.hide_preview();
        self.document = Some(source.clone());
        self.snapshot = true;
        self.current_evidence = Some(e.clone());
        self.dirty = false;
        let loc = &e["locator"];
        self.preview_locator = Some(loc.clone());
        if loc["kind"] == "paragraph" {
            match std::fs::read_to_string(&path) {
                Ok(body) => {
                    self.text.setString(&NSString::from_str(&body));
                    self.clear_undo();
                    self.last_text = body;
                    self.text.setEditable(false);
                    editor::style_with_base(
                        &self.text,
                        self.document.as_ref().and_then(document_base),
                    );
                    if let Some(q) = e["quote"].as_str() {
                        if !editor::locate_paragraph(
                            &self.text,
                            q,
                            loc["index"].as_u64().unwrap_or(1) as usize,
                        ) {
                            self.set_status("근거의 원문 위치를 찾을 수 없습니다");
                        }
                    }
                }
                Err(err) => {
                    self.fail(&format!("분석 원본을 읽을 수 없습니다: {err}"));
                    return;
                }
            }
        } else if path.to_lowercase().ends_with(".pptx") && pptx_preview::available() {
            self.start_pptx_preview(&path);
        } else {
            match editor::preview(
                &path,
                self.editor_scroll.frame(),
                if loc["kind"] == "page" {
                    loc["index"].as_u64().map(|v| v as usize)
                } else {
                    None
                },
            ) {
                Ok(p) => {
                    self.editor_scroll.setHidden(true);
                    self.center.addSubview(&p);
                    self.preview = Some(p);
                }
                Err(err) => self.fail(&err),
            }
        }
        self.update_header();
        self.set_status(&format!(
            "{} {} · {}",
            loc["kind"].as_str().unwrap_or("원본"),
            loc["index"],
            if e["stale"] == true {
                "구버전 근거 · 재분석 필요"
            } else if loc["kind"] == "slide" {
                "슬라이드 번호 · 원본에서 확인"
            } else {
                "분석 당시 원본"
            }
        ));
    }
    unsafe fn action(&mut self, tag: isize) {
        let Some(a) = self.actions.get(tag as usize).cloned().flatten() else {
            return;
        };
        self.dispatch_action(a);
    }
    unsafe fn dispatch_action(&mut self, a: Action) {
        // Drain any reentrant chat input before workspace/modal actions change
        // its context. A queued send must retain the source conversation.
        let chat_ws=self.workspace.clone();
        if self.chat.tick(chat_ws.as_deref()){self.connect();}
        if matches!(
            &a,
            Action::Open(_)
                | Action::Workspace(_)
                | Action::NewWorkspace
                | Action::DeleteWorkspace(..)
                | Action::DeleteDocument(_)
                | Action::Delete
                | Action::ToggleMap
                | Action::Suggest("edit", _)
        ) && pdf_session::has_pending()
        {
            pdf_session::request_flush_all();
            match pdf_session::flush_state() {
                Ok(false) => {
                    self.deferred_action = Some(a);
                    self.set_status("PDF 저장 후 요청한 작업을 계속합니다…");
                    return;
                }
                Err(e) => {
                    self.fail(&e);
                    return;
                }
                Ok(true) => {}
            }
        }
        let return_graph_focus = matches!(
            &a,
            Action::Node(_)
                | Action::Fit
                | Action::Focus
                | Action::ZoomIn
                | Action::ZoomOut
                | Action::Rotate
                | Action::Deselect
                | Action::Relation
                | Action::ClearSearch
                | Action::ToggleNodes
                | Action::MoreNodes
                | Action::MoreNeighbors
                | Action::Import
                | Action::Cancel
                | Action::Connect
        );
        match a {
            Action::ToggleShortcuts | Action::CloseShortcuts => {
                if let Some(panel) = &self.shortcuts {
                    panel.setHidden(matches!(a, Action::CloseShortcuts) || !panel.isHidden());
                    self.layout_frames(self.root.bounds().size);
                }
            }
            Action::ToggleSidebar(side) => self.toggle_sidebar(side),
            Action::RightMode(chat) => {
                self.chat_mode = chat;
                for i in 0..2 { theme::chat_tab(&self.right_tabs[i], chat == (i == 1), i); }
                self.layout_right_tabs(self.right_shell.frame().size.width, self.right_shell.frame().size.height);
                self.right.setHidden(chat);
                self.chat.view.setHidden(!chat);
                if let Err(e) = rpc(json!({"op":"chat.preferences","mode":if chat {"chat"} else {"materials"}})) { self.fail(&e); }
                self.suggest_open = false;
                self.render_suggest_pop();
                self.poll_suggestions();
                self._window.makeFirstResponder(Some(&self.text));
            },
            Action::NewWorkspace => {
                if !self.flush() {
                    return;
                }
                if let Some(name) = input(
                    "새 워크스페이스",
                    "과목 이름으로 자료와 지식지도를 나눕니다.",
                    "", "만들기",
                ) {
                    match rpc(json!({"op":"workspace.create","name":name})) {
                        Ok(w) => {
                            self.remember_screen();
                            if self.map.is_some() {
                                self.close_map();
                            }
                            let _ = document_tree::drain();
                            self.workspace = Some(strv(&w, "id"));
                            self.open_folders.clear();
                            self.document_tree = None;
                            self.restore_screen();
                            self.refresh();
                        }
                        Err(e) => self.fail(&e),
                    }
                }
            }
            Action::RenameWorkspace(id, old) => {
                if self.workspace_rename.is_some() { return; }
                if let Some((_, row, button)) = self.workspace_rows.iter().find(|(key, _, _)| key == &id) {
                    button.setTitle(&NSString::from_str(""));
                    let field = NSTextField::initWithFrame(NSTextField::alloc(mtm()), rect(38., 9., (row.bounds().size.width - 46.).max(30.), 22.));
                    field.setStringValue(&NSString::from_str(&old));
                    field.setIdentifier(Some(&NSUserInterfaceItemIdentifier::from_str("workspace-rename")));
                    field.setDelegate(Some(ProtocolObject::from_ref(&*self.target)));
                    row.addSubview(&field);
                    self._window.makeFirstResponder(Some(&field));
                    let _: () = msg_send![&field, selectText: std::ptr::null::<AnyObject>()];
                    self.workspace_rename = Some((id, old, field));
                }
            }
            Action::DeleteWorkspace(id, name) => {
                let active = self.workspace.as_ref() == Some(&id);
                if !confirm_action(&format!("‘{name}’ 워크스페이스 삭제"), "자료, 초안, 대화, 지식지도 등 이 워크스페이스의 관련 데이터가 모두 영구 삭제됩니다.", "확인")
                    || !confirm_action(&format!("‘{name}’ 워크스페이스를 정말 삭제할까요?"), "삭제한 데이터는 복구할 수 없습니다.", "영구 삭제") { return; }
                if self.chat.busy() {
                    self.fail("진행 중인 대화가 끝난 후 삭제하세요.");
                    return;
                }
                if active && !self.flush() { return; }
                match rpc(json!({"op":"workspace.delete","workspace_id":id})) {
                    Ok(_) => {
                        self.screens.remove(&id);
                        if active {
                            self.chat.forget_workspace();
                            if self.map.is_some() { self.close_map(); }
                            organize_panel::close();
                            let _ = document_tree::drain();
                            self.workspace = rpc(json!({"op":"workspace.list"})).ok().and_then(|r| r["workspaces"][0]["id"].as_str().map(str::to_owned));
                            self.suggest_open = false;
                            self.open_folders.clear();
                            self.document_tree = None;
                            self.restore_screen();
                        }
                        self.refresh();
                    }
                    Err(e) => self.fail(&e),
                }
            }
            Action::Workspace(id) => {
                if self.workspace.as_ref() == Some(&id) || !self.flush() { return; }
                self.remember_screen();
                if self.map.is_some() { self.close_map(); }
                let _ = document_tree::drain();
                organize_panel::close();
                self.suggest_open = false;
                self.workspace = Some(id);
                self.open_folders.clear();
                self.document_tree = None;
                self.restore_screen();
                self.refresh_left_with_motion(true);
                self.refresh_right();
                self.update_header();
            }
            Action::AiSettings => ai_settings::open(),
            Action::Organize => {
                self.suggest_open = false;
                self.render_suggest_pop();
                self.open_organize(None);
            },
            Action::Reread(id, title) => self.open_organize(Some((id, title))),
            Action::Suggest(kind, id) => {
                let active = self.editing_suggestion().is_some_and(|doc| doc["id"] == id.as_str());
                let result = match kind {
                    "ask_dismiss" | "cancel" => {
                        self.suggest_confirm = (kind == "ask_dismiss").then_some(id);
                        self.render_suggest_pop();
                        return;
                    }
                    "edit" => {
                        if !active {
                            let Some(draft) = self.suggestions["suggestions"].as_array().and_then(|all| {
                                all.iter().find(|item| item["id"] == id.as_str() && item["state"] == "draft")
                            }) else { return };
                            let doc = json!({"kind":"suggestion","id":id,"title":draft["title"],"body":draft["body"]});
                            if !self.open_doc(doc) {
                                return;
                            }
                            self.text.setSelectedRange(NSRange::new(0, 0));
                            self.text.scrollRangeToVisible(NSRange::new(0, 0));
                            self.set_status("정리 제안 초안");
                        }
                        self.suggest_open = false;
                        self.suggest_confirm = None;
                        self.refresh_right();
                        self._window.makeFirstResponder(Some(&self.text));
                        return;
                    }
                    "register" => {
                        if active && !self.flush() {
                            return;
                        }
                        rpc(json!({"op":"distill.register","id":id})).map(|v| {
                            let document = v["document"].clone();
                            let title = strv(&document, "title");
                            if active {
                                self.suggest_open = false;
                                self.document = Some(document.clone());
                                self.update_header();
                                self.open_doc(document);
                            }
                            format!("‘{title}’ 노트를 학습도우미 정리 폴더에 등록했어요 · 다음 과목 정리 때 지도에 들어가요")
                        })
                    }
                    "dismiss" => rpc(json!({"op":"distill.dismiss","id":id})).map(|_| "정리 제안을 삭제했어요".to_owned()),
                    _ => rpc(json!({"op":"distill.draft","id":id})).map(|_| "노트 초안을 만드는 중이에요 · 끝나면 정리 제안에 초안이 생겨요".to_owned()),
                };
                match result {
                    Ok(message) => {
                        if active && kind == "dismiss" {
                            self.finish_new_title(false);
                            self.show_blank();
                        }
                        self.set_status(&message);
                        self.suggest_confirm = None;
                        self.suggestions = Value::Null;
                        self.poll_suggestions();
                        self.chat.sync_notes();
                    }
                    Err(e) => self.fail(&e),
                }
            }
            Action::SuggestToggle => {
                self.suggest_open = !self.suggest_open;
                self.suggest_confirm = None;
                // 열 때 최신 제안을 읽는다. 본 것으로는 창을 닫을 때 표시한다.
                self.poll_suggestions();
                self.refresh_right();
            }
            Action::SuggestClose => {
                self.suggest_open = false;
                self.suggest_confirm = None;
                self.refresh_right();
            }
            Action::SuggestIntroSeen => match rpc(json!({"op":"distill.intro"})) {
                Ok(_) => {
                    self.suggestions["intro_seen"] = json!(true);
                    self.render_suggest_pop();
                }
                Err(e) => self.fail(&e),
            },
            Action::OrganizePause => {
                if let Some(ws) = self.workspace.clone() {
                    match rpc(json!({"op":"organize.pause","workspace_id":ws})) {
                        Ok(_) => {
                            self.job = None;
                            self.load_run();
                            self.set_status("멈췄어요 · 읽은 부분은 보관해 두었어요");
                        }
                        Err(e) => self.fail(&e),
                    }
                    self.refresh_right();
                }
            }
            Action::OrganizeResume => {
                if let Some(ws) = self.workspace.clone() {
                    match rpc(json!({"op":"organize.resume","workspace_id":ws})) {
                        Ok(job) => {
                            self.job = job["id"].as_str().map(str::to_string);
                            self.load_run();
                            self.set_status("이어서 정리하는 중 · 필기와 채팅을 계속할 수 있어요");
                        }
                        Err(e) => self.fail(&e),
                    }
                    self.refresh_right();
                }
            }
            Action::OrganizeDismiss => {
                if let Some(ws) = self.workspace.clone() {
                    let _ = rpc(json!({"op":"organize.dismiss","workspace_id":ws}));
                    self.load_run();
                    self.refresh_right();
                }
            }
            Action::OrganizeHow => {
                let lines: Vec<String> = self
                    .run
                    .as_ref()
                    .and_then(|r| r["how"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|l| l.as_str().map(str::to_owned))
                    .collect();
                alert("어떻게 정리했나요", &lines.join("\n"));
            }
            Action::Unit(id) => {
                self.unit_focus = if self.unit_focus == id { String::new() } else { id };
                graph::focus_unit(&self.unit_focus);
                self.selection = graph::selection().unwrap_or(Value::Null);
                self.refresh_right();
            }
            Action::ExpandLink(id) => {
                let offset = find_scroll(&self.right).map(|s| s.documentView().unwrap().frame().size.height - s.contentView().bounds().origin.y);
                self.expanded_link = id;
                self.refresh_right();
                if let (Some(offset),Some(scroll)) = (offset,find_scroll(&self.right)) {
                    if let Some(view) = scroll.documentView() {view.scrollPoint(NSPoint::new(0.,view.frame().size.height-offset));}
                }
            }
            Action::CleanupReview => self.start_cleanup(),
            Action::DeleteLink(link) => {
                let mut message = strv(&link,"sentence");
                if let Some(condition) = link["condition"].as_str().filter(|c| !c.is_empty()) {message.push_str(&format!(" (조건: {condition})"));}
                message.push_str("\n\n이 연결과 전용 근거를 삭제합니다. 복구할 수 없습니다.");
                if confirm_action("이 연결을 영구 삭제할까요?", &message, "영구 삭제") {
                    match rpc(json!({"op":"ontology.link.delete","workspace_id":self.workspace,"link":link})) {
                        Ok(_) => { self.reload_graph(); self.refresh_right(); self.set_status("연결을 영구 삭제했어요"); }
                        Err(e) => self.fail(&e),
                    }
                }
            }
            Action::HideLink(link, hide) => {
                if let Some(ws) = self.workspace.clone() {
                    match rpc(json!({"op":"ontology.link.hide","workspace_id":ws,"subject":link["subject"],"object":link["object"],"kind":link["relation"],"hidden":hide})) {
                        Ok(_) => {
                            self.reload_graph();
                            self.set_status(if hide { "연결을 숨겼어요 · 개념 카드에서 다시 보이게 할 수 있어요" } else { "연결을 다시 보이게 했어요" });
                        }
                        Err(e) => self.fail(&e),
                    }
                    self.refresh_right();
                }
            }
            Action::RestoreDescription(id) => {
                self.save_description(&id, "");
            }
            Action::Unmerge(member) => {
                if let Some(ws) = self.workspace.clone() {
                    match rpc(json!({"op":"ontology.unmerge","workspace_id":ws,"member_id":member})) {
                        Ok(_) => {
                            self.reload_graph();
                            self.set_status("따로 떼어 놓았어요 · 다음 정리에서도 합치지 않아요");
                        }
                        Err(e) => self.fail(&e),
                    }
                    self.refresh_right();
                }
            }
            Action::Save if self.diagram.is_some()=>self.finish_diagram(),
            Action::Delete if self.diagram.is_some()=>{self.discard_diagram();},
            Action::Save => {
                if let Some(doc) = self.editing_suggestion().filter(|_| self.map.is_none()) {
                    self.dispatch_action(Action::Suggest("register", strv(doc, "id")));
                } else {
                    self.save();
                }
            }
            Action::DeleteDocument(doc) => self.delete_document(doc),
            Action::Delete => self.delete(),
            Action::Open(d) => {
                self.open_doc(d);
                self.refresh_right();
            }
            Action::ToggleMap => {
                if self.map.is_some() {
                    self.close_map();
                } else {
                    self.open_map();
                }
            }
            Action::Connect => self.connect(),
            Action::Import => self.import(),
            Action::Cancel => {
                if let Some(p) = self.pending.take() {
                    p.cancel();
                    self.set_status("자료 추출 취소됨 · 원본 보관됨");
                    self.refresh_right();
                }
                if let Some(job) = self.job.clone() {
                    match rpc(json!({"op":"analysis.cancel","id":job})) {
                        Ok(_) => {
                            self.job = None;
                            self.refresh_right();
                            self.set_status("분석 취소됨 · 원본과 기존 지도는 보존됩니다");
                            self.refresh_right();
                        }
                        Err(e) => self.fail(&e),
                    }
                }
            }
            Action::Fit => {
                graph::select("");
                graph::fit();
                self.selection = Value::Null;
                self.nodes_open = false;
                self.refresh_right();
            }
            Action::Focus => graph::focus(),
            // Same steps as the + and − keys.
            Action::ZoomIn => graph::zoom(1. / 0.86),
            Action::ZoomOut => graph::zoom(1. / 1.16),
            Action::Rotate => graph::rotate(0.25),
            Action::Deselect => {
                graph::select("");
                self.selection = Value::Null;
                self.refresh_right();
            }
            Action::Node(id) => {
                graph::select(&id);
                graph::focus();
            }
            Action::Search => {
                if let Some(field) = &self.search {
                    let query = field.stringValue().to_string();
                    graph::search(&query);
                    let matches = graph_controls::matches(&self.graph_data, &query);
                    if let Some(c) = matches.first() {
                        graph::select(&strv(c, "id"));
                        graph::focus();
                        graph::focus_input();
                    } else {
                        self.set_status("검색 결과가 없습니다");
                    }
                }
            }
            Action::ClearSearch => {
                if let Some(s) = &self.search {
                    s.setStringValue(&NSString::from_str(""));
                }
                self.last_query.clear();
                graph::search("");
                self.set_status("개념 검색을 지웠습니다");
            }
            Action::ToggleNodes => {
                self.nodes_open = !self.nodes_open;
                self.refresh_right();
            }
            Action::MoreNodes => {
                self.node_limit += 40;
                self.refresh_right();
            }
            Action::MoreNeighbors => {
                self.neighbor_limit = if self.neighbor_limit == 8 {
                    usize::MAX.min(
                        self.graph_data["concepts"]
                            .as_array()
                            .map(Vec::len)
                            .unwrap_or(8),
                    )
                } else {
                    8
                };
                self.refresh_right();
            }
            Action::Relation => {
                if let Some(p) = &self.relation {
                    let i = p.indexOfSelectedItem();
                    if i == 0 {
                        graph::filter("");
                    } else {
                        let r = self.graph_data["relations"]
                            .as_array()
                            .and_then(|a| a.get((i - 1) as usize))
                            .map(|r| strv(r, "id"))
                            .unwrap_or_default();
                        graph::filter(&r);
                    }
                }
            }
            Action::Evidence(e) => self.evidence(e),
        }
        if return_graph_focus && self.map.is_some() {
            graph::focus_input();
        }
    }
    unsafe fn tick(&mut self) {
        self.update_ai_spinner();
        self.poll_cleanup();
        let ws = self.workspace.clone();
        self.chat.connection(self.provider_info.as_ref());
        if self.chat.tick(ws.as_deref()) { self.connect(); }
        ai_settings::tick();
        organize_panel::tick();
        if self.chat.take_notes_changed() {
            // 그대로 저장한 노트가 자료 목록에 바로 보이도록 목록도 다시 그린다.
            self.suggestions_for = None;
        }
        if self.suggestions_for != self.workspace || self.last_suggestion_poll.elapsed() > Duration::from_secs(5) {
            self.poll_suggestions();
        }
        while let Some(organize_panel::Event::Started(job)) = organize_panel::take_event() {
            self.job = job["id"].as_str().map(str::to_string);
            self.last_job_poll = Instant::now();
            self.load_run();
            self.set_status("과목 정리를 시작했어요 · 필기와 채팅을 계속할 수 있어요");
            self.refresh_right();
        }
        self.poll_pdf_sessions();
        self.start_waiting_analysis();
        self.poll_pdf_share();
        self.poll_annotation_conversion();
        if let Some(action) = self.deferred_action.take() {
            self.dispatch_action(action);
        }
        self.poll_description();
        let workspace_command = WORKSPACE_COMMAND.with(|c| c.replace(0));
        if self.workspace_rename.as_ref().is_some_and(|(_, _, field)| workspace_command != 0 || field.currentEditor().is_none()) {
            self.finish_workspace_rename(workspace_command != -1, true);
        }
        let title_command = TITLE_COMMAND.with(|c| c.replace(0));
        if self.editing_new_title {
            if title_command != 0 || self.header.currentEditor().is_none() {
                let finished = if title_command == -1 || self.diagram.is_some() {
                    self.finish_new_title(title_command != -1)
                } else {
                    self.flush()
                };
                if !finished { return; }
                if title_command != 0 {
                    if let Some(canvas) = &self.diagram {
                        canvas.focus();
                    } else {
                        self._window.makeFirstResponder(Some(&self.text));
                    }
                }
            }
        }
        self.handle_tree_events();
        self.poll_provider_probe();
        self.pulse_provider_row();
        if self.tree_refresh_pending {
            self.refresh_right();
        }
        let preview_result = self
            .preview_pending
            .as_ref()
            .and_then(|(p, _)| p.try_result());
        if let Some(result) = preview_result {
            let (_, original) = self.preview_pending.take().unwrap();
            if let Some(old) = self.preview.take() {
                editor::close_preview(&old);
                old.removeFromSuperview();
            }
            let (path, rendered, message) = match result {
                Ok(pdf) => (
                    pdf,
                    true,
                    "PPTX 미리보기 · 설치된 LibreOffice로 렌더링".to_string(),
                ),
                Err(e) => (
                    original,
                    false,
                    format!("PPTX 렌더링 실패: {e} · 시스템 미리보기 사용, PDF 입력 권장"),
                ),
            };
            match editor::preview(&path, self.editor_scroll.frame(), None) {
                Ok(p) => {
                    p.setHidden(self.map.is_some());
                    self.center.addSubview(&p);
                    self.preview = Some(p);
                    self.preview_rendered = rendered;
                }
                Err(e) => {
                    let p = view(self.editor_scroll.frame());
                    let label = text_label(
                        &format!(
                            "PPTX 미리보기를 표시할 수 없습니다.\n{e}\nPDF로 변환해 가져오세요."
                        ),
                        rect(
                            20.,
                            p.frame().size.height - 150.,
                            p.frame().size.width - 40.,
                            120.,
                        ),
                        14.,
                        false,
                    );
                    label.setMaximumNumberOfLines(8);
                    label.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
                    p.addSubview(&label);
                    p.setHidden(self.map.is_some());
                    self.center.addSubview(&p);
                    self.preview = Some(p);
                }
            }
            self.update_header();
            if self.map.is_none() {
                if let Some(loc) = self
                    .preview_locator
                    .as_ref()
                    .filter(|l| l["kind"] == "slide")
                {
                    self.set_status(&format!("근거 슬라이드 {} · {}", loc["index"], message));
                } else {
                    self.set_status(&message);
                }
            }
        }
        let pending_result = self
            .pending
            .as_ref()
            .and_then(|p| p.receiver.try_recv().ok());
        if let Some(result) = pending_result {
            self.pending = None;
            match result {
                Ok(j) => {
                    self.job = j["id"].as_str().map(str::to_string);
                    self.set_status("분석 중…");
                    self.refresh_right();
                }
                Err(e) => {
                    self.fail(&format!("원본은 보관되었습니다. 분석 시작 실패: {e}"));
                    self.refresh_right();
                }
            }
        }
        if self.map.is_none() {
            if let Some(field) = &self.file_search {
                let composing = field.currentEditor().is_some_and(|editor| {
                    let marked: bool = msg_send![&editor, hasMarkedText];
                    marked
                });
                let query = field.stringValue().to_string();
                if !composing && query != self.last_file_query {
                    self.last_file_query = query.clone();
                    if let Some(tree) = &self.document_tree {
                        let count = tree.filter(&query, &self.open_folders);
                        self.set_status(&if query.trim().is_empty() {
                            "모든 파일을 표시합니다".to_string()
                        } else if count == 0 {
                            "검색 결과가 없습니다".to_string()
                        } else {
                            format!("파일 검색 결과 {count}개")
                        });
                    }
                }
            }
        }
        self.capture_change();
        if self.map.is_none() && self.preview.is_none() {
            let selection: NSRange = msg_send![&self.text, selectedRange];
            let marked: bool = msg_send![&self.text, hasMarkedText];
            let responder: Option<Retained<AnyObject>> = msg_send![&self._window, firstResponder];
            let focused = responder
                .as_ref()
                .map(|r| Retained::as_ptr(r) == Retained::as_ptr(&self.text) as *const AnyObject)
                .unwrap_or(false);
            if !marked
                && (selection != self.last_selection
                    || focused != self.last_editor_focus
                    || (!editor::table_cell_is_editing(&self.text)
                        && editor::table_cells_need_layout(&self.text)))
            {
                self.last_selection = selection;
                self.last_editor_focus = focused;
                editor::style_with_base(&self.text, self.document.as_ref().and_then(document_base));
            }
        }
        if self.dirty
            && self.edited_at.elapsed() > Duration::from_millis(750)
            && !editor::table_cell_is_editing(&self.text)
        {
            self.flush();
        }
        if self.map.is_some() {
            let responder: Option<Retained<AnyObject>> = msg_send![&self._window, firstResponder];
            let window_only = responder
                .as_ref()
                .map(|r| Retained::as_ptr(r) == Retained::as_ptr(&self._window) as *const AnyObject)
                .unwrap_or(true);
            if window_only && self._window.isKeyWindow() {
                graph::focus_input();
            }
            while let Some(command) = graph::take_ui_command() {
                match command.as_str() {
                    cmd if cmd.starts_with("delete_node:") => self.confirm_nodes(vec![cmd[12..].to_owned()], Value::Null),
                    "hide_nodes" => {
                        self.nodes_open = false;
                        self.refresh_right();
                    }
                    "overview" => {
                        self.nodes_open = false;
                        self.selection = Value::Null;
                        self.refresh_right();
                    }
                    _ => {}
                }
            }
            if let Some(field) = &self.search {
                let query = field.stringValue().to_string();
                if query != self.last_query {
                    self.last_query = query.clone();
                    graph::search(&query);
                    let count = graph_controls::matches(&self.graph_data, &query).len();
                    self.set_status(if count == 0 {
                        "검색 결과가 없습니다"
                    } else {
                        "검색 결과에서 개념을 선택하거나 Enter를 누르세요"
                    });
                }
            }
            let selection = graph::selection().unwrap_or(Value::Null);
            if graph::take_node_selection() {
                if self.chat_mode {
                    self.dispatch_action(Action::RightMode(false));
                }
                if !self.sidebar_open[1] {
                    self.toggle_sidebar(1);
                }
            }
            if selection != self.selection {
                self.selection = selection;
                self.neighbor_limit = 8;
                self.refresh_relation_options();
                self.refresh_right();
            }
        }
        if self.last_job_poll.elapsed() > Duration::from_secs(2) {
            self.last_job_poll = Instant::now();
            if let Some(id) = self.connection_job.clone() {
                if let Ok(j) = rpc(json!({"op":"provider.job.status","id":id})) {
                    let state = j["state"].as_str().unwrap_or("");
                    self.set_status(&format!("공식 도구 연결 · {state}"));
                    if ["succeeded", "failed", "cancelled"].contains(&state) {
                        self.connection_job = None;
                        self.update_provider_row();
                        self.probe_provider();
                        if state == "failed" {
                            self.fail(j["error"].as_str().unwrap_or("연결에 실패했습니다."));
                        }
                    }
                }
            }
            if let Some(id) = self.job.clone() {
                if let Ok(j) = rpc(json!({"op":"analysis.status","id":id})) {
                    let state = j["state"].as_str().unwrap_or("");
                    // The worker writes the run record before its job state, so a finished job has a final record.
                    let here = self.workspace.as_deref() == j["workspace_id"].as_str();
                    if here {
                        self.load_run();
                    }
                    let run = self.run.clone().filter(|_| here).unwrap_or(Value::Null);
                    let single = run["single"] == true;
                    if ["succeeded", "failed", "cancelled"].contains(&state) {
                        self.job = None;
                        match state {
                            "succeeded" => self.set_status(j["result"]["text"].as_str().unwrap_or("정리 완료")),
                            "failed" if single => alert(
                                "분석 실패 · 원본은 보관됨",
                                j["error"].as_str().unwrap_or("분석 결과를 확인할 수 없습니다."),
                            ),
                            "failed" if run["state"] == "limit" => self.set_status("구독 사용 한도에 닿아 멈췄어요 · 이전 지도는 그대로예요"),
                            "failed" => self.set_status("정리를 마치지 못했어요 · 이전 지도는 그대로예요"),
                            _ => self.set_status("멈췄어요 · 원본과 기존 지도는 보존됩니다"),
                        }
                        self.reload_graph();
                        self.refresh_right();
                    } else {
                        let label = strv(&run["progress"], "label");
                        if !here {
                            self.set_status("다른 과목을 정리하는 중…");
                        } else if single || label.is_empty() {
                            self.set_status("자료 분석 중… · 필기와 편집을 계속할 수 있습니다");
                        } else {
                            self.set_status(&label);
                            if let Some(strip) = &self.organize_label {
                                strip.setStringValue(&NSString::from_str(&label));
                            }
                        }
                    }
                }
            }
        }
        self.update_analysis_help();
        let size = self.root.frame().size;
        if size != self.last_dimensions {
            self.layout(size);
        }
    }
    /// Reposition existing views only. Animation never reloads the tree, restyles
    /// Markdown, or replaces an editor/IME session.
    unsafe fn layout_frames(&mut self, size: NSSize) {
        // Unflipped sidebar lists grow upward. Preserve their visible top when
        // the viewport changes, otherwise rows briefly disappear below it.
        let mut sidebar_scrolls = Vec::new();
        for sidebar in [&self.left, &self.right] {
            for child in sidebar.subviews().iter() {
                if child.isKindOfClass(NSScrollView::class()) {
                    let document: Option<Retained<NSView>> = msg_send![&child, documentView];
                    if let Some(document) = document.filter(|view| !view.isFlipped()) {
                        let clip: Retained<NSClipView> = msg_send![&child, contentView];
                        let bounds = clip.bounds();
                        sidebar_scrolls.push((document, clip, bounds));
                    }
                }
            }
        }
        let layout = sidebar_layout::Layout::with_widths(size.width, self.sidebar_openness, self.left_width, self.right_width);
        let mut width = layout.center_width;
        if let Some(panel) = self.shortcuts.as_ref().filter(|panel| !panel.isHidden()) {
            let panel_width = sidebar_layout::DEFAULT_RIGHT_WIDTH;
            let x = size.width - 14. - panel_width;
            panel.setFrame(rect(x, 14., panel_width, size.height - 28.));
            width = width.min(x - sidebar_layout::RESIZE_WIDTH - layout.center_x);
        }
        let height = size.height - 28.;
        sidebar_resize::prepare_content_width(&self.left);
        self.left_shell.setFrame(rect(layout.left_x - self.left_drag_offset, 14., layout.left_width, height));
        self.left.setFrame(rect(0., 0., layout.left_width, height));
        self.left_hide_veil.setFrame(self.left_shell.frame());
        self.left_hide_veil.setHidden(!self.left_drag_pending);
        self.left_hide_arrow.setFrame(sidebar_hide::left_arrow_frame(self.left_shell.frame(), 14.));
        self.left_hide_arrow.setNeedsDisplay(true);
        self.left_hide_arrow.setHidden(!self.left_drag_pending);
        self.left.setAlphaValue(if self.left_drag_pending {0.45} else {1.});
        self.left_resizer.setFrame(rect(self.left_shell.frame().origin.x + layout.left_width,
            14., sidebar_layout::RESIZE_WIDTH, height));
        self.left_resizer.setHidden(!self.sidebar_open[0] || self.sidebar_motion.is_some());
        self._window.invalidateCursorRectsForView(&self.left_resizer);
        self.center.setFrame(rect(layout.center_x, 14., width, height));
        self.workspace_welcome.setFrame(rect(24., (height - 180.) / 2., (width - 48.).max(0.), 180.));
        let welcome_width = self.workspace_welcome.bounds().size.width;
        for (index, child) in self.workspace_welcome.subviews().iter().enumerate() {
            child.setFrame(match index {
                0 => rect(0., 130., welcome_width, 32.),
                1 => rect(0., 70., welcome_width, 48.),
                _ => rect((welcome_width - 180.) / 2., 10., 180., 36.),
            });
        }
        self.resize_right_contents(height - 54.);
        sidebar_resize::prepare_content_width(&self.right);
        self.right_shell.setFrame(rect(layout.right_x + self.right_drag_offset, 14., layout.right_width, height));
        self.right_hide_veil.setFrame(self.right_shell.frame());
        self.right_hide_veil.setHidden(!self.right_drag_pending);
        self.right_hide_arrow.setFrame(sidebar_hide::arrow_frame(
            self.right_shell.frame(), size.width - 14.));
        self.right_hide_arrow.setNeedsDisplay(true);
        self.right_hide_arrow.setHidden(!self.right_drag_pending);
        self.right_content.setAlphaValue(if self.right_drag_pending { 0.45 } else { 1. });
        for surface in [&self.left_shell, &self.center, &self.right_shell] {
            theme::sync_surface_shadow(surface);
        }
        self.right.setFrame(rect(0., 0., layout.right_width, height - 54.));
        self.chat.layout(layout.right_width, height - 54.);
        self.layout_right_tabs(layout.right_width, height);
        self.right_resizer.setFrame(rect(layout.right_x + self.right_drag_offset - sidebar_layout::RESIZE_WIDTH,
            14., sidebar_layout::RESIZE_WIDTH, height));
        self.right_resizer.setHidden(!self.sidebar_open[1] || self.sidebar_motion.is_some());
        self._window.invalidateCursorRectsForView(&self.right_resizer);
        for (document, clip, old) in sidebar_scrolls {
            if old.size.height != clip.bounds().size.height {
                document.scrollPoint(NSPoint::new(old.origin.x,
                    old.origin.y + old.size.height - clip.bounds().size.height));
            }
        }
        // Both cues belong to the window edge, outside the sidebar scrollbars.
        for side in 0..2 {
            self.sidebar_handles[side].setHidden(self.left_resizer.is_dragging() || self.right_resizer.is_dragging());
            let frame = rect(if side == 0 { 0. } else { size.width - sidebar_layout::GLOW_WIDTH },
                0., sidebar_layout::GLOW_WIDTH, size.height);
            self.sidebar_handles[side].setFrame(frame);
            self._window.invalidateCursorRectsForView(&self.sidebar_handles[side]);
        }
        self.header
            .setFrame(rect(28., height - 61., width - 210., 38.));
        self.subtitle
            .setFrame(rect(29., height - 82., width - 100., 20.));
        self.save_button
            .setFrame(rect(width - 89., height - 53., 67., 30.));
        self.delete_button
            .setFrame(rect(width - 160., height - 53., 63., 30.));
        self.layout_status(width);
        self.map_button.setFrame(rect(width - 192., 13., 170., 30.));
        let f = rect(24., 58., width - 48., height - 156.);
        // Keep the scrollbar and text selection clear of collapsed edge targets.
        let edge_inset = sidebar_layout::HANDLE_WIDTH - 14.;
        let left_inset = edge_inset * (1. - self.sidebar_openness[0]);
        let right_inset = edge_inset * (1. - self.sidebar_openness[1]);
        self.editor_scroll
            .setFrame(rect(left_inset, f.origin.y, width - left_inset - right_inset, f.size.height));
        let viewport_height = self.editor_scroll.contentSize().height;
        self.text.setMinSize(NSSize::new(0., viewport_height));
        let mut editor_size = self.text.frame().size;
        if editor_size.height < viewport_height {
            editor_size.height = viewport_height;
            self.text.setFrameSize(editor_size);
        }
        if let Some(p) = &self.preview {
            p.setFrame(f);
        }
        if let Some(g) = &self.map {
            g.setFrame(rect(0., 58., width, height - 158.));
        }
        if let Some(s) = &self.search {
            s.setFrame(rect(24., height - 139., 210., 28.));
        }
        if let Some(c) = &self.search_clear {
            c.setFrame(rect(238., height - 139., 26., 28.));
        }
        if let Some(r) = &self.relation {
            r.setFrame(rect(270., height - 139., (width - 294.).min(166.), 28.));
        }
        self.update_analysis_help();
        self.layout_diagram();
    }
    unsafe fn layout_right_tabs(&self, width: f64, height: f64) {
        // The tabs use the header width; collapse lives on the outer edge.
        let tab_width = (width - 32.) / 2.;
        self.right_tab_group.setFrame(rect(12., height - 48., width - 24., 40.));
        self.right_tab_selection.setFrame(rect(4. + if self.chat_mode { tab_width } else { 0. },
            4., tab_width, 32.));
        self.tab_dot.setFrame(rect(tab_width - 12., 26., 7., 7.));
        // Measure the full native label before choosing the compact display.
        // Both tabs use the same mode so switching selection never clips a title.
        for button in &self.right_tabs {
            button.setImagePosition(NSCellImagePosition::ImageLeading);
        }
        let compact = self.right_tabs.iter().any(|button|
            button.intrinsicContentSize().width + 16. > tab_width);
        for i in 0..2 {
            let button = &self.right_tabs[i];
            button.setFrame(rect(4. + tab_width * i as f64, 4., tab_width, 32.));
            button.setImagePosition(if compact { NSCellImagePosition::ImageOnly }
                else { NSCellImagePosition::ImageLeading });
        }
    }
    unsafe fn layout(&mut self, size: NSSize) {
        self.last_dimensions = size;
        self.layout_frames(size);
        self.refresh_left();
        self.refresh_right();
        if !self.snapshot && self.preview.is_none() && self.map.is_none() {
            editor::style_with_base(&self.text, self.document.as_ref().and_then(document_base));
        }
    }
    unsafe fn toggle_sidebar(&mut self, side: usize) {
        // Keep a live filename edit visible, including an uncommitted IME string.
        if side == 1 && self.sidebar_open[1]
            && self.document_tree.as_ref().is_some_and(|t| t.renaming_workspace().is_some()) {
            self.set_status("제목 수정을 마친 뒤 자료 사이드바를 접어 주세요");
            return;
        }
        if side == 1 && self.sidebar_open[1] {
            if let Some(field) = &self.file_search {
                if let Some(editor) = field.currentEditor() {
                    let marked: bool = msg_send![&editor, hasMarkedText];
                    if marked { return; }
                    // A hidden search field must not keep accepting keyboard input.
                    if !self._window.makeFirstResponder(Some(&self.text)) { return; }
                }
            }
        }
        let now = Instant::now();
        if let Some(motion) = &self.sidebar_motion {
            self.sidebar_openness = motion.sample(now).0;
            self.right_drag_offset = motion.right_offset(now);
            self.left_drag_offset = motion.left_offset(now);
        }
        if side == 0 && !self.sidebar_open[0] { self.left_drag_pending = false; }
        if side == 1 && !self.sidebar_open[1] { self.right_drag_pending = false; }
        self.sidebar_open[side] = !self.sidebar_open[side];
        let target = self.sidebar_open.map(|open| if open { 1. } else { 0. });
        self.left_shell.setHidden(false);
        self.right_shell.setHidden(false);
        for i in 0..2 {
            // Keep both edge controls available during either transition.
            sidebar_reveal::set_open(&self.sidebar_handles[i], self.sidebar_open[i]);
        }
        if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            self.sidebar_openness = target;
            self.right_drag_offset = 0.;
            self.left_drag_offset = 0.;
            self.layout_frames(self.root.frame().size);
            self.finish_sidebar_motion();
            return;
        }
        self.sidebar_motion = Some(sidebar_layout::Motion::new(self.sidebar_openness, target, now)
            .with_right_offset(self.right_drag_offset).with_left_offset(self.left_drag_offset));
        if self.sidebar_timer.is_none() {
            let timer: Retained<NSTimer> = msg_send![NSTimer::class(),
                timerWithTimeInterval:(1. / 60.) target:&*self.target
                selector:sel!(animateSidebar:) userInfo:std::ptr::null::<AnyObject>() repeats:true];
            NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes);
            self.sidebar_timer = Some(timer);
        }
    }
    unsafe fn animate_sidebar(&mut self) {
        let Some(motion) = &self.sidebar_motion else { return; };
        let now = Instant::now();
        let (position, done) = motion.sample(now);
        self.right_drag_offset = motion.right_offset(now);
        self.left_drag_offset = motion.left_offset(now);
        self.sidebar_openness = position;
        self.layout_frames(self.root.frame().size);
        if done { self.finish_sidebar_motion(); }
    }
    unsafe fn finish_sidebar_motion(&mut self) {
        self.sidebar_motion = None;
        self.left_drag_offset = 0.;
        self.left_drag_pending = false;
        self.left_hide_veil.setHidden(true);
        self.left_hide_arrow.setHidden(true);
        self.left.setAlphaValue(1.);
        self.left_resizer.setHidden(!self.sidebar_open[0]);
        self.right_drag_offset = 0.;
        self.right_drag_pending = false;
        self.right_hide_veil.setHidden(true);
        self.right_hide_arrow.setHidden(true);
        self.right_content.setAlphaValue(1.);
        for handle in &self.sidebar_handles { handle.setHidden(false); }
        self.right_resizer.setHidden(!self.sidebar_open[1]);
        if let Some(timer) = self.sidebar_timer.take() { timer.invalidate(); }
        self.left_shell.setHidden(!self.sidebar_open[0]);
        self.right_shell.setHidden(!self.sidebar_open[1]);
        // Markdown tables/images use measured overlays; settle their width once,
        // preserving the source and selection. The editor protects active IME
        // and table editing and restyles after those edits finish.
        if self.preview.is_none() && self.map.is_none() {
            editor::style_with_base(&self.text, self.document.as_ref().and_then(document_base));
        }
    }
}
#[derive(Clone, Copy)]
enum ProviderTone {
    Ok,
    Busy,
    Warn,
    Error,
    Idle,
}
impl ProviderTone {
    /// 시안 A 색: 초록 연결됨 · 남색 진행 중 · 주황 로그인 필요 · 빨강 설치 필요 · 회색 미선택/확인 중.
    fn color(self) -> Retained<NSColor> {
        let (r, g, b) = match self {
            ProviderTone::Ok => (0.13, 0.63, 0.42),
            ProviderTone::Busy => (0.31, 0.27, 0.90),
            ProviderTone::Warn => (0.88, 0.54, 0.12),
            ProviderTone::Error => (0.85, 0.27, 0.23),
            ProviderTone::Idle => (0.42, 0.43, 0.51),
        };
        NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.)
    }
}
unsafe fn menu(app: &NSApplication, target: &Delegate, updater: Option<&AnyObject>) {
    let main = NSMenu::new(mtm());
    let item = NSMenuItem::new(mtm());
    let m = NSMenu::new(mtm());
    if let Some(updater) = updater {
        let check = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm()),
            &NSString::from_str("업데이트 확인…"),
            Some(sel!(checkForUpdates:)),
            &NSString::from_str(""),
        );
        check.setTarget(Some(updater));
        m.addItem(&check);
        m.addItem(&NSMenuItem::separatorItem(mtm()));
    }
    let quit = NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm()),
        &NSString::from_str("Stology 종료"),
        Some(sel!(terminate:)),
        &NSString::from_str("q"),
    );
    m.addItem(&quit);
    item.setSubmenu(Some(&m));
    main.addItem(&item);
    let file = NSMenuItem::new(mtm());
    file.setTitle(&NSString::from_str("파일"));
    let fm = NSMenu::new(mtm());
    let new = NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm()),
        &NSString::from_str("새 노트"),
        Some(sel!(newDocument:)),
        &NSString::from_str("n"),
    );
    new.setTarget(Some(target));
    fm.addItem(&new);
    let save = NSMenuItem::initWithTitle_action_keyEquivalent(
        NSMenuItem::alloc(mtm()),
        &NSString::from_str("저장"),
        Some(sel!(saveDocument:)),
        &NSString::from_str("s"),
    );
    save.setTarget(Some(target));
    fm.addItem(&save);
    for (title, selector) in [
        ("PDF 공유…", sel!(shareDocument:)),
        ("PDF 저장 문제 해결…", sel!(resolvePdfSave:)),
    ] {
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm()),
            &NSString::from_str(title),
            Some(selector),
            &NSString::from_str(""),
        );
        item.setTarget(Some(target));
        fm.addItem(&item);
    }
    file.setSubmenu(Some(&fm));
    main.addItem(&file);
    let edit = NSMenuItem::new(mtm());
    edit.setTitle(&NSString::from_str("편집"));
    let em = NSMenu::new(mtm());
    for (title, selector, key) in [
        ("실행 취소", sel!(undo:), "z"),
        ("다시 실행", sel!(redo:), "Z"),
        ("잘라내기", sel!(cut:), "x"),
        ("복사", sel!(copy:), "c"),
        ("붙여넣기", sel!(paste:), "v"),
        ("모두 선택", sel!(selectAll:), "a"),
        ("선택 영역 확장", sel!(expandSelection:), "d"),
    ] {
        let i = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm()),
            &NSString::from_str(title),
            Some(selector),
            &NSString::from_str(key),
        );
        em.addItem(&i);
    }
    edit.setSubmenu(Some(&em));
    main.addItem(&edit);
    let mapitem = NSMenuItem::new(mtm());
    mapitem.setTitle(&NSString::from_str("지식지도"));
    let mapmenu = NSMenu::new(mtm());
    for control in [false, true] {
        let search = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm()),
            &NSString::from_str("개념 검색"),
            Some(sel!(focusMapSearch:)),
            &NSString::from_str("k"),
        );
        search.setTarget(Some(target));
        if control {
            search.setKeyEquivalentModifierMask(NSEventModifierFlags::Control);
            search.setHidden(true);
            search.setAllowsKeyEquivalentWhenHidden(true);
        }
        mapmenu.addItem(&search);
    }
    mapitem.setSubmenu(Some(&mapmenu));
    main.addItem(&mapitem);
    let help_item = NSMenuItem::new(mtm());
    help_item.setTitle(&NSString::from_str("도움말"));
    let help = NSMenu::new(mtm());
    help_item.setSubmenu(Some(&help));
    main.addItem(&help_item);
    app.setHelpMenu(Some(&help));
    app.setMainMenu(Some(&main));
}
unsafe fn shortcut_icon(button: &NSButton, symbol: &str, label: &str) {
    let label = NSString::from_str(label);
    button.setBordered(false);
    button.setAccessibilityLabel(Some(&label));
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(&NSString::from_str(symbol), Some(&label));
    button.setImage(image.as_deref());
    button.setImagePosition(NSCellImagePosition::ImageOnly);
}

fn create_diagram_from_menu() {
    STATE.with(|state| {
        if let Some(app) = state.borrow_mut().as_mut() {
            unsafe { app.start_diagram(None, true); }
        }
    });
}

fn main() {
    unsafe {
        font::register_bundled_fonts();
        let app = NSApplication::sharedApplication(mtm());
        app.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        let target: Retained<Delegate> = msg_send![Delegate::alloc(mtm()), init];
        app.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let updater = updater::start();
        menu(&app, &target, updater);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            window_appearance::alloc(mtm()),
            rect(0., 0., 1280., 820.),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setTitle(&NSString::from_str("Stology — 개인 지식 작업실"));
        theme::apply_window(&window);
        window.setAppearance(NSAppearance::appearanceNamed(NSAppearanceNameAqua).as_deref());
        window.setMinSize(NSSize::new(1100., 680.));
        window.setReleasedWhenClosed(false);
        window.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        window.setFrameAutosaveName(&NSString::from_str("StologyLocalMain"));
        let root = view(rect(0., 0., 1280., 820.));
        root.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let (left_shell, left) = theme::panel(rect(14., 14., 196., 792.));
        let (right_shell, right_content, right) =
            theme::panel_with_body(rect(1010., 14., 256., 792.), 54.);
        let center = view(rect(210., 0., 800., 820.));
        theme::canvas(&root);
        theme::document(&center);
        root.addSubview(&left_shell);
        root.addSubview(&center);
        root.addSubview(&right_shell);
        let title_field: Retained<NoteTitleField> = msg_send![NoteTitleField::alloc(mtm()), initWithFrame: rect(28., 759., 590., 38.)];
        let header: Retained<NSTextField> = Retained::cast_unchecked(title_field);
        header.setBezeled(false);
        header.setDrawsBackground(false);
        header.setFont(Some(&font::bold(editor::heading_size(1))));
        header.setTextColor(Some(&NSColor::labelColor()));
        let subtitle = text_label(
            "왼쪽에서 과목을 만들어 시작하세요",
            rect(29., 738., 700., 20.),
            11.,
            false,
        );
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        header.setFocusRingType(NSFocusRingType::None);
        header.setIdentifier(Some(&NSString::from_str("new-note-title")));
        header.setDelegate(Some(ProtocolObject::from_ref(&*target)));
        let _: () = msg_send![&header, setAccessibilityLabel: &*NSString::from_str("노트 제목")];
        center.addSubview(&header);
        let title_divider = theme::soft_divider(rect(14., 729., 772., 1.), true);
        center.addSubview(&title_divider);
        let analysis_help = text_label("", rect(0., 0., 90., 18.), 11., false);
        analysis_help.setHidden(true);
        center.addSubview(&analysis_help);
        center.addSubview(&subtitle);
        let status = text_label(
            "로컬 저장 · 개인 작업실",
            rect(26., 18., 580., 20.),
            10.,
            false,
        );
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        center.addSubview(&status);
        let ai_spinner = NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm()), rect(26., 22., 14., 14.));
        ai_spinner.setStyle(NSProgressIndicatorStyle::Spinning);
        ai_spinner.setControlSize(NSControlSize::Small);
        ai_spinner.setDisplayedWhenStopped(false);
        ai_spinner.setAccessibilityLabel(Some(&NSString::from_str("AI 작업 중")));
        ai_spinner.setHidden(true);
        center.addSubview(&ai_spinner);
        let (scroll, text) = editor::make_editor(rect(0., 58., 800., 664.));
        center.addSubview(&scroll);
        text.setEditable(false);
        let placeholder = NSButton::new(mtm());
        let mut s = State {
            target: target.clone(),
            workspace_rows: Vec::new(),
            workspace_rename: None,
            _window: window.clone(),
            root: root.clone(),
            left,
            left_shell,
            right_shell,
            right_content,
            right,
            center,
            workspace_welcome: view(rect(0., 0., 600., 180.)),
            chat: chat_panel::Panel::new(),
            chat_mode: rpc(json!({"op":"chat.preferences"})).ok().is_some_and(|v|v["mode"]=="chat"),
            right_tabs: [placeholder.clone(), placeholder.clone()],
            right_tab_group: view(rect(0.,0.,274.,40.)),
            right_tab_selection: view(rect(4.,4.,133.,32.)),
            sidebar_handles: [placeholder.clone(), placeholder.clone()],
            sidebar_open: [true, true],
            left_width: sidebar_layout::MIN_LEFT_WIDTH,
            left_drag_offset: 0.,
            left_drag_pending: false,
            left_hide_veil: sidebar_hide::new(true),
            left_hide_arrow: sidebar_hide::new_arrow(true),
            left_resizer: sidebar_resize::Divider::new(0),
            shortcuts: None,
            right_width: sidebar_layout::DEFAULT_RIGHT_WIDTH,
            right_drag_offset: 0.,
            right_drag_pending: false,
            right_hide_veil: sidebar_hide::new(false),
            right_hide_arrow: sidebar_hide::new_arrow(false),
            right_resizer: sidebar_resize::Divider::new(1),
            sidebar_openness: [1., 1.],
            sidebar_motion: None,
            sidebar_timer: None,
            header,
            analysis_help,
            editing_new_title: false,
            blank_title: "제목 없음".into(),
            title_divider,
            subtitle,
            status,
            ai_spinner,
            diagram: None,
            save_button: placeholder.clone(),
            delete_button: placeholder.clone(),
            map_button: placeholder,
            editor_scroll: scroll,
            text,
            preview: None,
            preview_pending: None,
            preview_rendered: false,
            preview_editable: false,
            annotation_conversion: None,
            annotation_publish: None,
            analysis_waiting: None,
            deferred_tree: None,
            deferred_action: None,
            last_pdf_status: String::new(),
            share_waiting: None,
            share_picker: None,
            preview_locator: None,
            map: None,
            search: None,
            search_clear: None,
            file_search: None,
            last_file_query: String::new(),
            last_query: String::new(),
            nodes_open: true,
            node_limit: 40,
            expanded_link: String::new(),
            neighbor_limit: 8,
            desc_field: None,
            desc_hint: None,
            desc_concept: String::new(),
            desc_plain: String::new(),
            screens: Default::default(),
            current_evidence: None,
            map_preferences: std::collections::HashMap::new(),
            relation: None,
            cleanup_review: None,
            graph_data: json!({}),
            selection: Value::Null,
            workspace: None,
            document: None,
            last_text: String::new(),
            last_selection: NSRange::new(usize::MAX, 0),
            last_editor_focus: false,
            dirty: false,
            edited_at: Instant::now(),
            actions: vec![],
            open_folders: HashSet::new(),
            document_tree: None,
            tree_refresh_pending: false,
            job: None,
            last_job_poll: Instant::now(),
            last_dimensions: NSSize::new(0., 0.),
            provider: String::new(),
            snapshot: false,
            connection_job: None,
            provider_info: None,
            connection_installing: false,
            connection_node: false,
            provider_probe: None,
            provider_probe_again: false,
            connect_button: None,
            connect_help: None,
            pending: None,
            run: None,
            organize_label: None,
            suggestions: Value::Null,
            suggestions_for: None,
            last_suggestion_poll: Instant::now(),
            tab_dot: theme::chat_status_dot(rect(0., 0., 7., 7.)),
            suggest_button: None,
            suggest_open: false,
            suggest_confirm: None,
            suggest_layer: Vec::new(),
            unit_focus: String::new(),
        };
        // Keep the toggle in the title bar so it remains reachable when collapsed.
        if let Some(close) = window.standardWindowButton(NSWindowButton::CloseButton) {
            if let Some(titlebar) = close.superview() {
                let f = close.frame();
                let new_note = NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(""), Some(&target), Some(sel!(newDocument:)), mtm(),
                );
                new_note.setFrame(rect(titlebar.bounds().size.width - 106., f.origin.y - 3., 28., 22.));
                new_note.setBordered(false);
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("plus"), Some(&NSString::from_str("새 노트")),
                ) {
                    new_note.setImage(Some(&image));
                    new_note.setImagePosition(NSCellImagePosition::ImageOnly);
                }
                new_note.setAccessibilityLabel(Some(&NSString::from_str("새 노트")));
                new_note.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin | NSAutoresizingMaskOptions::ViewMinXMargin);
                titlebar.addSubview(&new_note);
                let shortcuts = s.button("", rect(titlebar.bounds().size.width - 74., f.origin.y - 3., 28., 22.), Action::ToggleShortcuts);
                shortcut_icon(&shortcuts, "keyboard", "단축키 열기 또는 닫기");
                shortcuts.setAutoresizingMask(new_note.autoresizingMask());
                titlebar.addSubview(&shortcuts);
                for side in 0..2 {
                let x = if side == 0 { f.origin.x + 76. } else { titlebar.bounds().size.width - 42. };
                let label = if side == 0 { "왼쪽 사이드바 접기 또는 펼치기" } else { "오른쪽 사이드바 접기 또는 펼치기" };
                let toggle = s.button("", rect(x, f.origin.y - 3., 28., 22.), Action::ToggleSidebar(side));
                toggle.setBordered(false);
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str(if side == 0 { "sidebar.left" } else { "sidebar.right" }),
                    Some(&NSString::from_str(label)),
                ) {
                    toggle.setImage(Some(&image));
                    toggle.setImagePosition(NSCellImagePosition::ImageOnly);
                }
                toggle.setAccessibilityLabel(Some(&NSString::from_str(label)));
                toggle.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinYMargin | if side == 1 { NSAutoresizingMaskOptions::ViewMinXMargin } else { NSAutoresizingMaskOptions::empty() });
                titlebar.addSubview(&toggle);
                }
            }
        }
        s.right_tabs = [s.button("자료", rect(0.,0.,100.,28.), Action::RightMode(false)), s.button("AI", rect(0.,0.,126.,28.), Action::RightMode(true))];
        let (group, tab_content)=theme::chat_tab_group(rect(0.,0.,274.,40.));
        s.right_tab_group=group;
        s.right_tab_selection=theme::chat_tab_selection(rect(if s.chat_mode {137.}else{4.},4.,133.,32.));
        tab_content.addSubview(&s.right_tab_selection);
        for i in 0..2 {
            s.right_tabs[i].setFrame(rect(4. + 133. * i as f64,4.,133.,32.));
            theme::chat_tab(&s.right_tabs[i],s.chat_mode==(i==1),i);
            tab_content.addSubview(&s.right_tabs[i]);
        }
        theme::chat_status_dot_state(&s.tab_dot, true);
        s.tab_dot.setHidden(true);
        tab_content.addSubview(&s.tab_dot);
        s.right_content.addSubview(&s.right_tab_group);
        s.right_content.addSubview(&s.chat.view);
        s.chat.view.setHidden(!s.chat_mode);
        s.right.setHidden(s.chat_mode);
        for side in 0..2 {
            let handle = sidebar_reveal::new(side);
            handle.setTarget(Some(&s.target));
            handle.setAction(Some(sel!(action:)));
            handle.setTag(s.register(Action::ToggleSidebar(side)));
            s.root.addSubview(&handle);
            s.sidebar_handles[side] = handle;
        }
        s.root.addSubview(&s.left_hide_veil);
        s.root.addSubview(&s.left_hide_arrow);
        s.root.addSubview(&s.left_resizer);
        s.root.addSubview(&s.right_hide_veil);
        s.root.addSubview(&s.right_hide_arrow);
        s.root.addSubview(&s.right_resizer);
        let close_shortcuts = s.button("", rect(0., 0., 32., 32.), Action::CloseShortcuts);
        shortcut_icon(&close_shortcuts, "xmark", "단축키 닫기");
        let shortcuts = shortcut_panel::new(&close_shortcuts);
        s.root.addSubview(&shortcuts);
        s.shortcuts = Some(shortcuts);
        s.save_button = s.button("저장", rect(711., 767., 67., 30.), Action::Save);
        s.save_button.setKeyEquivalent(&NSString::from_str("s"));
        s.center.addSubview(&s.save_button);
        s.delete_button = s.button("삭제", rect(640., 767., 63., 30.), Action::Delete);
        s.center.addSubview(&s.delete_button);
        s.map_button = s.button(
            "지식지도 펼치기 ↗",
            rect(608., 13., 170., 30.),
            Action::ToggleMap,
        );
        s.center.addSubview(&s.map_button);
        let welcome_title = text_label("워크스페이스를 먼저 만들어 주세요", rect(0., 130., 600., 32.), 21., true);
        welcome_title.setAlignment(NSTextAlignment::Center);
        let welcome_detail = text_label("과목이나 주제별로 공간을 만들고\n노트 작성과 자료 정리를 시작하세요.", rect(0., 70., 600., 48.), 14., false);
        welcome_detail.setAlignment(NSTextAlignment::Center);
        welcome_detail.setMaximumNumberOfLines(2);
        welcome_detail.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let welcome_button = s.button("워크스페이스 만들기", rect(210., 10., 180., 36.), Action::NewWorkspace);
        s.workspace_welcome.addSubview(&welcome_title);
        s.workspace_welcome.addSubview(&welcome_detail);
        s.workspace_welcome.addSubview(&welcome_button);
        s.center.addSubview(&s.workspace_welcome);
        if let Ok(ws) = rpc(json!({"op":"workspace.list"})) {
            s.workspace = ws["workspaces"]
                .as_array()
                .and_then(|a| a.first())
                .map(|w| strv(w, "id"));
            s.text.setEditable(s.workspace.is_some());
        }
        let data_root = std::env::var_os("STOLOGY_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join("Library/Application Support/Stology Local")
            });
        if let Ok(bytes) = std::fs::read(data_root.join("provider.json")) {
            if let Ok(p) = serde_json::from_slice::<Value>(&bytes) {
                s.provider = strv(&p, "provider");
            }
        }
        window.setContentView(Some(&root));
        window.center();
        // Attaching the root gives it the restored window's actual content size.
        s.layout(root.bounds().size);
        s.update_header();
        s.probe_provider();
        editor::set_diagram_opener(|link|STATE.with(|state|{
            if let Ok(mut state)=state.try_borrow_mut(){if let Some(s)=state.as_mut(){s.start_diagram(Some(link), false);}}
        }));
        editor::set_image_importer(import_image);
        STATE.with(|state| *state.borrow_mut() = Some(s));
        window.makeKeyAndOrderFront(None);
        app.activateIgnoringOtherApps(true);
        let _timer: Retained<NSTimer> = msg_send![NSTimer::class(),scheduledTimerWithTimeInterval:0.20f64 target:&*target selector:sel!(tick:) userInfo:std::ptr::null::<AnyObject>() repeats:true];
        app.run();
    }
}
