//! Dedicated durable-command and PDF serialization workers. No PDFKit object is
//! shared with the main thread; only JSON annotation commands cross queues.
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
};
use objc2_foundation::{NSString, NSURL};
use serde_json::Value;
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{mpsc, Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub enum Request {
    Edit { seq: u64, command: Value },
    Flush,
    Export(PathBuf),
    Discard,
    Stop,
}
pub enum Event {
    Ready {
        version: i64,
        hash: String,
        path: String,
        pdf_seq: u64,
        journal_seq: u64,
        commands: Vec<Value>,
    },
    Journal(u64),
    Published {
        version: i64,
        hash: String,
        path: String,
        seq: u64,
    },
    Failed {
        seq: u64,
        message: String,
    },
    Resolved {
        export_path: Option<String>,
    },
}
pub struct Worker {
    pub sender: mpsc::SyncSender<Request>,
    pub events: mpsc::Receiver<Event>,
}
struct Pending {
    commands: VecDeque<(u64, Value)>,
    last_edit: Instant,
    last_save: Instant,
    force: bool,
    stop: bool,
    failed: bool,
    export: Option<PathBuf>,
    discard: bool,
    terminal: bool,
    resolution: Option<mpsc::Sender<bool>>,
}

pub fn start(id: String) -> Worker {
    let (sender, input) = mpsc::sync_channel(256);
    let (events_tx, events) = mpsc::channel();
    std::thread::Builder::new().name("pdf-journal".into()).spawn(move || {
        let result = (|| -> Result<(), String> {
            let root = stology_core::data_root().map_err(|e|e.to_string())?;
            let (session,startup_error) = match stology_core::pdf_recovery::open_session(&root, &id) {
                Ok(session)=>(session,None),
                Err(e)=>match stology_core::pdf_recovery::rescue_session(&root,&id){
                    Ok(session)=>(session,Some(format!("PDF 복구 충돌: {e}. 별도 사본 저장은 현재 PDF 위에 복구 필기를 적용합니다."))),
                    Err(_)=>{terminal_commands(&root,&id,&input,&events_tx,e.to_string());return Ok(())}
                }
            };
            let pending = Arc::new((Mutex::new(Pending {
                commands: session.commands.iter().map(|e|(e.seq,e.command.clone())).collect(),
                last_edit: Instant::now(), last_save: Instant::now(), force: !session.commands.is_empty(), stop: false, failed: startup_error.is_some(), export: None, discard: false, terminal: false, resolution: None,
            }),Condvar::new()));
            let worker_pending = pending.clone();
            let save_events = events_tx.clone();
            let save_root = root.clone(); let save_id = id.clone();
            std::thread::Builder::new().name("pdf-save".into()).spawn(move || unsafe {
                // A thread-local autorelease pool encloses each serializer's
                // lifetime, and each save adds a shorter pool below.
                objc2::rc::autoreleasepool(|_| {
                    if let Err(e)=save_loop(save_root,save_id,session,worker_pending.clone(),save_events.clone(),startup_error) {
                        let (lock,cv)=&*worker_pending;
                        if let Ok(mut p)=lock.lock(){p.terminal=true;if let Some(reply)=p.resolution.take(){let _=reply.send(false);}cv.notify_one();}
                        let _=save_events.send(Event::Failed { seq: 0, message: e });
                    }
                });
            }).map_err(|e|e.to_string())?;
            let mut retry=VecDeque::new();
            let mut journal_failed=false;
            while let Ok(request)=input.recv() {
                let flush=matches!(request,Request::Flush);
                match request {
                    Request::Edit{seq,command} => retry.push_back((seq,command)),
                    Request::Flush=>journal_failed=false,
                    request @ (Request::Export(_) | Request::Discard)=>{
                        let (reply,answer)=mpsc::channel();
                        let (lock,cv)=&*pending;let mut p=lock.lock().map_err(|_|"PDF 저장 큐 오류")?;
                        if p.terminal {
                            drop(p);
                            if matches!(request,Request::Discard) {
                                match stology_core::pdf_recovery::discard_recovery(&root,&id){Ok(())=>{let _=events_tx.send(Event::Resolved{export_path:None});break}Err(e)=>{let _=events_tx.send(Event::Failed{seq:0,message:e.to_string()});}}
                            }else{let _=events_tx.send(Event::Failed{seq:0,message:"PDF를 열지 못해 사본을 작성할 수 없습니다. 원본 PDF를 복원한 뒤 다시 열거나 복구 필기를 버려 주세요.".into()});}
                            continue;
                        }
                        for (seq,command) in &retry {if !p.commands.iter().any(|(n,_)|n==seq){p.commands.push_back((*seq,command.clone()));}}
                        match request{Request::Export(path)=>p.export=Some(path),Request::Discard=>p.discard=true,_=>unreachable!()}
                        p.resolution=Some(reply);cv.notify_one();drop(p);
                        // Freeze append/retry processing until the serializer
                        // finishes resolving. Queued Flush cannot resurrect a
                        // discarded journal after the Resolved event.
                        if answer.recv().unwrap_or(false){break}
                        continue;
                    }
                    Request::Stop=>break,
                }
                while !journal_failed {
                    let Some((seq,command))=retry.front() else {break};
                    match stology_core::pdf_recovery::append(&root,&id,*seq,command) {
                        Ok(ack)=>{
                            let (seq,command)=retry.pop_front().unwrap();
                            let (lock,cv)=&*pending;
                            let mut p=lock.lock().map_err(|_|"PDF 저장 큐 오류")?;
                            p.commands.push_back((seq,command));p.last_edit=Instant::now();
                            let _=events_tx.send(Event::Journal(ack));cv.notify_one();
                        }
                        Err(e)=>{journal_failed=true;let _=events_tx.send(Event::Failed { seq: *seq, message: format!("복구 기록 실패: {e}. 저장을 다시 요청하면 재시도합니다.") });}
                    }
                }
                if flush && !journal_failed {let (lock,cv)=&*pending;let mut p=lock.lock().map_err(|_|"PDF 저장 큐 오류")?;p.force=true;p.failed=false;cv.notify_one();}
            }
            let (lock,cv)=&*pending;let mut p=lock.lock().map_err(|_|"PDF 저장 큐 오류")?;p.stop=true;cv.notify_one();
            Ok(())
        })();
        if let Err(e)=result {let _=events_tx.send(Event::Failed { seq: 0, message: e });}
    }).expect("PDF journal worker");
    Worker { sender, events }
}
fn terminal_commands(
    root: &std::path::Path,
    id: &str,
    input: &mpsc::Receiver<Request>,
    events: &mpsc::Sender<Event>,
    message: String,
) {
    let _ = events.send(Event::Failed { seq: 0, message });
    while let Ok(request) = input.recv() {
        match request {
            Request::Discard => match stology_core::pdf_recovery::discard_recovery(root, id) {
                Ok(()) => {
                    let _ = events.send(Event::Resolved { export_path: None });
                    return;
                }
                Err(e) => {
                    let _ = events.send(Event::Failed {
                        seq: 0,
                        message: e.to_string(),
                    });
                }
            },
            Request::Export(_) => {
                let _=events.send(Event::Failed{seq:0,message:"복구할 PDF를 열 수 없습니다. 원본 파일을 복원해 다시 열거나 복구 필기를 버려 주세요.".into()});
            }
            Request::Stop => return,
            _ => {}
        }
    }
}
unsafe fn open(path: &std::path::Path) -> Result<Retained<AnyObject>, String> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
    let doc: Option<Retained<AnyObject>> = msg_send![allocated,initWithURL:&*url];
    let doc = doc.ok_or("PDF 저장 작업자가 파일을 열 수 없습니다")?;
    let locked: bool = msg_send![&*doc, isLocked];
    if locked {
        return Err("암호로 잠긴 PDF입니다".into());
    }
    Ok(doc)
}
unsafe fn save_loop(
    root: PathBuf,
    id: String,
    session: stology_core::pdf_recovery::Session,
    pending: Arc<(Mutex<Pending>, Condvar)>,
    events: mpsc::Sender<Event>,
    startup_error: Option<String>,
) -> Result<(), String> {
    let mut path = session.path;
    let mut version = session.version;
    let mut hash = session.hash;
    let mut document = open(&path)?;
    let pages: usize = msg_send![&*document, pageCount];
    events
        .send(Event::Ready {
            version,
            hash: hash.clone(),
            path: path.to_string_lossy().into_owned(),
            pdf_seq: session.pdf_seq,
            journal_seq: session.journal_seq,
            commands: session.commands.iter().map(|e| e.command.clone()).collect(),
        })
        .map_err(|e| e.to_string())?;
    if let Some(message) = startup_error {
        let _ = events.send(Event::Failed {
            seq: session.journal_seq,
            message,
        });
    }
    loop {
        let (batch, export, discard, resolution) = {
            let (lock, cv) = &*pending;
            let mut p = lock.lock().map_err(|_| "PDF 저장 큐 오류")?;
            loop {
                if p.stop {
                    return Ok(());
                }
                let due = p.force
                    || p.last_edit.elapsed() >= Duration::from_millis(500)
                    || p.last_save.elapsed() >= Duration::from_secs(2);
                if p.export.is_some() || p.discard || (!p.commands.is_empty() && due && !p.failed) {
                    break;
                }
                p = cv
                    .wait_timeout(p, Duration::from_millis(50))
                    .map_err(|_| "PDF 저장 대기 오류")?
                    .0;
            }
            p.force = false;
            let commands = p.commands.iter().cloned().collect::<Vec<_>>();
            let export = p.export.take();
            let discard = p.discard;
            p.discard = false;
            let resolution = p.resolution.take();
            (commands, export, discard, resolution)
        };
        let through = batch.last().map(|v| v.0).unwrap_or(session.pdf_seq);
        if discard {
            match stology_core::pdf_recovery::discard_recovery(&root, &id) {
                Ok(()) => {
                    let _ = events.send(Event::Resolved { export_path: None });
                    if let Some(reply) = resolution {
                        let _ = reply.send(true);
                    }
                    return Ok(());
                }
                Err(e) => {
                    let _ = events.send(Event::Failed {
                        seq: through,
                        message: format!("필기 복구 기록을 버리지 못했습니다: {e}"),
                    });
                    if let Some(reply) = resolution {
                        let _ = reply.send(false);
                    }
                    continue;
                }
            }
        }
        if let Some(destination) = export {
            let result = objc2::rc::autoreleasepool(|_| -> Result<(), String> {
                for (_, command) in &batch {
                    crate::pdf_annotations::apply(&document, command)?;
                }
                let parent = destination.parent().ok_or("저장 폴더를 찾을 수 없습니다")?;
                let temp = tempfile::Builder::new()
                    .prefix(".stology-export-")
                    .suffix(".pdf")
                    .tempfile_in(parent)
                    .map_err(|e| e.to_string())?;
                let url =
                    NSURL::fileURLWithPath(&NSString::from_str(&temp.path().to_string_lossy()));
                let ok: bool = msg_send![&*document,writeToURL:&*url];
                if !ok {
                    return Err("필기 사본 PDF 작성에 실패했습니다".into());
                }
                let verified = open(temp.path())?;
                let count: usize = msg_send![&*verified, pageCount];
                if count != pages {
                    return Err("필기 사본 페이지 검증에 실패했습니다".into());
                }
                crate::pdf_annotations::validate_roundtrip(
                    &verified,
                    &batch.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>(),
                )?;
                temp.as_file().sync_all().map_err(|e| e.to_string())?;
                // Hard-link publication is atomic and refuses an existing name.
                std::fs::hard_link(temp.path(), &destination).map_err(|e| {
                    format!(
                        "새 PDF를 게시하지 못했습니다. 기존 파일과 다른 이름을 선택해 주세요: {e}"
                    )
                })?;
                std::fs::File::open(parent)
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
                stology_core::pdf_recovery::discard_recovery(&root, &id)
                    .map_err(|e| format!("사본은 저장됐지만 복구 기록 정리에 실패했습니다: {e}"))?;
                Ok(())
            });
            match result {
                Ok(()) => {
                    let _ = events.send(Event::Resolved {
                        export_path: Some(destination.to_string_lossy().into_owned()),
                    });
                    if let Some(reply) = resolution {
                        let _ = reply.send(true);
                    }
                    return Ok(());
                }
                Err(e) => {
                    let (lock, _) = &*pending;
                    lock.lock().map_err(|_| "PDF 저장 큐 오류")?.failed = true;
                    let _ = events.send(Event::Failed {
                        seq: through,
                        message: e,
                    });
                    if let Some(reply) = resolution {
                        let _ = reply.send(false);
                    }
                    continue;
                }
            }
        }
        let saved = objc2::rc::autoreleasepool(|_| -> Result<_, String> {
            for (_, command) in &batch {
                crate::pdf_annotations::apply(&document, command)?;
            }
            let parent = path.parent().ok_or("PDF 폴더를 찾을 수 없습니다")?;
            let temp = tempfile::Builder::new()
                .prefix(".stology-pdf-")
                .suffix(".pdf")
                .tempfile_in(parent)
                .map_err(|e| e.to_string())?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(&temp.path().to_string_lossy()));
            let ok: bool = msg_send![&*document,writeToURL:&*url];
            if !ok {
                return Err(
                    "PDF 임시 파일을 작성하지 못했습니다. 여유 공간과 쓰기 권한을 확인해 주세요."
                        .into(),
                );
            }
            let verified = open(temp.path())?;
            let count: usize = msg_send![&*verified, pageCount];
            if count != pages {
                return Err("저장한 PDF 페이지 수 검증에 실패했습니다".into());
            }
            crate::pdf_annotations::validate_roundtrip(
                &verified,
                &batch.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>(),
            )?;
            let published = stology_core::pdf_recovery::publish(
                &root,
                &id,
                version,
                &hash,
                through,
                temp.path(),
            )
            .map_err(|e| e.to_string())?;
            Ok((published, verified))
        });
        match saved {
            Ok((saved, verified)) => {
                version = saved.version;
                hash = saved.hash;
                path = saved.path;
                // Publication renamed the temporary file. Retain its verified
                // object only until reopening the canonical path succeeds.
                document = open(&path).unwrap_or(verified);
                let (lock, _) = &*pending;
                let mut p = lock.lock().map_err(|_| "PDF 저장 큐 오류")?;
                while p.commands.front().is_some_and(|(seq, _)| *seq <= through) {
                    p.commands.pop_front();
                }
                p.last_save = Instant::now();
                let _ = events.send(Event::Published {
                    version,
                    hash: hash.clone(),
                    path: path.to_string_lossy().into_owned(),
                    seq: saved.pdf_seq,
                });
            }
            Err(e) => {
                let (lock, _) = &*pending;
                let mut p = lock.lock().map_err(|_| "PDF 저장 큐 오류")?;
                p.failed = true;
                let _ = events.send(Event::Failed {
                    seq: through,
                    message: format!("PDF 저장 실패: {e}"),
                });
            }
        }
    }
}
