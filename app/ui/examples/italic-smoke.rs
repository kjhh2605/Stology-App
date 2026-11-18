//! Offscreen AppKit regression: SUIT emphasis, nested weight, and style removal.
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
#[path = "../src/font.rs"]
#[allow(dead_code)]
mod font;
#[path = "../src/editor.rs"]
#[allow(dead_code, unused_imports)]
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
use objc2_foundation::{NSDictionary, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL};

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerRegisterFontsForURL(
        font_url: *const AnyObject,
        scope: u32,
        error: *mut *const AnyObject,
    ) -> bool;
}

unsafe fn slant(storage: &NSTextStorage, index: usize) -> f64 {
    let font: Retained<NSFont> = msg_send![storage,
        attribute: NSFontAttributeName, atIndex: index,
        effectiveRange: std::ptr::null_mut::<NSRange>()];
    { let matrix = std::slice::from_raw_parts(font.matrix().as_ptr(), 6); matrix[2] / matrix[0] }
}

fn main() {
    unsafe {
        let mtm = MainThreadMarker::new().unwrap();
        let _app = NSApplication::sharedApplication(mtm);
        for weight in ["Regular", "Bold"] {
            let path = format!("{}/../assets/fonts/SUIT-{weight}.otf", env!("CARGO_MANIFEST_DIR"));
            let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
            assert!(CTFontManagerRegisterFontsForURL(
                Retained::as_ptr(&url).cast(), 1, std::ptr::null_mut()
            ));
        }
        let manager = NSFontManager::sharedFontManager(mtm);
        let suit = font::regular(16.0);
        assert_eq!(suit.fontName().to_string(), "SUIT-Regular");
        assert!(!manager.traitsOfFont(&manager.convertFont_toHaveTrait(
            &suit, NSFontTraitMask::ItalicFontMask
        )).contains(NSFontTraitMask::ItalicFontMask), "fixture must lack a real italic face");
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0), NSSize::new(700.0, 500.0)
        );
        let (scroll, text) = editor::make_editor(frame);
        for (source, start, bold) in [
            ("*한글 Memory* 평문", 1, false),
            ("***한글 Memory*** 평문", 3, true),
            ("**굵게 *한글 Memory* 유지** 평문", 6, true),
            ("*기울임 **한글 Memory** 유지* 평문", 7, true),
        ] {
            text.setString(&NSString::from_str(source));
            let selected = NSRange::new(start, 2);
            text.setSelectedRange(selected);
            editor::style(&text);
            let storage = text.textStorage().unwrap();
            assert_eq!(text.string().to_string(), source);
            let actual: NSRange = msg_send![&*text, selectedRange];
            assert_eq!(actual, selected, "style changed selection");
            for index in start..start + "한글 Memory".encode_utf16().count() {
                assert_eq!(slant(&storage, index), 0.2, "upright glyph at {index} in {source}");
                let styled: Retained<NSFont> = msg_send![&*storage,
                    attribute:NSFontAttributeName, atIndex:index,
                    effectiveRange:std::ptr::null_mut::<NSRange>()];
                assert_eq!(styled.pointSize(), 16.0, "font size changed by transform");
                assert_eq!(manager.traitsOfFont(&styled).contains(NSFontTraitMask::BoldFontMask), bold,
                    "weight lost at {index} in {source}");
            }
            assert_eq!(slant(&storage, source.encode_utf16().count() - 1), 0.0);
            // Removing emphasis must clear the old slant, including on a second styling pass.
            text.setString(&NSString::from_str("한글 Memory 평문"));
            editor::style(&text);
            editor::style(&text);
            for index in 0..text.string().length() {
                assert_eq!(slant(&storage, index), 0.0, "stale italic at {index}");
            }
        }
        if let Ok(output) = std::env::var("STOLOGY_ITALIC_CAPTURE") {
            let window = NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm), frame, NSWindowStyleMask::Titled,
                NSBackingStoreType::Buffered, false,
            );
            window.setContentView(Some(&scroll));
            text.setString(&NSString::from_str("기억과 학습 Memory\n\n*기억과 학습 Memory*\n\n**기억과 학습 Memory**\n\n***기억과 학습 Memory***"));
            editor::style(&text);
            scroll.layoutSubtreeIfNeeded();
            let bitmap = scroll.bitmapImageRepForCachingDisplayInRect(scroll.bounds()).unwrap();
            scroll.cacheDisplayInRect_toBitmapImageRep(scroll.bounds(), &bitmap);
            let png = bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()).unwrap();
            assert!(png.writeToFile_atomically(&NSString::from_str(&output), true));
        }
        println!("italic-smoke PASS: real SUIT fallback, Korean/Latin, nested bold, size/source/selection preservation, clearing emphasis");
    }
}
