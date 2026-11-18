//! Main-thread PDF sessions. Acknowledgements describe durable journal and
//! published PDF separately; leaving a view never discards an unsaved session.
use crate::pdf_save::{Event, Request, Worker};
use objc2::{
    rc::{Retained, Weak},
    runtime::AnyObject,
};
use objc2_app_kit::NSView;
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    rc::Rc,
    sync::mpsc::TrySendError,
};

const MAX_OUTSTANDING_BYTES: usize = 8 * 1024 * 1024;
struct Session {
    worker: Worker,
    ready: bool,
    edit_seq: u64,
    journal_seq: u64,
    pdf_seq: u64,
    outstanding: VecDeque<(u64, usize)>,
    outstanding_bytes: usize,
    error: Option<String>,
    error_seq: u64,
    rescuing: bool,
    document: Retained<AnyObject>,
    view: Option<Weak<NSView>>,
    active: bool,
    recovered: Vec<Value>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.worker.sender.try_send(Request::Stop);
    }
}
thread_local! {static SESSIONS:RefCell<HashMap<String,Rc<RefCell<Session>>>>=RefCell::new(HashMap::new());}
pub struct SaveEvent {
    pub document_id: String,
    pub version: i64,
    pub hash: String,
    pub path: String,
    pub resolved: bool,
    pub export_path: Option<String>,
}

/// Return a callback bound to this document. The UI applies first, then rolls
/// back if this callback refuses admission. Bounded admission never drops edits.
pub unsafe fn connect(
    id: &str,
    document: Retained<AnyObject>,
) -> Result<Rc<dyn Fn(Value) -> Result<(), String>>, String> {
    let session = SESSIONS.with(|sessions| -> Result<_, String> {
        let mut sessions = sessions.borrow_mut();
        if let Some(existing) = sessions.get(id) {
            let mut s = existing.borrow_mut();
            if s.edit_seq != s.pdf_seq || (!s.ready && s.error.is_none()) {
                return Err("이 PDF의 저장이 진행 중입니다. 저장 후 다시 열어 주세요.".into());
            }
            s.active = false;
        }
        let s = Rc::new(RefCell::new(Session {
            worker: crate::pdf_save::start(id.to_owned()),
            ready: false,
            edit_seq: 0,
            journal_seq: 0,
            pdf_seq: 0,
            outstanding: VecDeque::new(),
            outstanding_bytes: 0,
            error: None,
            error_seq: 0,
            rescuing: false,
            document,
            view: None,
            active: true,
            recovered: Vec::new(),
        }));
        sessions.insert(id.to_owned(), s.clone());
        Ok(s)
    })?;
    Ok(Rc::new(move |command| {
        let mut s = session
            .try_borrow_mut()
            .map_err(|_| "필기 상태를 갱신 중입니다. 다시 시도해 주세요.")?;
        if !s.ready {
            return Err("PDF 복구 기록을 불러오는 중입니다.".into());
        }
        if !s.rescuing {
            if let Some(e) = &s.error {
                return Err(e.clone());
            }
        }
        let size = command.to_string().len();
        if size > 4 * 1024 * 1024 {
            return Err(
                "한 번의 필기가 4 MiB 한도를 넘었습니다. 획을 나누어 작성해 주세요.".into(),
            );
        }
        if s.outstanding_bytes.saturating_add(size) > MAX_OUTSTANDING_BYTES {
            return Err(
                "필기 저장을 기다려 주세요. 미저장 편집 용량이 한도에 도달했습니다.".into(),
            );
        }
        let seq = s
            .edit_seq
            .checked_add(1)
            .ok_or("PDF 편집 순서가 한도를 넘었습니다")?;
        match s.worker.sender.try_send(Request::Edit { seq, command }) {
            Ok(()) => {
                s.edit_seq = seq;
                s.outstanding.push_back((seq, size));
                s.outstanding_bytes += size;
                unsafe { update_label(&s) };
                Ok(())
            }
            Err(TrySendError::Full(_)) => {
                Err("필기 저장 큐가 가득 찼습니다. 잠시 후 다시 시도해 주세요.".into())
            }
            Err(TrySendError::Disconnected(_)) => {
                s.error = Some("PDF 저장 작업자가 종료되었습니다. 복구 기록은 보존됩니다.".into());
                Err(s.error.clone().unwrap())
            }
        }
    }))
}
pub unsafe fn attach(id: &str, view: &NSView) {
    SESSIONS.with(|sessions| {
        if let Some(s) = sessions.borrow().get(id) {
            let mut s = s.borrow_mut();
            s.view = Some(Weak::from_retained(
                &Retained::retain(view as *const _ as *mut NSView).unwrap(),
            ));
            update_label(&s);
        }
    });
}
pub unsafe fn detach(view: &NSView) {
    SESSIONS.with(|sessions| {
        for s in sessions.borrow().values() {
            let mut s = s.borrow_mut();
            if s.view
                .as_ref()
                .and_then(Weak::load)
                .is_some_and(|v| Retained::as_ptr(&v) == view as *const NSView)
            {
                s.active = false;
            }
        }
    });
}
fn label(s: &Session) -> String {
    if let Some(e) = &s.error {
        format!("저장 실패 · {e}")
    } else if !s.ready {
        "PDF 복구 확인 중".into()
    } else if s.edit_seq > s.pdf_seq {
        "저장 중".into()
    } else {
        "PDF에 저장됨".into()
    }
}
unsafe fn update_label(s: &Session) {
    if let Some(view) = s.view.as_ref().and_then(Weak::load) {
        crate::pdf_annotations::set_status(&view, &label(s));
    }
}
pub fn status(id: &str) -> Option<String> {
    SESSIONS.with(|sessions| sessions.borrow().get(id).map(|s| label(&s.borrow())))
}
pub fn poll() -> Vec<SaveEvent> {
    let mut changes = Vec::new();
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        for (id, s) in sessions.iter() {
            let mut s = s.borrow_mut();
            while let Ok(event) = s.worker.events.try_recv() {
                match event {
                    Event::Ready {
                        version,
                        hash,
                        path,
                        pdf_seq,
                        journal_seq,
                        commands,
                    } => {
                        s.pdf_seq = pdf_seq;
                        s.journal_seq = journal_seq;
                        s.edit_seq = journal_seq;
                        let mut error = None;
                        for command in &commands {
                            if let Err(e) =
                                unsafe { crate::pdf_annotations::apply(&s.document, command) }
                            {
                                error = Some(e);
                                break;
                            }
                        }
                        s.outstanding = commands
                            .iter()
                            .zip((pdf_seq + 1)..)
                            .map(|(c, seq)| (seq, c.to_string().len()))
                            .collect();
                        s.outstanding_bytes = s.outstanding.iter().map(|(_, n)| *n).sum();
                        s.recovered = commands;
                        s.error = error;
                        s.error_seq = journal_seq;
                        s.ready = s.error.is_none();
                        if let Some(view) = s.view.as_ref().and_then(Weak::load) {
                            view.setNeedsDisplay(true);
                        }
                        changes.push(SaveEvent {
                            document_id: id.clone(),
                            version,
                            hash,
                            path,
                            resolved: false,
                            export_path: None,
                        });
                    }
                    Event::Journal(seq) => {
                        s.journal_seq = s.journal_seq.max(seq);
                    }
                    Event::Published {
                        version,
                        hash,
                        path,
                        seq,
                    } => {
                        s.pdf_seq = s.pdf_seq.max(seq);
                        // Keep a byte budget until PDF publication, not merely the
                        // journal ACK: a slow serializer must not grow indefinitely.
                        while s.outstanding.front().is_some_and(|(n, _)| *n <= seq) {
                            let (_, size) = s.outstanding.pop_front().unwrap();
                            s.outstanding_bytes = s.outstanding_bytes.saturating_sub(size);
                        }
                        s.recovered.clear();
                        if seq >= s.error_seq {
                            s.error = None;
                        }
                        changes.push(SaveEvent {
                            document_id: id.clone(),
                            version,
                            hash,
                            path,
                            resolved: false,
                            export_path: None,
                        });
                    }
                    Event::Resolved { export_path } => {
                        s.pdf_seq = s.edit_seq;
                        s.journal_seq = s.edit_seq;
                        s.error = None;
                        s.ready = true;
                        s.active = false;
                        s.outstanding.clear();
                        s.outstanding_bytes = 0;
                        changes.push(SaveEvent {
                            document_id: id.clone(),
                            version: 0,
                            hash: String::new(),
                            path: String::new(),
                            resolved: true,
                            export_path,
                        });
                    }
                    Event::Failed { seq, message } => {
                        s.error = Some(message);
                        s.error_seq = seq;
                    }
                }
            }
            unsafe { update_label(&s) };
        }
        sessions.retain(|_, s| {
            let s = s.borrow();
            s.active || !s.ready || s.error.is_some() || s.edit_seq != s.pdf_seq
        });
    });
    changes
}
fn commit_views() -> bool {
    let views = SESSIONS.with(|sessions| {
        sessions
            .borrow()
            .values()
            .filter_map(|s| s.borrow().view.as_ref().and_then(Weak::load))
            .collect::<Vec<_>>()
    });
    let mut committed = true;
    for view in views {
        if !unsafe { crate::pdf_annotations::commit_pending(&view) } {
            committed = false
        }
    }
    committed
}
pub fn request_flush_all() {
    commit_views();
    SESSIONS.with(|sessions| {
        for s in sessions.borrow().values() {
            let _ = s.borrow().worker.sender.try_send(Request::Flush);
        }
    });
}
pub fn flush_state() -> Result<bool, String> {
    let committed = commit_views();
    SESSIONS.with(|sessions| {
        let mut ready = committed;
        for s in sessions.borrow().values() {
            let s = s.borrow();
            if let Some(e) = &s.error {
                return Err(e.clone());
            }
            if !s.ready || s.edit_seq != s.pdf_seq {
                ready = false
            }
        }
        Ok(ready)
    })
}
pub fn has_pending() -> bool {
    SESSIONS.with(|sessions| {
        sessions.borrow().values().any(|s| {
            let s = s.borrow();
            !s.ready || s.error.is_some() || s.edit_seq != s.pdf_seq
        })
    })
}
/// Only call after core confirms deletion, or after a successful flush before
/// moving/reopening the document. Dropping a live dirty session is prohibited.
pub fn forget(id: &str) {
    SESSIONS.with(|sessions| {
        let mut sessions = sessions.borrow_mut();
        let clean = sessions.get(id).is_some_and(|s| {
            let s = s.borrow();
            s.ready && s.edit_seq == s.pdf_seq && s.error.is_none()
        });
        if clean {
            sessions.remove(id);
        }
    });
}

pub fn request_export(id: &str, path: &str) -> Result<(), String> {
    let session = SESSIONS
        .with(|sessions| sessions.borrow().get(id).cloned())
        .ok_or("열려 있는 필기 세션이 없습니다")?;
    let view = {
        let mut s = session.borrow_mut();
        s.rescuing = true;
        s.view.as_ref().and_then(Weak::load)
    };
    let committed = view
        .as_ref()
        .is_none_or(|v| unsafe { crate::pdf_annotations::commit_pending(v) });
    let mut s = session.borrow_mut();
    s.rescuing = false;
    if !committed {
        return Err("입력 중인 글자를 확정한 뒤 다시 저장해 주세요. 조합 중인 글자는 강제로 확정하지 않습니다.".into());
    }
    s.worker
        .sender
        .try_send(Request::Export(path.into()))
        .map_err(|e| e.to_string())
}
pub fn discard_recovery(id: &str) -> Result<(), String> {
    SESSIONS.with(|sessions| {
        let sessions = sessions.borrow();
        let s = sessions
            .get(id)
            .ok_or("열려 있는 필기 세션이 없습니다")?
            .borrow();
        s.worker
            .sender
            .try_send(Request::Discard)
            .map_err(|e| e.to_string())
    })
}
