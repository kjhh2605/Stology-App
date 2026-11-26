//! Measures what one typed character costs in notes of several sizes.
//! Synthetic notes, no drawing, no input method: compare runs, not absolutes.
//! cargo run --release --manifest-path app/ui/Cargo.toml --example editor-perf
//! `-- loop <bytes> <seconds>` types steadily so `sample <pid>` can profile it.
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
#[path = "../src/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/editor.rs"]
#[allow(dead_code)]
mod editor;
#[path = "../src/pdf_annotations.rs"]
#[allow(dead_code)]
mod pdf_annotations;
#[path = "../src/pdf_save.rs"]
#[allow(dead_code)]
mod pdf_save;
#[path = "../src/pdf_session.rs"]
#[allow(dead_code)]
mod pdf_session;
use objc2::{msg_send, rc::Retained, runtime::AnyObject, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::*;
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString};
use std::time::Instant;

/// A synthetic study note: headings, prose with inline marks, lists, tasks.
/// `extras` adds one code block and one table near the top.
fn note(bytes: usize, extras: bool) -> String {
    let mut out = String::from("# 세포 호흡 정리\n\n");
    if extras {
        out.push_str("```python\ndef atp(glucose):\n    # 해당과정\n    return glucose * 30\n```\n\n");
        out.push_str("| 단계 | 장소 | ATP |\n|---|---|---|\n| 해당과정 | 세포질 | 2 |\n| 시트르산 회로 | 미토콘드리아 기질 | 2 |\n| 산화적 인산화 | 내막 | 26 |\n\n");
    }
    let mut section = 0;
    while out.len() < bytes {
        section += 1;
        out.push_str(&format!("## {section}. 전자 전달계와 화학 삼투\n\n"));
        out.push_str("미토콘드리아 내막의 **전자 전달계**는 NADH와 FADH2에서 받은 전자를 산소까지 옮기며, 그 과정에서 `H+`를 막 사이 공간으로 퍼낸다. 이렇게 생긴 *농도 기울기*가 ATP 합성 효소를 돌린다 → 산화적 인산화.\n\n");
        out.push_str("- 복합체 I: NADH 탈수소 효소\n- 복합체 II: 숙신산 탈수소 효소\n  - FADH2가 여기로 들어온다\n- 복합체 IV: 사이토크롬 c 산화 효소\n\n");
        out.push_str("1. 전자가 이동한다\n2. 양성자가 퍼내진다\n3. ATP가 만들어진다\n\n");
        out.push_str("- [ ] 교재 9장 복습\n- [x] 기출 문제 풀기\n\n");
        out.push_str("> 시험에 자주 나오는 부분: 산소가 최종 전자 수용체라는 점.\n\n");
    }
    out.push_str("마지막 문단에서 이어서 쓴다");
    out
}

fn median(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (samples[samples.len() / 2], samples[(samples.len() * 95 / 100).min(samples.len() - 1)])
}

unsafe fn insert(text: &NSTextView, value: &str) {
    let range: NSRange = msg_send![text, selectedRange];
    let _: () = msg_send![text, insertText:&*NSString::from_str(value), replacementRange:range];
}

unsafe fn clear_undo(text: &NSTextView) {
    let undo: Option<Retained<AnyObject>> = msg_send![text, undoManager];
    if let Some(undo) = undo {
        let _: () = msg_send![&*undo, removeAllActions];
    }
}

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerRegisterFontsForURL(font_url: *const AnyObject, scope: u32, error: *mut *const AnyObject) -> bool;
}

fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().expect("main thread");
        let _app = NSApplication::sharedApplication(mtm);
        // The app registers SUIT from its bundle; without it every Korean
        // character falls back to another font and the numbers mislead.
        for weight in ["Regular", "Medium", "SemiBold", "Bold"] {
            let path = format!("{}/../assets/fonts/SUIT-{weight}.otf", env!("CARGO_MANIFEST_DIR"));
            let url = objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&path));
            assert!(CTFontManagerRegisterFontsForURL(Retained::as_ptr(&url).cast(), 1, std::ptr::null_mut()), "SUIT not registered: {path}");
        }
        assert_eq!(font::regular(16.0).fontName().to_string(), "SUIT-Regular");
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(760.0, 700.0));
        let (scroll, text) = editor::make_editor(frame);
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm), frame, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false,
        );
        window.setContentView(Some(&scroll));
        assert!(window.makeFirstResponder(Some(&text)));

        // A plain TextKit 2 view with the same text: the floor AppKit allows.
        let plain_frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(760.0, 700.0));
        let plain_scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), plain_frame);
        let plain: Retained<NSTextView> = msg_send![NSTextView::alloc(mtm), initUsingTextLayoutManager:true];
        plain.setFrame(plain_frame);
        plain.setVerticallyResizable(true);
        plain.setRichText(false);
        plain_scroll.setDocumentView(Some(&plain));
        let plain_window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm), plain_frame, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false,
        );
        plain_window.setContentView(Some(&plain_scroll));

        let args: Vec<String> = std::env::args().collect();
        if args.get(1).map(String::as_str) == Some("loop") {
            // Steady typing for a sampling profiler.
            let bytes: usize = args.get(2).and_then(|v| v.parse().ok()).unwrap_or(8000);
            let seconds: u64 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(12);
            let source = note(bytes, true);
            text.setString(&NSString::from_str(&source));
            clear_undo(&text);
            text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
            editor::style(&text);
            let start = Instant::now();
            let mut typed = 0;
            while start.elapsed().as_secs() < seconds {
                insert(&text, "가");
                typed += 1;
                if typed % 400 == 0 {
                    text.setString(&NSString::from_str(&source));
                    clear_undo(&text);
                    text.setSelectedRange(NSRange::new(source.encode_utf16().count(), 0));
                }
            }
            println!("typed {typed} characters in {seconds}s");
            return;
        }

        // style: one styling pass. type: a character at the end of the note.
        // mid: a character near the top. plain: an unstyled TextKit 2 view.
        // repeat: the pass the 0.2s timer asks for right after a keystroke.
        println!("{:>8} {:>7} {:>6} | {:>10} {:>10} | {:>10} {:>10} | {:>10} | {:>10} | {:>14}",
            "bytes", "chars", "extras", "style p50", "style p95", "type p50", "type p95", "mid p50", "plain p50", "repeat p50");
        for (bytes, extras) in [(1_600, false), (1_600, true), (8_000, false), (8_000, true), (22_000, true), (60_000, true)] {
            let source = note(bytes, extras);
            let units = source.encode_utf16().count();
            text.setString(&NSString::from_str(&source));
            clear_undo(&text);
            text.setSelectedRange(NSRange::new(units, 0));
            editor::style(&text);

            let mut style = Vec::new();
            for _ in 0..40 {
                let at = Instant::now();
                editor::style(&text);
                style.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            let mut typing = Vec::new();
            let mut repeat = Vec::new();
            for _ in 0..60 {
                let at = Instant::now();
                insert(&text, "가");
                typing.push(at.elapsed().as_secs_f64() * 1000.0);
                // What the 0.2s timer asks for right after a keystroke.
                let at = Instant::now();
                editor::style_if_changed(&text, None);
                repeat.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            // Typing in a paragraph near the top of the note.
            text.setString(&NSString::from_str(&source));
            clear_undo(&text);
            let middle = source[..source.find("미토콘드리아 내막").unwrap()].encode_utf16().count();
            text.setSelectedRange(NSRange::new(middle, 0));
            editor::style(&text);
            let mut mid = Vec::new();
            for _ in 0..60 {
                let at = Instant::now();
                insert(&text, "가");
                mid.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            clear_undo(&text);

            plain.setString(&NSString::from_str(&source));
            clear_undo(&plain);
            plain.setSelectedRange(NSRange::new(units, 0));
            let mut floor = Vec::new();
            for _ in 0..60 {
                let at = Instant::now();
                insert(&plain, "가");
                floor.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            clear_undo(&plain);

            let (style50, style95) = median(style);
            let (type50, type95) = median(typing);
            let (mid50, _) = median(mid);
            let (plain50, _) = median(floor);
            println!("{:>8} {:>7} {:>6} | {:>8.2}ms {:>8.2}ms | {:>8.2}ms {:>8.2}ms | {:>8.2}ms | {:>8.3}ms | {:>12.3}ms",
                source.len(), source.chars().count(), extras, style50, style95, type50, type95, mid50, plain50, median(repeat).0);
        }
    }
}
