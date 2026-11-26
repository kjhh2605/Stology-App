//! SUIT is registered for this process from the app bundle at launch.
//! The fallback keeps standalone development binaries usable without a bundle.
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::NSFont;
use objc2_foundation::{NSString, NSURL};

#[link(name = "CoreText", kind = "framework")]
unsafe extern "C" {
    fn CTFontManagerRegisterFontsForURL(
        font_url: *const AnyObject,
        scope: u32,
        error: *mut *const AnyObject,
    ) -> bool;
}

pub fn register_bundled_fonts() {
    let Some(resources) = std::env::current_exe().ok().and_then(|exe| {
        exe.parent()?
            .parent()
            .map(|contents| contents.join("Resources/fonts"))
    }) else {
        return;
    };
    for weight in ["Regular", "Medium", "SemiBold", "Bold"] {
        let path = resources.join(format!("SUIT-{weight}.otf"));
        if !path.is_file() {
            eprintln!("Bundled SUIT font is missing: {}", path.display());
            continue;
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
        // kCTFontManagerScopeProcess = 1. The font is never installed for other apps.
        let registered = unsafe {
            CTFontManagerRegisterFontsForURL(Retained::as_ptr(&url).cast(), 1, std::ptr::null_mut())
        };
        if !registered {
            eprintln!("Could not register bundled SUIT font: {}", path.display());
        }
    }
}

pub fn regular(size: f64) -> Retained<NSFont> {
    named("SUIT-Regular", size).unwrap_or_else(|| NSFont::systemFontOfSize(size))
}

pub fn bold(size: f64) -> Retained<NSFont> {
    named("SUIT-Bold", size).unwrap_or_else(|| NSFont::boldSystemFontOfSize(size))
}

pub fn weighted(size: f64, weight: f64) -> Retained<NSFont> {
    let name = if weight >= 0.35 {
        "SUIT-Bold"
    } else if weight >= 0.2 {
        "SUIT-SemiBold"
    } else if weight >= 0.1 {
        "SUIT-Medium"
    } else {
        "SUIT-Regular"
    };
    named(name, size).unwrap_or_else(|| NSFont::systemFontOfSize_weight(size, weight))
}

fn named(name: &str, size: f64) -> Option<Retained<NSFont>> {
    NSFont::fontWithName_size(&NSString::from_str(name), size)
}
