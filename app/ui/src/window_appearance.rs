//! Keep native glass vivid without changing real window focus or input routing.
//!
//! AppKit currently exposes no public active-state switch for NSGlassEffectView.
//! This personal macOS build uses a guarded, window-local appearance override;
//! no AppKit class is swizzled. See ADR-020 for the compatibility boundary.
use objc2::{
    msg_send, sel, ClassType, MainThreadMarker, MainThreadOnly,
    rc::Allocated,
    runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel},
};
use objc2_app_kit::NSWindow;
use std::sync::OnceLock;

fn compatible_hook(class: &AnyClass) -> bool {
    class.instance_method(sel!(_hasActiveAppearance)).is_some_and(|method| {
        method.arguments_count() == 2 && method.return_type().to_bytes() == b"B"
    })
}

extern "C-unwind" fn active_appearance(_window: *mut AnyObject, _selector: Sel) -> Bool {
    Bool::YES
}

fn glass_window_class() -> Option<&'static AnyClass> {
    static CLASS: OnceLock<Option<&'static AnyClass>> = OnceLock::new();
    *CLASS.get_or_init(|| {
        AnyClass::get(c"NSGlassEffectView")?;
        let superclass = NSWindow::class();
        if !compatible_hook(superclass) { return None; }
        let mut builder = ClassBuilder::new(c"StologyGlassWindow", superclass)?;
        // The selector's BOOL ABI is checked above before overriding it.
        unsafe {
            builder.add_method(sel!(_hasActiveAppearance),
                active_appearance as extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool);
        }
        Some(builder.register())
    })
}

pub fn alloc(mtm: MainThreadMarker) -> Allocated<NSWindow> {
    match glass_window_class() {
        // This class inherits NSWindow's allocation and initialization contract.
        Some(class) => unsafe { msg_send![class, alloc] },
        None => NSWindow::alloc(mtm),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_appearance_hook_is_rejected() {
        assert!(!compatible_hook(AnyClass::get(c"NSObject").unwrap()));
    }

    #[test]
    fn override_does_not_replace_focus_or_input_methods() {
        let Some(class) = glass_window_class() else { return; };
        let methods = class.instance_methods();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name(), sel!(_hasActiveAppearance));
        assert!(compatible_hook(class));
    }
}
