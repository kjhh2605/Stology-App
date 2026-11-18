//! Optional installed-LibreOffice integration check; no installation performed.
//! cargo run --manifest-path local-app/ui/Cargo.toml --example pptx-preview-smoke -- local-app/bench/biology.pptx
#[path = "../src/pptx_preview.rs"]
mod pptx_preview;
use std::{
    thread,
    time::{Duration, Instant},
};
fn wait(handle: &pptx_preview::Handle) -> Result<String, String> {
    let began = Instant::now();
    loop {
        if let Some(result) = handle.try_result() {
            return result;
        }
        if began.elapsed() > Duration::from_secs(65) {
            handle.cancel();
            return Err("smoke timeout".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}
fn main() {
    assert!(
        pptx_preview::available(),
        "LibreOffice is not installed; integration test not run"
    );
    let mut arguments = std::env::args().skip(1);
    let input = arguments.next().expect("provide a PPTX fixture path");
    // Optional artifact destination for independent rendered-slide inspection.
    let output_pdf = arguments.next();
    let original = std::fs::read(&input).unwrap();
    let data = tempfile::tempdir().unwrap();
    std::env::set_var("STOLOGY_DATA_DIR", data.path());
    let start = Instant::now();
    let handle = pptx_preview::start(&input);
    let pdf = wait(&handle).expect("actual LibreOffice conversion");
    let elapsed = start.elapsed();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    assert!(std::path::Path::new(&pdf).starts_with(data.path().join("preview-cache")));
    drop(handle);
    let repeat = pptx_preview::start(&input);
    assert_eq!(wait(&repeat).unwrap(), pdf);
    drop(repeat);
    assert_eq!(std::fs::read(&input).unwrap(), original, "source changed");
    std::fs::write(&pdf, b"%PDF-1.4\njunkjunkjunkjunk\nstartxref\n9\n%%EOF\n").unwrap();
    let repaired = pptx_preview::start(&input);
    assert_eq!(
        wait(&repaired).expect("corrupt cache repaired by actual conversion"),
        pdf
    );
    drop(repaired);
    assert!(
        std::fs::metadata(&pdf).unwrap().len() > 128,
        "corrupt cache was reused"
    );
    assert_eq!(
        std::fs::read(&input).unwrap(),
        original,
        "cache repair changed source"
    );
    if let Some(output) = output_pdf {
        std::fs::copy(&pdf, &output).expect("copy validation PDF artifact");
        println!("Rendered artifact: {output}");
    }
    let cancelled_data = tempfile::tempdir().unwrap();
    std::env::set_var("STOLOGY_DATA_DIR", cancelled_data.path());
    let cancelled = pptx_preview::start(&input);
    thread::sleep(Duration::from_millis(100));
    let stop = Instant::now();
    cancelled.cancel();
    assert!(wait(&cancelled).is_err());
    drop(cancelled);
    assert!(
        stop.elapsed() < Duration::from_secs(2),
        "cancellation did not reap promptly"
    );
    let cache = cancelled_data.path().join("preview-cache");
    if cache.exists() {
        assert!(
            !std::fs::read_dir(cache)
                .unwrap()
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("conversion-")),
            "temporary profile remains"
        );
    }
    println!("PASS: installed LibreOffice converted PPTX to internal PDF in {:.2}s; cache reuse/corruption repair; original unchanged; cancel/reap/temp-profile cleanup.",elapsed.as_secs_f64());
    println!(
        "NOT TESTED: pixel fidelity of converted slides; embedded animations or complex SmartArt."
    );
}
