//! Native production PDF serialization/journal/session smoke with isolated data.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pdf-annotation-smoke
//! An optional PDF path exercises the same flow with a larger fixture.
#[path = "../src/font.rs"]
mod font;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "../src/pdf_save.rs"]
#[allow(dead_code)]
mod pdf_save;
#[path = "../src/pdf_session.rs"]
#[allow(dead_code)]
mod pdf_session;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
    MainThreadMarker,
};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSArray, NSRect, NSString, NSURL};
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}

fn fixture(path: &Path) {
    let text = "BT /F1 14 Tf 50 740 Td (PDF annotation preservation smoke) Tj ET";
    let objects=vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R /Annots [6 0 R] >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Length {} >>\nstream\n{}\nendstream",text.len(),text),
        "<< /Type /Annot /Subtype /Link /Rect [50 720 180 740] /Border [0 0 0] /A << /S /URI /URI (https://example.org/) >> >>".into(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", i + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    std::fs::write(path, out).unwrap();
}
unsafe fn open(path: &Path) -> Retained<AnyObject> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
    let document: Option<Retained<AnyObject>> = msg_send![allocated,initWithURL:&*url];
    document.expect("independent PDF reopen")
}
unsafe fn foreign(document: &AnyObject) -> Vec<Value> {
    let count: usize = msg_send![document, pageCount];
    let mut result = Vec::new();
    for page_index in 0..count {
        let page: Retained<AnyObject> = msg_send![document,pageAtIndex:page_index];
        let annotations: Retained<NSArray<AnyObject>> = msg_send![&*page, annotations];
        for annotation in annotations.iter() {
            let owned: Option<Retained<AnyObject>> =
                msg_send![&*annotation,valueForAnnotationKey:&*NSString::from_str("/StologyUUID")];
            if owned.is_some() {
                continue;
            }
            let kind: Option<Retained<NSString>> = msg_send![&*annotation, type];
            let contents: Option<Retained<NSString>> = msg_send![&*annotation, contents];
            let url: Option<Retained<NSURL>> = msg_send![&*annotation, URL];
            let bounds: NSRect = msg_send![&*annotation, bounds];
            result.push(json!({"page":page_index,"type":kind.map(|s|s.to_string()),"contents":contents.map(|s|s.to_string()),"url":url.and_then(|u|u.absoluteString()).map(|s|s.to_string()),"bounds":[bounds.origin.x,bounds.origin.y,bounds.size.width,bounds.size.height]}));
        }
    }
    result
}
fn command(before: Value, after: Value) -> Value {
    let v = if after.is_null() { &before } else { &after };
    json!({"id":v["id"],"page":v["page"],"before":before,"after":after})
}
fn model(id: &str, kind: &str) -> Value {
    json!({"schema":1,"id":id,"page":0,"type":kind,"bounds":[50.0,400.0,180.0,60.0],"text":"한글 메모: 받침 수정과 여러 줄\n두 번째 줄","points":[[55.0,410.0],[100.0,440.0],[190.0,420.0]],"quads":[[50.0,460.0],[230.0,460.0],[50.0,400.0],[230.0,400.0]]})
}
fn send(worker: &pdf_save::Worker, seq: u64, c: Value) {
    worker
        .sender
        .send(pdf_save::Request::Edit { seq, command: c })
        .unwrap();
}
fn ready(worker: &pdf_save::Worker) -> (i64, u64) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "worker ready timeout");
        match worker.events.recv_timeout(remaining).unwrap() {
            pdf_save::Event::Ready {
                version,
                pdf_seq,
                commands,
                ..
            } => {
                assert!(commands.is_empty());
                return (version, pdf_seq);
            }
            pdf_save::Event::Failed { message, .. } => panic!("worker start failed: {message}"),
            _ => {}
        }
    }
}
fn saved(worker: &pdf_save::Worker, through: u64) -> i64 {
    let started = Instant::now();
    let deadline = started + Duration::from_secs(30);
    let mut journal = 0;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "save timeout through {through}");
        match worker
            .events
            .recv_timeout(remaining)
            .unwrap_or_else(|e| panic!("save through {through}: {e}"))
        {
            pdf_save::Event::Journal(seq) => journal = journal.max(seq),
            pdf_save::Event::Published {
                version, seq, path, ..
            } => {
                assert!(seq <= journal, "publication before journal ACK");
                if seq >= through {
                    println!(
                        "published seq={seq} wait_ms={} pdf_bytes={}",
                        started.elapsed().as_millis(),
                        std::fs::metadata(path).unwrap().len()
                    );
                    return version;
                }
            }
            pdf_save::Event::Failed { seq, message } => panic!("save failed at {seq}: {message}"),
            _ => {}
        }
    }
}
fn failed(worker: &pdf_save::Worker) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "failure timeout");
        match worker.events.recv_timeout(remaining).unwrap() {
            pdf_save::Event::Failed { message, .. } => return message,
            pdf_save::Event::Published { .. } => panic!("conflicting PDF was published"),
            _ => {}
        }
    }
}
fn resolved(worker: &pdf_save::Worker) -> Option<String> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "resolution timeout");
        match worker.events.recv_timeout(remaining).unwrap() {
            pdf_save::Event::Resolved { export_path } => return export_path,
            pdf_save::Event::Failed { message, .. } => panic!("resolution failed: {message}"),
            _ => {}
        }
    }
}
fn quiet_after_resolution(worker: &pdf_save::Worker) {
    match worker.events.recv_timeout(Duration::from_millis(250)) {
        Err(
            std::sync::mpsc::RecvTimeoutError::Disconnected
            | std::sync::mpsc::RecvTimeoutError::Timeout,
        ) => {}
        Ok(pdf_save::Event::Failed { message, .. }) => panic!("post-resolution error: {message}"),
        Ok(_) => panic!("post-resolution worker activity"),
    }
}
fn no_versions(root: &Path) {
    let p = root.join("versions");
    assert!(
        !p.exists() || std::fs::read_dir(p).unwrap().next().is_none(),
        "annotation save created source snapshot"
    );
}
fn check_journal(root: &Path, id: &str, seq: u64) {
    let state = stology_core::pdf_recovery::open_session(root, id).unwrap();
    assert_eq!(state.pdf_seq, seq);
    assert_eq!(state.journal_seq, seq);
    assert!(
        state.commands.is_empty(),
        "published journal entries not collected"
    );
}
unsafe fn verify(path: &Path, commands: &[Value], count: usize) -> Retained<AnyObject> {
    let document = open(path);
    pdf_annotations::validate_roundtrip(&document, commands).unwrap();
    assert_eq!(pdf_annotations::read_page(&document, 0).len(), count);
    document
}
fn main() {
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            let mtm = MainThreadMarker::new().expect("main thread");
            let _app = NSApplication::sharedApplication(mtm);
            let source = tempfile::tempdir().unwrap();
            let root = tempfile::tempdir().unwrap();
            std::env::set_var("STOLOGY_DATA_DIR", root.path());
            let input = std::env::args()
                .nth(1)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    let p = source.path().join("fixture.pdf");
                    fixture(&p);
                    p
                });
            let workspace = stology_core::dispatch(
                root.path(),
                &json!({"op":"workspace.create","name":"PDF smoke"}),
            )
            .unwrap();
            let material = stology_core::dispatch(
                root.path(),
                &json!({"op":"material.import","workspace_id":workspace["id"],"path":input}),
            )
            .unwrap();
            let id = material["id"].as_str().unwrap();
            let path = std::path::PathBuf::from(material["absolute_path"].as_str().unwrap());
            no_versions(root.path());
            let original = open(&path);
            let pages: usize = msg_send![&*original, pageCount];
            let body: Option<Retained<NSString>> = msg_send![&*original, string];
            let body = body.map(|s| s.to_string());
            let original_foreign = foreign(&original);
            let worker = pdf_save::start(id.into());
            let (initial_version, seq) = ready(&worker);
            assert_eq!(seq, 0);
            let box_model = model("11111111-1111-4111-8111-111111111111", "box");
            let note = model("22222222-2222-4222-8222-222222222222", "note");
            let ink = model("33333333-3333-4333-8333-333333333333", "ink");
            let highlight = model("44444444-4444-4444-8444-444444444444", "highlight");
            let mut expected = Vec::new();
            for (i, m) in [&box_model, &note, &ink, &highlight]
                .into_iter()
                .enumerate()
            {
                let c = command(Value::Null, m.clone());
                send(&worker, i as u64 + 1, c.clone());
                expected.push(c);
            }
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            let first = saved(&worker, 4);
            assert!(first > initial_version);
            verify(&path, &expected, 4);
            check_journal(root.path(), id, 4);
            no_versions(root.path());
            let mut moved = box_model.clone();
            moved["bounds"] = json!([80.0, 200.0, 180.0, 60.0]);
            let move_command = command(box_model.clone(), moved.clone());
            send(&worker, 5, move_command.clone());
            expected.push(move_command);
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            // These edits enter while the previous flush may be serializing. A later
            // final publication must include them, never just the first requested seq.
            let deletion = command(note.clone(), Value::Null);
            send(&worker, 6, deletion.clone());
            expected.push(deletion.clone());
            let undo = pdf_annotations::inverse(&deletion);
            send(&worker, 7, undo.clone());
            expected.push(undo);
            let mut edited = note.clone();
            edited["text"] = json!("저장 중 추가한 한글 메모\n끝");
            let edit = command(note.clone(), edited.clone());
            send(&worker, 8, edit.clone());
            expected.push(edit);
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            let second = saved(&worker, 8);
            assert!(second > first);
            verify(&path, &expected, 4);
            check_journal(root.path(), id, 8);
            let delete_box = command(moved.clone(), Value::Null);
            send(&worker, 9, delete_box.clone());
            expected.push(delete_box);
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            let third = saved(&worker, 9);
            assert!(third > second);
            verify(&path, &expected, 3);
            check_journal(root.path(), id, 9);
            let _ = worker.sender.send(pdf_save::Request::Stop);
            drop(worker);
            let worker = pdf_save::start(id.into());
            let (restarted, seq) = ready(&worker);
            assert_eq!(restarted, third);
            assert_eq!(seq, 9);
            let restore = command(Value::Null, moved.clone());
            send(&worker, 10, restore.clone());
            expected.push(restore);
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            let fourth = saved(&worker, 10);
            assert!(fourth > third);
            verify(&path, &expected, 4);
            check_journal(root.path(), id, 10);
            let _ = worker.sender.send(pdf_save::Request::Stop);
            drop(worker);
            // Test the actual main-thread session's ACK/flush accounting without a GUI
            // view. Native interaction/IME are covered separately by the app QA.
            let ui_document = open(&path);
            let callback = pdf_session::connect(id, ui_document.clone()).unwrap();
            let deadline = Instant::now() + Duration::from_secs(30);
            while pdf_session::has_pending() {
                pdf_session::poll();
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut final_note = edited.clone();
            final_note["text"] = json!("세션 flush 완료: 한글");
            let final_edit = command(edited, final_note);
            pdf_annotations::apply(&ui_document, &final_edit).unwrap();
            callback(final_edit.clone()).unwrap();
            expected.push(final_edit);
            assert!(pdf_session::has_pending());
            pdf_session::request_flush_all();
            assert_eq!(pdf_session::flush_state().unwrap(), false);
            loop {
                pdf_session::poll();
                if pdf_session::flush_state().unwrap() {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            check_journal(root.path(), id, 11);
            no_versions(root.path());
            let final_document = verify(&path, &expected, 4);
            let final_pages: usize = msg_send![&*final_document, pageCount];
            assert_eq!(final_pages, pages);
            assert_eq!(
                foreign(&final_document),
                original_foreign,
                "foreign link/form/annotation changed"
            );
            let final_body: Option<Retained<NSString>> = msg_send![&*final_document, string];
            assert_eq!(final_body.map(|s| s.to_string()), body, "page body changed");
            pdf_session::forget(id);
            drop(callback);
            drop(ui_document);
            let standalone = source.path().join("standalone.pdf");
            std::fs::copy(&path, &standalone).unwrap();
            // External changes stop normal publication. A failed rescue destination
            // must retain both the other app's PDF and the committed journal.
            let worker = pdf_save::start(id.into());
            let (_, seq) = ready(&worker);
            assert_eq!(seq, 11);
            let mut external = std::fs::read(&path).unwrap();
            external.extend_from_slice(b"\n% external application revision\n");
            std::fs::write(&path, &external).unwrap();
            let area = model("55555555-5555-4555-8555-555555555555", "area");
            let rescue_command = command(Value::Null, area);
            send(&worker, 12, rescue_command.clone());
            worker.sender.send(pdf_save::Request::Flush).unwrap();
            let conflict = failed(&worker);
            assert!(
                conflict.contains("변경") || conflict.contains("수정"),
                "{conflict}"
            );
            assert_eq!(std::fs::read(&path).unwrap(), external);
            let _ = worker.sender.send(pdf_save::Request::Stop);
            drop(worker);
            // Restart with a durable journal whose base differs from the external PDF.
            // This must enter rescue-only mode rather than silently rebasing autosave.
            let worker = pdf_save::start(id.into());
            match worker.events.recv_timeout(Duration::from_secs(30)).unwrap() {
                pdf_save::Event::Ready {
                    pdf_seq,
                    journal_seq,
                    commands,
                    ..
                } => {
                    assert_eq!(pdf_seq, 11);
                    assert_eq!(journal_seq, 12);
                    assert_eq!(commands.len(), 1);
                }
                pdf_save::Event::Failed { message, .. } => {
                    panic!("rescue reopen did not become available: {message}")
                }
                _ => panic!("unexpected rescue startup event"),
            }
            let restarted_conflict = failed(&worker);
            assert!(restarted_conflict.contains("충돌"));
            assert_eq!(std::fs::read(&path).unwrap(), external);
            let occupied = source.path().join("occupied.pdf");
            std::fs::write(&occupied, b"must preserve existing export destination").unwrap();
            worker
                .sender
                .send(pdf_save::Request::Export(occupied.clone()))
                .unwrap();
            let collision = failed(&worker);
            assert!(!collision.is_empty());
            assert_eq!(
                std::fs::read(&occupied).unwrap(),
                b"must preserve existing export destination"
            );
            assert_eq!(std::fs::read(&path).unwrap(), external);
            let rescue = source.path().join("rescued.pdf");
            worker
                .sender
                .send(pdf_save::Request::Export(rescue.clone()))
                .unwrap();
            let _ = worker.sender.send(pdf_save::Request::Flush);
            assert_eq!(resolved(&worker).as_deref(), rescue.to_str());
            quiet_after_resolution(&worker);
            assert_eq!(std::fs::read(&path).unwrap(), external);
            let mut rescued_expected = expected.clone();
            rescued_expected.push(rescue_command);
            verify(&rescue, &rescued_expected, 5);
            let _ = worker.sender.send(pdf_save::Request::Stop);
            drop(worker);
            // Reopening after explicit rescue leaves no pending journal for the source.
            let reconciled = stology_core::pdf_recovery::open_session(root.path(), id).unwrap();
            assert!(reconciled.commands.is_empty());
            no_versions(root.path());
            // A PDFDocument initialization failure still accepts explicit discard. A
            // flush queued behind discard must never recreate commands or emit errors.
            std::fs::write(&path, b"invalid PDF contents for initialization failure").unwrap();
            let worker = pdf_save::start(id.into());
            let invalid = failed(&worker);
            assert!(!invalid.is_empty());
            worker.sender.send(pdf_save::Request::Discard).unwrap();
            let _ = worker.sender.send(pdf_save::Request::Flush);
            assert!(resolved(&worker).is_none());
            quiet_after_resolution(&worker);
            drop(worker);
            drop(root);
            verify(&standalone, &expected, 4);
            verify(&rescue, &rescued_expected, 5);
            println!("PASS production PDF worker + journal + main-thread session: 12 commands, five annotation types, Korean multiline memo, edit/delete/undo, queued newer edits, worker restart, standalone PDF, body/pages/foreign annotations preserved, journal GC, no versions snapshots, external conflict/restart rescue, export collision, rescue copy, initialization discard, no post-resolution retries");
        });
    }
}
