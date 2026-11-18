//! Replay a preserved app QA journal using the production serializer. The source
//! data directory is read-only; SQLite backup and documents are copied to a temp
//! root before any core API runs. No personal data is used or modified.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pdf-recovery-smoke -- /tmp/stology-pdf-ui-state.json
#[path = "../src/font.rs"]
mod font;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "../src/pdf_save.rs"]
#[allow(dead_code)]
mod pdf_save;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
    MainThreadMarker,
};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSString, NSURL};
use serde_json::Value;
use std::{
    path::Path,
    time::{Duration, Instant},
};
#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}
unsafe fn open(path: &Path) -> Retained<AnyObject> {
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
    let document: Option<Retained<AnyObject>> = msg_send![allocated,initWithURL:&*url];
    document.expect("independent reopened recovered PDF")
}
fn main() {
    unsafe {
        objc2::rc::autoreleasepool(|_| {
            let mtm = MainThreadMarker::new().expect("main thread");
            let _app = NSApplication::sharedApplication(mtm);
            let state_path = std::env::args()
                .nth(1)
                .expect("provide preserved QA state JSON");
            let state: Value = serde_json::from_slice(&std::fs::read(state_path).unwrap()).unwrap();
            let source = Path::new(state["data"].as_str().unwrap());
            let id = state["pdf"]["id"].as_str().unwrap();
            let source_pdf = Path::new(state["pdf"]["absolute_path"].as_str().unwrap());
            let original_pdf = std::fs::read(source_pdf).unwrap();
            let root = tempfile::tempdir().unwrap();
            let copied = std::process::Command::new("python3")
                .arg("-c")
                .arg(
                    r#"
import pathlib, shutil, sqlite3, sys
source, target = map(pathlib.Path, sys.argv[1:])
for entry in source.iterdir():
    if '.sqlite' in entry.name:
        continue
    if entry.is_dir():
        shutil.copytree(entry, target / entry.name)
    elif entry.is_file():
        shutil.copy2(entry, target / entry.name)
for entry in source.glob('*.sqlite'):
    src=sqlite3.connect(entry.as_uri()+'?mode=ro', uri=True)
    dst=sqlite3.connect(target / entry.name)
    src.backup(dst)
    dst.close(); src.close()
"#,
                )
                .arg(source)
                .arg(root.path())
                .status()
                .unwrap();
            assert!(copied.success());
            std::env::set_var("STOLOGY_DATA_DIR", root.path());
            let session = stology_core::pdf_recovery::open_session(root.path(), id).unwrap();
            assert_eq!(session.pdf_seq, 5, "expected observed app QA baseline");
            assert_eq!(session.journal_seq, 8);
            assert_eq!(session.commands.len(), 3);
            let commands: Vec<Value> = session.commands.iter().map(|c| c.command.clone()).collect();
            let latest = commands.last().unwrap();
            let moved_x = latest["after"]["bounds"][0].as_f64().unwrap();
            assert_eq!(moved_x.to_bits(), 193.33170656813786_f64.to_bits());
            let baseline_bytes = std::fs::read(&session.path).unwrap();
            assert_eq!(baseline_bytes, original_pdf);
            let worker = pdf_save::start(id.into());
            let started = Instant::now();
            let deadline = started + Duration::from_secs(30);
            let mut saw_ready = false;
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                assert!(!remaining.is_zero(), "recovery publish timeout");
                match worker.events.recv_timeout(remaining).unwrap() {
                    pdf_save::Event::Ready {
                        pdf_seq,
                        journal_seq,
                        commands: replayed,
                        ..
                    } => {
                        assert_eq!(pdf_seq, 5);
                        assert_eq!(journal_seq, 8);
                        assert_eq!(replayed, commands);
                        saw_ready = true;
                    }
                    pdf_save::Event::Published { seq, .. } => {
                        assert!(saw_ready);
                        assert_eq!(seq, 8);
                        break;
                    }
                    pdf_save::Event::Failed { seq, message } => {
                        panic!("actual QA recovery failed at {seq}: {message}")
                    }
                    _ => {}
                }
            }
            let reopened = open(&session.path);
            pdf_annotations::validate_roundtrip(&reopened, &commands).unwrap();
            let model = pdf_annotations::read_page(&reopened, 0)
                .into_iter()
                .find(|v| v["id"] == latest["id"])
                .unwrap();
            assert_eq!(
                model["bounds"][0].as_f64().unwrap().to_bits(),
                moved_x.to_bits(),
                "PDF payload lost one ULP"
            );
            let clean = stology_core::pdf_recovery::open_session(root.path(), id).unwrap();
            assert_eq!(clean.pdf_seq, 8);
            assert_eq!(clean.journal_seq, 8);
            assert!(clean.commands.is_empty());
            let _ = worker.sender.send(pdf_save::Request::Stop);
            drop(worker);
            assert_eq!(
                std::fs::read(source_pdf).unwrap(),
                original_pdf,
                "read-only QA source changed"
            );
            println!("PASS production QA recovery: seq 5→8; three journal commands replayed; PDF x={moved_x:.17} bit-exact; journal empty; source PDF unchanged; elapsed_ms={}",started.elapsed().as_millis());
        });
    }
}
