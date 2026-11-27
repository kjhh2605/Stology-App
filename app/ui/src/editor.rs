//! TextKit 2 Markdown editor and in-window macOS source previews.
//!
//! Styling changes attributes only. It never replaces the string, selects a new
//! range, or touches marked text, so Korean composition and native undo survive.
use objc2::{
    class, define_class, msg_send,
    rc::{Allocated, Retained, Weak},
    runtime::AnyObject,
    sel, AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::{NSAffineTransform, NSAffineTransformStruct, NSDictionary, NSPoint, NSRange, NSRect, NSSize, NSString, NSURL};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::{
    cell::RefCell,
    ops::Range,
    path::{Path, PathBuf},
};

#[path = "code_highlight.rs"]
mod code_highlight;
use code_highlight::CODE_LANGUAGES;
#[path = "image_edit.rs"]
mod image_edit;
#[path = "markdown_input.rs"]
mod markdown_input;
#[path = "markdown_shortcuts.rs"]
mod markdown_shortcuts;
#[path = "note_blocks.rs"]
mod note_blocks;
#[path = "note_context.rs"]
pub mod note_context;
use note_blocks::Placement;
#[path = "smart_edit.rs"]
mod smart_edit;
#[path = "diagram_paint.rs"]
pub mod diagram_paint;
pub use image_edit::{set_image_importer, set_diagram_opener};
use image_edit::{ImageState, InlineImage};

// Approximate row pitch used for block hit testing.
const NOTE_LINE_HEIGHT: f64 = 26.0;
const NOTE_PARAGRAPH_SPACING: f64 = 10.0;
const NOTE_LIST_SPACING: f64 = 4.0;

unsafe fn input_command(text: &NSTextView, command: markdown_input::Command) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return false;
    }
    let selection: NSRange = msg_send![text, selectedRange];
    if selection.length != 0 {
        if !matches!(command, markdown_input::Command::Tab | markdown_input::Command::BackTab) {
            return false;
        }
        let source = text.string().to_string();
        let start = byte_for_utf16(&source, selection.location);
        let end = byte_for_utf16(&source, selection.location.saturating_add(selection.length));
        let Some(edit) = markdown_input::indent_selected_numbers(
            &source, start..end, command == markdown_input::Command::BackTab,
        ) else { return false };
        if text.selectedRanges().count() == 1 && (!edit.range.is_empty() || !edit.replacement.is_empty()) {
            apply_source_edit(text, edit);
        }
        return true;
    }
    let source = text.string().to_string();
    let at = byte_for_utf16(&source, selection.location);
    let Some(edit) = markdown_input::command(&source, at, command) else {
        return false;
    };
    let mut result = source.clone();
    result.replace_range(edit.range.clone(), &edit.replacement);
    let _: () = msg_send![text, insertText:&*NSString::from_str(&edit.replacement), replacementRange:utf16_range(&source, edit.range)];
    text.setSelectedRange(NSRange::new(result[..edit.caret].encode_utf16().count(), 0));
    let is_native: bool = msg_send![text, isKindOfClass:MarkdownEditor::class()];
    if let Some(native) = is_native.then(|| &*(text as *const NSTextView as *const MarkdownEditor))
    {
        let base = native.ivars().base_directory.borrow().clone();
        style_with_base(text, base.as_deref());
    }
    true
}

#[derive(Debug)]
struct CodeBlock {
    bytes: Range<usize>,
    open: Range<usize>,
    close: Range<usize>,
    language: String,
    fence: String,
}
fn code_blocks(source: &str) -> Vec<CodeBlock> {
    Parser::new(source)
        .into_offset_iter()
        .filter_map(|(event, bytes)| {
            let Event::Start(Tag::CodeBlock(pulldown_cmark::CodeBlockKind::Fenced(info))) = event
            else {
                return None;
            };
            let raw = &source[bytes.clone()];
            // An unfinished fence line is ordinary input until Enter commits it.
            let first_end = raw.find('\n')? + 1;
            let indent = raw.bytes().take_while(|b| *b == b' ').count();
            if indent > 3 {
                return None;
            }
            let fence: String = raw[indent..]
                .chars()
                .take_while(|c| *c == '`' || *c == '~')
                .collect();
            if fence.len() < 3 {
                return None;
            }
            let trimmed = raw.trim_end_matches(['\r', '\n']);
            let last = trimmed.rfind('\n').map(|i| i + 1).unwrap_or(trimmed.len());
            let closing = &trimmed[last..];
            let is_closed = closing.trim().len() >= fence.len()
                && closing
                    .trim()
                    .chars()
                    .all(|c| Some(c) == fence.chars().next());
            let close = if is_closed {
                bytes.start + last..bytes.end
            } else {
                bytes.end..bytes.end
            };
            let language = code_highlight::effective_language(
                &info,
                &source[bytes.start + first_end..close.start],
            );
            Some(CodeBlock {
                open: bytes.start..bytes.start + first_end,
                close,
                language,
                fence: format!("{}{}", " ".repeat(indent), fence),
                bytes,
            })
        })
        .collect()
}

/// Select editable code only: delimiters and the newline before a closing fence
/// belong to the Markdown structure, not to the code the user can replace.
fn code_selection(source: &str, selection: NSRange) -> Option<NSRange> {
    code_blocks(source).into_iter().find_map(|block| {
        let start = block.open.end;
        let mut end = block.close.start;
        if !block.close.is_empty() && end > start && source[..end].ends_with('\n') {
            end -= 1;
            if end > start && source[..end].ends_with('\r') {
                end -= 1;
            }
        }
        let body = utf16_range(source, start..end);
        // Native line selection may include its terminating newline. It still
        // belongs to this block; Cmd+A trims that structural newline away.
        let content_line_end = source[..block.close.start].encode_utf16().count();
        (body.location <= selection.location
            && selection.location.checked_add(selection.length)? <= content_line_end)
            .then_some(body)
    })
}

define_class!(
    /// Transparent layer above the note's selection highlight that draws list
    /// numbers, bullets and quote bars from the editor's last styling pass.
    #[unsafe(super(NSView, NSResponder))]
    #[name = "StologyMarkOverlay"]
    #[thread_kind = MainThreadOnly]
    struct MarkOverlay;
    impl MarkOverlay {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { std::ptr::null_mut() }
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) { unsafe {
            let Some(editor) = self.superview().and_then(|v| v.downcast::<MarkdownEditor>().ok()) else { return };
            // Empty lines at the end of the note stay selected (Cmd+A still
            // deletes them) but are not shown as highlighted below the content.
            let selection: NSRange = msg_send![&*editor, selectedRange];
            if selection.length > 0 {
                let source = editor.string().to_string();
                let tail = trailing_blank_start(&source);
                let tail_utf16 = utf16_offset(&source, tail);
                if tail < source.len() && selection.location < tail_utf16 && selection.location + selection.length > tail_utf16 {
                    if let Some(frame) = layout_fragment_rect(&editor, tail_utf16) {
                        let bounds = self.bounds();
                        editor.backgroundColor().setFill();
                        NSBezierPath::bezierPathWithRect(NSRect::new(
                            NSPoint::new(0.0, frame.origin.y),
                            NSSize::new(bounds.size.width, (bounds.size.height - frame.origin.y).max(0.0)),
                        )).fill();
                    }
                }
            }
            NSColor::separatorColor().setFill();
            for frame in editor.ivars().rule_bars.borrow().iter() {
                NSBezierPath::bezierPathWithRect(*frame).fill();
            }
            let color = NSColor::secondaryLabelColor();
            let mono = NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0);
            let attrs = NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*mono as &AnyObject, &*color as &AnyObject]);
            for (number, point) in editor.ivars().line_numbers.borrow().iter() {
                NSString::from_str(number).drawAtPoint_withAttributes(*point, Some(&attrs));
            }
            for (label, point, size) in editor.ivars().list_labels.borrow().iter() {
                let font = crate::font::regular(*size);
                let attrs = NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font as &AnyObject, &*color as &AnyObject]);
                NSString::from_str(label).drawAtPoint_withAttributes(*point, Some(&attrs));
            }
            NSColor::colorWithSRGBRed_green_blue_alpha(0.48, 0.49, 0.54, 1.0).setFill();
            for bar in editor.ivars().quote_bars.borrow().iter() {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(*bar, 1.5, 1.5).fill();
            }
        }}
    }
);
/// The editor's mark overlay, created on first use and kept covering the text.
unsafe fn mark_overlay(text: &NSTextView) -> Retained<MarkOverlay> {
    let existing = text.subviews().into_iter().find_map(|v| v.downcast::<MarkOverlay>().ok());
    let overlay = existing.unwrap_or_else(|| {
        let allocated = MarkOverlay::alloc(MainThreadMarker::new_unchecked());
        let view: Retained<MarkOverlay> = msg_send![allocated, initWithFrame: text.bounds()];
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable);
        text.addSubview(&view);
        view
    });
    overlay.setFrame(text.bounds());
    overlay
}
define_class!(
    #[unsafe(super(NSTextField, NSControl, NSView, NSResponder))]
    #[name = "StologyHeadingPlaceholder"]
    #[thread_kind = MainThreadOnly]
    struct HeadingPlaceholder;
    impl HeadingPlaceholder {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { std::ptr::null_mut() }
    }
);
struct CodeLanguageState {
    text: Weak<NSTextView>,
    range: std::cell::Cell<NSRange>,
    fingerprint: std::cell::Cell<u64>,
    fence: RefCell<String>,
    language: String,
}
struct CodeCopyState {
    code: RefCell<String>,
}
define_class!(
    #[unsafe(super(NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologyCodeCopyButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CodeCopyState]
    struct CodeCopyButton;
    impl CodeCopyButton {
        #[unsafe(method(copyCode:))]
        fn copy_code(&self, _sender: Option<&AnyObject>) { unsafe {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard.clearContents();
            pasteboard.setString_forType(&NSString::from_str(&self.ivars().code.borrow()), NSPasteboardTypeString);
            self.setTitle(&NSString::from_str("복사됨"));
        }}
    }
);
define_class!(
    #[unsafe(super(NSPopUpButton, NSButton, NSControl, NSView, NSResponder))]
    #[name = "StologyCodeLanguageButton"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CodeLanguageState]
    struct CodeLanguageButton;
    impl CodeLanguageButton {
        #[unsafe(method(changeLanguage:))]
        fn change_language(&self, _sender: Option<&AnyObject>) { unsafe {
            let _keep_alive = Retained::retain(self as *const Self as *mut Self).unwrap();
            let state = self.ivars();
            let Some(text) = state.text.load() else { return; };
            let marked: bool = msg_send![&*text, hasMarkedText];
            if !text.isEditable() || marked { return; }
            let source = text.string().to_string();
            if fingerprint(&source) != state.fingerprint.get() { return; }
            let index = self.indexOfSelectedItem() as usize;
            let language = CODE_LANGUAGES.get(index).map(|(id, _)| *id).unwrap_or(&state.language);
            let replacement = format!("{}{}\n", state.fence.borrow(), language);
            let selection: NSRange = msg_send![&*text, selectedRange];
            let range = state.range.get();
            let delta = replacement.encode_utf16().count() as isize - range.length as isize;
            let _: () = msg_send![&*text, insertText:&*NSString::from_str(&replacement), replacementRange:range];
            if selection.location >= range.location + range.length {
                text.setSelectedRange(NSRange::new(selection.location.saturating_add_signed(delta), selection.length));
            }
            if let Some(window) = text.window() { window.makeFirstResponder(Some(&text)); }
        }}
    }
);

include!("note_insert_menu.rs");

#[derive(Default)]
struct EditorState {
    create_diagram: std::cell::Cell<Option<fn()>>,
    selected_image: std::cell::Cell<Option<usize>>,
    pasting: std::cell::Cell<bool>,
    empty_text_selection: std::cell::Cell<Option<u64>>,
    base_directory: RefCell<Option<PathBuf>>,
    code_panels: RefCell<Vec<NSRect>>,
    line_numbers: RefCell<Vec<(String, NSPoint)>>,
    list_labels: RefCell<Vec<(String, NSPoint, f64)>>,
    quote_bars: RefCell<Vec<NSRect>>,
    image_drop_line: std::cell::Cell<Option<f64>>,
    rule_bars: RefCell<Vec<NSRect>>,
    sides: RefCell<Vec<SideText>>,
    /// Blocks and outlines for the current layout, measured on first use.
    block_cache: RefCell<Option<(u64, Vec<note_blocks::Block>, Vec<Option<BlockBox>>)>>,
    /// Source fingerprint and byte ranges of the selected blocks.
    selected_blocks: RefCell<(u64, Vec<Range<usize>>)>,
    block_marks: RefCell<Vec<NSRect>>,
    block_marquee: std::cell::Cell<Option<NSRect>>,
    marquee: RefCell<Option<BlockMarquee>>,
    block_drop: std::cell::Cell<Option<NSRect>>,
    /// What the last finished styling pass saw.
    rendered: std::cell::Cell<Option<RenderKey>>,
}
/// Everything a polling caller can change between two styling passes.
#[derive(Clone, Copy, PartialEq)]
struct RenderKey {
    text: u64,
    length: usize,
    selection: (usize, usize),
    focused: bool,
    editable: bool,
    windowed: bool,
    width: f64,
}
define_class!(
    #[unsafe(super(NSTextView, NSText, NSView, NSResponder))]
    #[name = "StologyMarkdownEditor"]
    #[thread_kind = MainThreadOnly]
    #[ivars = EditorState]
    struct MarkdownEditor;
    impl MarkdownEditor {
        #[unsafe(method(setString:))]
        fn set_document_text(&self, value: &NSString) { unsafe {
            self.ivars().selected_image.set(None);
            let _: () = msg_send![super(self), setString:value];
        }}
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, value: &AnyObject, range: NSRange) { unsafe {
            self.ivars().empty_text_selection.set(None);
            let is_string: bool = msg_send![value, isKindOfClass:NSString::class()];
            let marked: bool = msg_send![self, hasMarkedText];
            if is_string && !marked && self.isEditable() {
                let value_string = &*(value as *const AnyObject as *const NSString);
                if let Some((replacement,caret))=image_edit::replacing_selected_text(self,&value_string.to_string(),range) {
                    self.breakUndoCoalescing();
                    let _: ()=msg_send![super(self),insertText:&*NSString::from_str(&replacement),replacementRange:NSRange::new(0,self.string().length())];
                    self.setSelectedRange(NSRange::new(caret,0));
                    self.breakUndoCoalescing();
                    return;
                }
                let selection: NSRange = msg_send![self, selectedRange];
                let effective = if range.location > self.string().length() { selection } else { range };
                if effective.length == 0 && !value_string.to_string().starts_with(['\n','\r','|']) {
                    let source = self.string().to_string();
                    let at = byte_for_utf16(&source,effective.location);
                    if after_grid_line(&source, at) {
                        let replacement = format!("\n{}", value_string);
                        let _: () = msg_send![super(self), insertText:&*NSString::from_str(&replacement), replacementRange:effective];
                        return;
                    }
                }
                if effective.length == 0 && !self.ivars().pasting.get() {
                    let source = self.string().to_string();
                    let caret = byte_for_utf16(&source, effective.location);
                    if let Some(edit) = markdown_input::typed_fence(&source, caret, &value_string.to_string()) {
                        self.breakUndoCoalescing();
                        let _: () = msg_send![super(self), insertText:&*NSString::from_str(&edit.replacement), replacementRange:utf16_range(&source, edit.range)];
                        self.setSelectedRange(NSRange::new(source[..caret].encode_utf16().count() + 1, 0));
                        self.breakUndoCoalescing();
                        return;
                    }
                    if let Some(replacement) = smart_edit::typed_rule(&source, caret, &value_string.to_string()) {
                        self.breakUndoCoalescing();
                        let _: () = msg_send![super(self), insertText:&*NSString::from_str(replacement), replacementRange:effective];
                        self.breakUndoCoalescing();
                        return;
                    }
                    if let Some((replace, replacement)) = smart_edit::typed_arrow(&source, caret, &value_string.to_string()) {
                        self.breakUndoCoalescing();
                        let _: () = msg_send![super(self), insertText:&*NSString::from_str(replacement), replacementRange:utf16_range(&source, replace)];
                        self.breakUndoCoalescing();
                        return;
                    }
                }
                if value_string.to_string() == " " && effective.length == 0 {
                    let source = self.string().to_string();
                    let caret = byte_for_utf16(&source, effective.location);
                    if !self.ivars().pasting.get() {
                        if let Some((range, replacement)) = smart_edit::arrow(&source, caret) {
                            self.breakUndoCoalescing();
                            let _: () = msg_send![super(self), insertText:&*NSString::from_str(replacement), replacementRange:utf16_range(&source, range)];
                            self.breakUndoCoalescing();
                            return;
                        }
                    }
                    let start = source[..caret].rfind('\n').map_or(0, |n| n+1);
                    if matches!(&source[start..caret], "|" | "\"")
                        && !spans(&source).iter().any(|s| matches!(s.treatment, Treatment::CodeBlock | Treatment::Table) && s.bytes.start <= start && start < s.bytes.end) {
                        let _: () = msg_send![super(self), insertText:&*NSString::from_str("> "), replacementRange:utf16_range(&source, start..caret)];
                        return;
                    }
                }
            }
            let _: () = msg_send![super(self), insertText:value, replacementRange:range];
        }}
        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_image_safe_marked_text(&self, value: &AnyObject, selected: NSRange, range: NSRange) { unsafe {
            if self.isEditable() {
                let selection:NSRange = msg_send![self,selectedRange];
                let effective = if range.location > self.string().length() {selection} else {range};
                let source = self.string().to_string();
                let marked:bool = msg_send![self,hasMarkedText];
                if !marked && effective.length == 0 && after_grid_line(&source,byte_for_utf16(&source,effective.location)) {
                    let _:() = msg_send![super(self),insertText:&*NSString::from_str("\n"),replacementRange:effective];
                    let _:() = msg_send![super(self),setMarkedText:value,selectedRange:selected,replacementRange:NSRange::new(usize::MAX,0)];
                    return;
                }
                if let Some((replacement,caret))=image_edit::replacing_selected_text(self,"",range) {
                    self.breakUndoCoalescing();
                    let _: ()=msg_send![super(self),insertText:&*NSString::from_str(&replacement),replacementRange:NSRange::new(0,self.string().length())];
                    self.setSelectedRange(NSRange::new(caret,0));
                    let _: ()=msg_send![super(self),setMarkedText:value,selectedRange:selected,replacementRange:NSRange::new(usize::MAX,0)];
                    return;
                }
            }
            let _: ()=msg_send![super(self),setMarkedText:value,selectedRange:selected,replacementRange:range];
        }}
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool { unsafe {
            let flags = event.modifierFlags();
            if event.keyCode() == 2 && flags.contains(NSEventModifierFlags::Command | NSEventModifierFlags::Shift)
                && !flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
                && self.ivars().create_diagram.get().is_some() {
                if let Some(create) = self.ivars().create_diagram.get() { create(); }
                return true.into();
            }
            if table_shortcut(self, event) || selection_shortcut(self, event) || formatting_shortcut(self, event) { true } else { msg_send![super(self), performKeyEquivalent:event] }
        }}
        #[unsafe(method(expandSelection:))]
        fn expand_selection(&self, _sender: Option<&AnyObject>) { unsafe {
            expand_selection(self);
        }}
        #[unsafe(method(validateUserInterfaceItem:))]
        fn validate_image_paste(&self, item: &AnyObject) -> bool { unsafe {
            let action: Option<objc2::runtime::Sel>=msg_send![item,action];
            if action==Some(sel!(expandSelection:)) {
                let marked: bool = msg_send![self, hasMarkedText];
                self.isSelectable() && !marked
            } else if action==Some(sel!(paste:)) && image_edit::has_clipboard_image() {
                self.isEditable()
            } else if (action==Some(sel!(copy:)) || action==Some(sel!(cut:))) && !self.ivars().selected_blocks.borrow().1.is_empty() {
                true
            } else { msg_send![super(self),validateUserInterfaceItem:item] }
        }}
        #[unsafe(method(paste:))]
        fn paste(&self, sender: Option<&AnyObject>) { unsafe {
            self.ivars().pasting.set(true);
            if !image_edit::paste(self) { let _: () = msg_send![super(self), paste:sender]; }
            self.ivars().pasting.set(false);
        }}
        #[unsafe(method(menuForEvent:))]
        fn menu_for_event(&self, _event: &NSEvent) -> *mut NSMenu { unsafe {
            self.note_insert_menu().map_or(std::ptr::null_mut(), Retained::autorelease_return)
        }}
        #[unsafe(method(rightMouseDown:))]
        fn note_right_mouse_down(&self, event: &NSEvent) { unsafe {
            // NSTextView can select the word under a secondary click. Keep the writing caret.
            if let Some(menu) = self.note_insert_menu() {
                NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
            }
        }}
        #[unsafe(method(insertNoteCode:))]
        fn insert_note_code(&self, _: Option<&AnyObject>) { unsafe {
            let selection: NSRange = msg_send![self, selectedRange];
            insert_note_block(self, selection, "```text\n\n```\n", "```text\n".len());
        }}
        #[unsafe(method(insertNoteTable:))]
        fn insert_note_table(&self, _: Option<&AnyObject>) { unsafe {
            let selection: NSRange = msg_send![self, selectedRange];
            insert_note_block(self, selection, EMPTY_NOTE_TABLE, EMPTY_NOTE_TABLE.len());
        }}
        #[unsafe(method(createNoteDiagram:))]
        fn create_note_diagram(&self, _: Option<&AnyObject>) {
            if let Some(create) = self.ivars().create_diagram.get() { create(); }
        }
        #[unsafe(method(selectAll:))]
        fn select_all(&self, sender: Option<&AnyObject>) { unsafe {
            self.ivars().empty_text_selection.set(None);
            let marked: bool = msg_send![self, hasMarkedText];
            let selection: NSRange = msg_send![self, selectedRange];
            if !marked && self.selectedRanges().count() == 1 {
                if let Some(body) = code_selection(&self.string().to_string(), selection) {
                    self.setSelectedRange(body);
                    return;
                }
            }
            if !marked && image_edit::select_text(self) {
                let selected: NSRange=msg_send![self,selectedRange];
                if selected.length==0 {self.ivars().empty_text_selection.set(Some(fingerprint(&self.string().to_string())));}
                return;
            }
            let _: () = msg_send![super(self), selectAll:sender];
        }}
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) { unsafe {
            if !self.ivars().selected_blocks.borrow().1.is_empty() {
                let flags = event.modifierFlags();
                let plain = !flags.intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Option | NSEventModifierFlags::Control);
                if plain && matches!(event.keyCode(), 51 | 117) { self.delete_selected_blocks(); return; }
                // Escape only lets go of the blocks; any other key goes back to writing.
                if !flags.contains(NSEventModifierFlags::Command) && self.clear_block_selection() && event.keyCode() == 53 { return; }
            }
            if matches!(event.keyCode(), 51 | 117)
                && !event.modifierFlags().intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Option | NSEventModifierFlags::Control)
                && image_edit::delete_selected_image(self) { return; }
            image_edit::select_image(self, None);
            if table_shortcut(self, event) || selection_shortcut(self, event) || formatting_shortcut(self, event) { return; }
            if matches!(event.keyCode(), 36 | 76)
                && event.modifierFlags().contains(NSEventModifierFlags::Shift)
                && input_command(self, markdown_input::Command::ShiftEnter) { return; }
            let _: () = msg_send![super(self), keyDown:event];
        }}
        #[unsafe(method(accessibilityChildren))]
        fn accessibility_children(&self) -> *mut objc2_foundation::NSArray<NSView> { Retained::autorelease_return(self.subviews()) }
        #[unsafe(method(moveLeft:))]
        fn move_left(&self, sender: Option<&AnyObject>) { unsafe {
            let before: NSRange = msg_send![self, selectedRange];
            let _: () = msg_send![super(self), moveLeft:sender];
            skip_heading_marker(self, true);
            skip_block_marker(self, true);
            if !settle_inline_selection(self, before, true) { skip_inline_marker(self, true,inline_syntax); }
            skip_code_fence(self, true); skip_image_block(self, true); skip_rule(self, true);
        }}
        #[unsafe(method(moveRight:))]
        fn move_right(&self, sender: Option<&AnyObject>) { unsafe {
            let before: NSRange = msg_send![self, selectedRange];
            let _: () = msg_send![super(self), moveRight:sender];
            skip_heading_marker(self, false);
            skip_block_marker(self, false);
            if !settle_inline_selection(self, before, false) { skip_inline_marker(self, false,inline_syntax); }
            skip_code_fence(self, false); skip_image_block(self, false); skip_rule(self, false);
        }}
        #[unsafe(method(moveUp:))]
        fn move_up(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveUp:sender];
            visible_line_start(self); skip_code_fence(self, true); skip_image_block(self, true); skip_rule(self, true);
        }}
        #[unsafe(method(moveDown:))]
        fn move_down(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveDown:sender];
            visible_line_start(self); skip_code_fence(self, false); skip_image_block(self, false); skip_rule(self, false);
        }}
        #[unsafe(method(moveToEndOfDocument:))]
        fn move_document_end(&self, sender: Option<&AnyObject>) { unsafe {
            let attachment = self.ends_with_rendered_attachment();
            let _: () = msg_send![super(self), moveToEndOfDocument:sender];
            if attachment {
                self.open_writing_line_after_attachment();
            }
        }}
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            image_edit::select_image(self, None);
            self.ivars().empty_text_selection.set(None);
            if self.begin_block_marquee(event) {
                // A Shift drag keeps the selection; it is redrawn as the drag grows.
                if !event.modifierFlags().contains(NSEventModifierFlags::Shift) { self.clear_block_selection(); }
                return;
            }
            self.clear_block_selection();
            self.click(event);
        }}
        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) { unsafe {
            if !self.continue_block_marquee(event) { let _: () = msg_send![super(self), mouseDragged:event]; }
        }}
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) { unsafe {
            match self.finish_block_marquee() {
                // The pointer never moved: replay the press as an ordinary
                // click, with this release for the text view's own tracking.
                Some(Some(down)) => {
                    if let Some(window) = self.window() { window.postEvent_atStart(event, true); }
                    self.click(&down);
                }
                Some(None) => {}
                None => { let _: () = msg_send![super(self), mouseUp:event]; }
            }
        }}
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) { unsafe {
            let _: () = msg_send![super(self), mouseMoved:event];
            self.hover_block(Some(self.convertPoint_fromView(event.locationInWindow(), None)));
        }}
        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, event: &NSEvent) { unsafe {
            let _: () = msg_send![super(self), mouseExited:event];
            self.hover_block(None);
        }}
        #[unsafe(method(copy:))]
        fn copy_blocks(&self, sender: Option<&AnyObject>) { unsafe {
            if !self.copy_selected_blocks() { let _: () = msg_send![super(self), copy:sender]; }
        }}
        #[unsafe(method(cut:))]
        fn cut_blocks(&self, sender: Option<&AnyObject>) { unsafe {
            if self.copy_selected_blocks() { self.delete_selected_blocks(); } else { let _: () = msg_send![super(self), cut:sender]; }
        }}
        #[unsafe(method(deleteSelectedBlocks:))]
        fn delete_blocks(&self, _sender: Option<&AnyObject>) { unsafe { self.delete_selected_blocks(); } }
        #[unsafe(method(moveToBeginningOfLine:))]
        fn move_line_start(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveToBeginningOfLine:sender];
            visible_line_start(self);
            skip_code_fence(self, false);
        }}
        #[unsafe(method(moveToBeginningOfParagraph:))]
        fn move_paragraph_start(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveToBeginningOfParagraph:sender];
            visible_line_start(self);
            skip_code_fence(self, false);
        }}
        #[unsafe(method(drawViewBackgroundInRect:))]
        fn draw_background(&self, rect: NSRect) { unsafe {
            let _: () = msg_send![super(self), drawViewBackgroundInRect:rect];
            self.draw_block_marks();
            NSColor::colorWithSRGBRed_green_blue_alpha(0.957, 0.961, 0.980, 1.0).setFill();
            for frame in self.ivars().code_panels.borrow().iter() {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(*frame, 10.0, 10.0).fill();
            }
            if let Some(y) = self.ivars().image_drop_line.get() {
                NSColor::controlAccentColor().setFill();
                NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::new(EDITOR_INSET, y), NSSize::new(self.bounds().size.width - 2. * EDITOR_INSET, 2.))).fill();
            }
            // Rules, code line numbers, list labels and quote bars are drawn by
            // MarkOverlay so the selection highlight, painted above this
            // background, cannot hide them.
        }}
        #[unsafe(method(didChangeText))]
        fn did_change_text(&self) { unsafe {
            self.ivars().selected_image.set(None);
            // Writing puts the handle away until the pointer moves again.
            if let Some(handle) = self.subviews().into_iter().find_map(|v| v.downcast::<BlockHandle>().ok()) { handle.setHidden(true); }
            let _: () = msg_send![super(self), didChangeText];
            let base = self.ivars().base_directory.borrow().clone();
            style_with_base(self, base.as_deref());
        }}
        // Undo and redo replace text without didChangeText, so overlays drawn
        // from the last styling pass (rules, panels) would otherwise stay stale.
        #[unsafe(method(stologyUndoChanged:))]
        fn undo_changed(&self, notification: &objc2_foundation::NSNotification) { unsafe {
            let manager: Option<Retained<AnyObject>> = msg_send![self, undoManager];
            let same = match (notification.object(), manager) {
                (Some(sender), Some(manager)) => Retained::as_ptr(&sender) == Retained::as_ptr(&manager),
                _ => false,
            };
            let marked: bool = msg_send![self, hasMarkedText];
            if !same || marked { return; }
            settle_rule_selection(self);
            // AppKit may select the restored text only after this notification.
            let _: () = msg_send![self, performSelector:sel!(stologySettleRule:), withObject:std::ptr::null::<AnyObject>(), afterDelay:0.0f64];
            let base = self.ivars().base_directory.borrow().clone();
            style_with_base(self, base.as_deref());
        }}
        #[unsafe(method(stologySettleRule:))]
        fn settle_rule(&self, _sender: Option<&AnyObject>) { unsafe {
            if settle_rule_selection(self) {
                let base = self.ivars().base_directory.borrow().clone();
                style_with_base(self, base.as_deref());
            }
        }}
        #[unsafe(method(deleteForward:))]
        fn delete_forward(&self, sender: Option<&AnyObject>) { unsafe {
            if image_edit::delete_selected_image(self) { return; }
            let selection:NSRange=msg_send![self,selectedRange];
            if selection==NSRange::new(0,0) && self.ivars().empty_text_selection.get()==Some(fingerprint(&self.string().to_string())) {return;}
            let _: ()=msg_send![super(self),deleteForward:sender];
        }}
        #[unsafe(method(deleteBackward:))]
        fn delete_backward(&self, sender: Option<&AnyObject>) { unsafe {
            if image_edit::delete_selected_image(self) { return; }
            if !input_command(self, markdown_input::Command::Backspace) && !heading_backspace(self) && !inline_backspace(self,inline_syntax) {
                let _: () = msg_send![super(self), deleteBackward:sender];
            }
        }}
        #[unsafe(method(insertNewline:))]
        fn insert_newline(&self, sender: Option<&AnyObject>) { unsafe {
            if !empty_heading_enter(self) && !input_command(self, markdown_input::Command::Enter) {
                let _: () = msg_send![super(self), insertNewline:sender];
            }
        }}
        #[unsafe(method(insertLineBreak:))]
        fn insert_line_break(&self, sender: Option<&AnyObject>) { unsafe {
            if !input_command(self, markdown_input::Command::ShiftEnter) {
                let _: () = msg_send![super(self), insertLineBreak:sender];
            }
        }}
        #[unsafe(method(insertTab:))]
        fn insert_tab(&self, sender: Option<&AnyObject>) { unsafe {
            if !input_command(self, markdown_input::Command::Tab) {
                let _: () = msg_send![super(self), insertTab:sender];
            }
        }}
        #[unsafe(method(insertBacktab:))]
        fn insert_backtab(&self, sender: Option<&AnyObject>) { unsafe {
            if !input_command(self, markdown_input::Command::BackTab) {
                let _: () = msg_send![super(self), insertBacktab:sender];
            }
        }}
    }
);

impl MarkdownEditor {
    unsafe fn click(&self, event: &NSEvent) {
        if self.place_block_caret(event) { return; }
        let _: () = msg_send![super(self), mouseDown:event];
        visible_line_start(self); skip_code_fence(self, false); skip_image_block(self, false); skip_rule(self, false);
    }
}

unsafe fn expand_selection(text: &NSTextView) {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isSelectable() || text.selectedRanges().count() != 1 { return; }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    let word = text.selectionRangeForProposedRange_granularity(selection, NSSelectionGranularity::SelectByWord);
    let bytes = |range: NSRange| byte_for_utf16(&source, range.location)
        ..byte_for_utf16(&source, range.location.saturating_add(range.length));
    if let Some(next) = smart_edit::expand(&source, bytes(selection), bytes(word)) {
        let range = utf16_range(&source, next);
        text.setSelectedRange(range);
        text.scrollRangeToVisible(range);
    }
}

unsafe fn selection_shortcut(text: &NSTextView, event: &NSEvent) -> bool {
    let flags = event.modifierFlags();
    if event.keyCode() != 2 || !flags.contains(NSEventModifierFlags::Command)
        || flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option | NSEventModifierFlags::Shift) {
        return false;
    }
    // Consume the command during composition without changing marked text.
    expand_selection(text);
    true
}

fn heading_prefix(raw: &str) -> Option<(usize, u8)> {
    let indent = raw.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let hashes = raw[indent..].bytes().take_while(|b| *b == b'#').count();
    let end = indent + hashes;
    if !(1..=6).contains(&hashes) || !matches!(raw.as_bytes().get(end), Some(b' ' | b'\t')) {
        return None;
    }
    let spaces = raw[end..]
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    Some((end + spaces, hashes as u8))
}

unsafe fn apply_source_edit(text: &NSTextView, edit: markdown_shortcuts::Edit) {
    let source = text.string().to_string();
    let mut result = source.clone();
    result.replace_range(edit.range.clone(), &edit.replacement);
    let _: () = msg_send![text, insertText:&*NSString::from_str(&edit.replacement), replacementRange:utf16_range(&source, edit.range)];
    text.setSelectedRange(utf16_range(&result, edit.selection));
}

fn toggle_format(
    source: &str,
    selection: Range<usize>,
    open: &str,
    close: &str,
) -> Option<markdown_shortcuts::Edit> {
    if selection.is_empty() {
        return None;
    }
    let kind = match open {
        "**" => Treatment::Bold,
        "*" => Treatment::Italic,
        "~~" => Treatment::Strike,
        "<u>" => Treatment::Underline,
        "`" => Treatment::Code,
        _ => return None,
    };
    let hidden = hidden_syntax(source, None);
    let concealed = |range: Range<usize>| {
        range
            .into_iter()
            .all(|i| hidden.iter().any(|r| r.contains(&i)))
    };
    let mut candidates: Vec<_> = spans(source)
        .into_iter()
        .filter(|s| s.treatment == kind)
        .collect();
    candidates.sort_by_key(|s| s.bytes.len());
    for span in candidates {
        let opening = if kind == Treatment::Code {
            source[span.bytes.clone()]
                .bytes()
                .take_while(|b| *b == b'`')
                .count()
        } else {
            open.len()
        };
        let closing = if kind == Treatment::Code {
            opening
        } else {
            close.len()
        };
        let body = span.bytes.start + opening..span.bytes.end - closing;
        if body.start <= selection.start
            && selection.end <= body.end
            && concealed(body.start..selection.start)
            && concealed(selection.end..body.end)
        {
            return Some(markdown_shortcuts::Edit {
                range: span.bytes,
                replacement: source[body].to_string(),
                selection: selection.start - opening..selection.end - opening,
            });
        }
    }
    markdown_shortcuts::wrap(source, selection, open, close)
}

fn inline_marks(event: &NSEvent) -> Option<(&'static str,&'static str)> {
    let flags = event.modifierFlags();
    if !flags.contains(NSEventModifierFlags::Command) || flags.intersects(NSEventModifierFlags::Option | NSEventModifierFlags::Control) { return None; }
    match (event.keyCode(),flags.contains(NSEventModifierFlags::Shift)) {
        (11,false) => Some(("**","**")), (34,false) => Some(("*","*")),
        (32,false) => Some(("<u>","</u>")), (1,true) => Some(("~~","~~")),
        (14,false) => Some(("`","`")), _ => None,
    }
}

/// Matches physical QWERTY keys as well as characters so Korean input sources
/// use the same shortcuts. System cut/copy/paste remain native responder actions.
unsafe fn formatting_shortcut(text: &NSTextView, event: &NSEvent) -> bool {
    let flags = event.modifierFlags();
    if !flags.contains(NSEventModifierFlags::Command)
        || flags.contains(NSEventModifierFlags::Control)
    {
        return false;
    }
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return false;
    }
    let selection: NSRange = msg_send![text, selectedRange];
    let source = text.string().to_string();
    let bytes = byte_for_utf16(&source, selection.location)
        ..byte_for_utf16(&source, selection.location + selection.length);
    let option = flags.contains(NSEventModifierFlags::Option);
    let shift = flags.contains(NSEventModifierFlags::Shift);
    let key = event.keyCode();
    let block = if option && !shift {
        match key {
            29 => Some(0),
            18 => Some(1),
            19 => Some(2),
            20 => Some(3),
            21 => Some(4),
            23 => Some(5),
            22 => Some(6),
            28 => Some(8),
            _ => None,
        }
    } else {
        None
    };
    let marks = inline_marks(event);
    let link = key == 40 && !option && !shift && selection.length > 0;
    if block.is_none() && marks.is_none() && !link {
        return false;
    }
    // Do not turn literal code or table syntax into formatting delimiters.
    if spans(&source).iter().any(|s| {
        matches!(
            s.treatment,
            Treatment::CodeBlock | Treatment::Code | Treatment::Table
        ) && s.bytes.start < bytes.end.max(bytes.start + 1)
            && bytes.start < s.bytes.end
            && !(marks == Some(("`", "`")) && s.treatment == Treatment::Code)
    }) {
        return true;
    }
    let edit = if let Some(kind) = block {
        markdown_shortcuts::block_edit(&source, bytes, kind)
    } else if let Some((open, close)) = marks {
        toggle_format(&source, bytes, open, close)
    } else {
        return add_link(text);
    };
    if let Some(edit) = edit {
        apply_source_edit(text, edit);
    }
    true
}

pub unsafe fn add_link(text: &NSTextView) -> bool {
    let selection: NSRange = msg_send![text, selectedRange];
    let marked: bool = msg_send![text, hasMarkedText];
    if selection.length == 0 || !text.isEditable() || marked {
        return false;
    }
    let source = text.string().to_string();
    let bytes = byte_for_utf16(&source, selection.location)
        ..byte_for_utf16(&source, selection.location + selection.length);
    if source[bytes.clone()].contains(['\r', '\n']) {
        return true;
    }
    if spans(&source).iter().any(|s| {
        matches!(
            s.treatment,
            Treatment::CodeBlock | Treatment::Code | Treatment::Table
        ) && s.bytes.start < bytes.end
            && bytes.start < s.bytes.end
    }) {
        return true;
    }
    let mtm = MainThreadMarker::new().unwrap();
    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str("링크 추가"));
    alert.setInformativeText(&NSString::from_str(
        "선택한 텍스트에 연결할 URL을 입력하세요.",
    ));
    let field = NSTextField::initWithFrame(
        NSTextField::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(360.0, 28.0)),
    );
    field.setFont(Some(&crate::font::regular(13.0)));
    field.setPlaceholderString(Some(&NSString::from_str("https://")));
    alert.setAccessoryView(Some(&field));
    alert.addButtonWithTitle(&NSString::from_str("적용"));
    alert.addButtonWithTitle(&NSString::from_str("취소"));
    alert.window().setInitialFirstResponder(Some(&field));
    if alert.runModal() == 1000 {
        let url = field.stringValue().to_string();
        let url = url.trim();
        if url.starts_with("https://") || url.starts_with("http://") || url.starts_with("mailto:") {
            let url = url
                .replace(' ', "%20")
                .replace('(', "%28")
                .replace(')', "%29")
                .replace('\n', "")
                .replace('\r', "");
            if let Some(edit) = markdown_shortcuts::wrap(&source, bytes, "[", &format!("]({url})"))
            {
                apply_source_edit(text, edit);
            }
        }
    }
    true
}

unsafe fn skip_block_marker(text: &NSTextView, backwards: bool) {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || selection.length != 0 {
        return;
    }
    let source = text.string().to_string();
    let caret = byte_for_utf16(&source, selection.location);
    if let Some(block) = markdown_shortcuts::blocks(&source)
        .into_iter()
        .find(|b| b.prefix.start <= caret && caret < b.prefix.end)
    {
        let target = if backwards && block.prefix.start > 0 {
            block.prefix.start - 1
        } else {
            block.prefix.end
        };
        text.setSelectedRange(NSRange::new(source[..target].encode_utf16().count(), 0));
    }
}
fn heading_at(source: &str, selection: NSRange) -> Option<(Range<usize>, Range<usize>, u8)> {
    if selection.length != 0 {
        return None;
    }
    let caret = byte_for_utf16(source, selection.location);
    spans(source).into_iter().find_map(|span| {
        let Treatment::Heading(level) = span.treatment else {
            return None;
        };
        let raw = &source[span.bytes.clone()];
        let (len, _) = heading_prefix(raw)?;
        let end = span.bytes.start + raw.trim_end_matches(['\r', '\n']).len();
        (span.bytes.start <= caret && caret <= end).then(|| {
            (
                span.bytes.start..span.bytes.start + len,
                span.bytes.start + len..end,
                level,
            )
        })
    })
}
unsafe fn heading_backspace(text: &NSTextView) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return false;
    }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    let Some((prefix, _, _)) = heading_at(&source, selection) else {
        return false;
    };
    if byte_for_utf16(&source, selection.location) > prefix.end {
        return false;
    }
    let _: () = msg_send![text, insertText:&*NSString::from_str(""), replacementRange:utf16_range(&source, prefix)];
    true
}

struct InlineSyntax {
    bytes: Range<usize>,
    content: Range<usize>,
}
fn inline_syntax(source: &str) -> Vec<InlineSyntax> {
    spans(source)
        .into_iter()
        .filter_map(|span| {
            let raw = &source[span.bytes.clone()];
            let (opening, closing) = match span.treatment {
                Treatment::Underline => (3, 4),
                Treatment::Bold | Treatment::Strike => (2, 2),
                Treatment::Italic => (1, 1),
                Treatment::Code => {
                    let ticks = raw.bytes().take_while(|b| *b == b'`').count();
                    if ticks == 0 {
                        return None;
                    }
                    (ticks, ticks)
                }
                Treatment::Link(_) if raw.starts_with('[') => {
                    let mut depth = 0usize;
                    let mut escaped = false;
                    let mut end = None;
                    for (index, c) in raw.char_indices() {
                        if escaped {
                            escaped = false;
                            continue;
                        }
                        match c {
                            '\\' => escaped = true,
                            '[' => depth += 1,
                            ']' => {
                                depth = depth.saturating_sub(1);
                                if depth == 0 && raw[index + 1..].starts_with('(') {
                                    end = Some(index);
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    (1, raw.len() - end?)
                }
                _ => return None,
            };
            if raw.len() < opening + closing {
                return None;
            }
            Some(InlineSyntax {
                content: span.bytes.start + opening..span.bytes.end - closing,
                bytes: span.bytes,
            })
        })
        .collect()
}
fn inline_marker_ranges(syntax: &[InlineSyntax]) -> Vec<Range<usize>> {
    let mut ranges: Vec<_> = syntax
        .iter()
        .flat_map(|s| [s.bytes.start..s.content.start, s.content.end..s.bytes.end])
        .collect();
    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ranges {
        if let Some(previous) = merged.last_mut() {
            if range.start <= previous.end {
                previous.end = previous.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }
    merged
}
unsafe fn previous_composed_character(source: &str, end: usize) -> Option<Range<usize>> {
    let units = source[..end].encode_utf16().count();
    if units == 0 {
        return None;
    }
    let native = NSString::from_str(source);
    let range = native.rangeOfComposedCharacterSequenceAtIndex(units - 1);
    Some(
        byte_for_utf16(source, range.location)
            ..byte_for_utf16(source, range.location + range.length),
    )
}
unsafe fn inline_backspace(text: &NSTextView, syntax_for:fn(&str)->Vec<InlineSyntax>) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || !text.isEditable() || selection.length != 0 {
        return false;
    }
    let source = text.string().to_string();
    let syntax = syntax_for(&source);
    let markers = inline_marker_ranges(&syntax);
    let caret = byte_for_utf16(&source, selection.location);
    let mut end = caret;
    for range in markers.iter().rev() {
        if range.start < end && end <= range.end {
            end = range.start;
        }
    }
    let Some(mut deletion) = previous_composed_character(&source, end) else {
        return end != caret;
    };
    let mut removed_wrapper = false;
    for span in &syntax {
        if span.content.start <= deletion.start && deletion.end <= span.content.end {
            let remaining = source[span.content.clone()]
                .char_indices()
                .any(|(offset, _)| {
                    let position = span.content.start + offset;
                    !deletion.contains(&position)
                        && !markers.iter().any(|range| range.contains(&position))
                });
            if !remaining {
                deletion = span.bytes.clone();
                removed_wrapper = true;
            }
        }
    }
    // Let AppKit perform ordinary deletion inside formatted content. Intercept
    // only hidden delimiters or a final grapheme whose empty wrapper is invalid.
    if end == caret && !removed_wrapper {
        return false;
    }
    let _: () = msg_send![text, insertText:&*NSString::from_str(""), replacementRange:utf16_range(&source, deletion)];
    true
}

// Called after native left/right movement. A key moves by one visible composed
// character, never leaving the insertion point in hidden emphasis/link syntax.
unsafe fn settle_inline_selection(text: &NSTextView, before: NSRange, backwards: bool) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || before.length == 0 || selection.length != 0 { return false; }
    let source = text.string().to_string();
    let caret = byte_for_utf16(&source, selection.location);
    let markers = inline_marker_ranges(&inline_syntax(&source));
    let marker = markers.iter().find(|range| if backwards { range.end == caret } else { range.start == caret });
    let Some(marker) = marker else { return false };
    let target = if backwards { marker.start } else { marker.end };
    text.setSelectedRange(NSRange::new(source[..target].encode_utf16().count(), 0));
    true
}

unsafe fn skip_inline_marker(text: &NSTextView, backwards: bool, syntax_for:fn(&str)->Vec<InlineSyntax>) {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || selection.length != 0 {
        return;
    }
    let source = text.string().to_string();
    let markers = inline_marker_ranges(&syntax_for(&source));
    let caret = byte_for_utf16(&source, selection.location);
    let Some(marker) = markers.iter().find(|range| {
        if backwards {
            range.start <= caret && caret < range.end
        } else {
            range.start < caret && caret <= range.end
        }
    }) else {
        return;
    };
    let target = if backwards {
        previous_composed_character(&source, marker.start)
            .map(|range| range.start)
            .unwrap_or(marker.end)
    } else if marker.end < source.len() {
        let native = NSString::from_str(&source);
        let position = source[..marker.end].encode_utf16().count();
        let range = native.rangeOfComposedCharacterSequenceAtIndex(position);
        byte_for_utf16(&source, range.location + range.length)
    } else {
        marker.start
    };
    text.setSelectedRange(NSRange::new(source[..target].encode_utf16().count(), 0));
}
unsafe fn empty_heading_enter(text: &NSTextView) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return false;
    }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    let Some((prefix, content, _)) = heading_at(&source, selection) else {
        return false;
    };
    if !content.is_empty() {
        return false;
    }
    let _: () = msg_send![text, insertText:&*NSString::from_str(""), replacementRange:utf16_range(&source, prefix)];
    true
}

pub(crate) fn heading_size(level: u8) -> f64 {
    match level {
        1 => 30.0,
        2 => 24.0,
        3 => 20.0,
        _ => 17.0,
    }
}
unsafe fn visible_line_start(text: &NSTextView) {
    skip_block_marker(text, false);
    let marked: bool = msg_send![text, hasMarkedText];
    if marked {
        return;
    }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    if let Some((prefix, _, _)) = heading_at(&source, selection) {
        if byte_for_utf16(&source, selection.location) < prefix.end {
            text.setSelectedRange(NSRange::new(source[..prefix.end].encode_utf16().count(), 0));
        }
    }
}
unsafe fn skip_code_fence(text: &NSTextView, backwards: bool) {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked {
        return;
    }
    let selection: NSRange = msg_send![text, selectedRange];
    if selection.length != 0 {
        return;
    }
    let source = text.string().to_string();
    if let Some(position) = markdown_input::fenced_caret(
        &source,
        byte_for_utf16(&source, selection.location),
        backwards,
    ) {
        text.setSelectedRange(NSRange::new(source[..position].encode_utf16().count(), 0));
    }
}
/// Keep the caret out of a drawn image's hidden link line and separator.
unsafe fn settle_rule_selection(text: &NSTextView) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || text.selectedRanges().count() != 1 {
        return false;
    }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    let bytes = byte_for_utf16(&source, selection.location)..byte_for_utf16(&source, selection.location + selection.length);
    let Some(caret) = markdown_shortcuts::rule_selection_caret(&source, bytes) else { return false };
    text.setSelectedRange(NSRange::new(source[..caret].encode_utf16().count(), 0));
    true
}
unsafe fn skip_rule(text: &NSTextView, backwards: bool) {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || selection.length != 0 {
        return;
    }
    let source = text.string().to_string();
    if let Some(target) = markdown_shortcuts::rule_caret(&source, byte_for_utf16(&source, selection.location), backwards) {
        text.setSelectedRange(NSRange::new(source[..target].encode_utf16().count(), 0));
    }
}
unsafe fn skip_image_block(text: &NSTextView, backwards: bool) {
    let marked: bool = msg_send![text, hasMarkedText];
    let selection: NSRange = msg_send![text, selectedRange];
    if marked || selection.length != 0 {
        return;
    }
    let source = text.string().to_string();
    let caret = byte_for_utf16(&source, selection.location);
    for child in text.subviews() {
        let Some(image) = child.downcast_ref::<InlineImage>() else { continue };
        let link = &image.ivars().link.bytes;
        if !source.is_char_boundary(link.end) || !standalone(&source, link) {
            continue;
        }
        let start = source[..link.start].rfind('\n').map_or(0, |n| n + 1);
        let mut end = source[link.end..].find('\n').map_or(source.len(), |n| link.end + n + 1);
        // The blank separator line is hidden with the link.
        let separator = source[end..].split_inclusive('\n').next().unwrap_or("");
        if end > link.end && separator.ends_with('\n') && separator.trim().is_empty() {
            end += separator.len();
        }
        if start <= caret && caret < end {
            let target = if backwards && start > 0 { start - 1 } else { end };
            text.setSelectedRange(NSRange::new(source[..target].encode_utf16().count(), 0));
            return;
        }
    }
}
unsafe fn skip_heading_marker(text: &NSTextView, backwards: bool) {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked {
        return;
    }
    let source = text.string().to_string();
    let selection: NSRange = msg_send![text, selectedRange];
    if let Some((prefix, _, _)) = heading_at(&source, selection) {
        if byte_for_utf16(&source, selection.location) < prefix.end {
            let location = if backwards && prefix.start > 0 {
                source[..prefix.start].encode_utf16().count() - 1
            } else {
                source[..prefix.end].encode_utf16().count()
            };
            text.setSelectedRange(NSRange::new(location, 0));
        }
    }
}

#[link(name = "PDFKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "QuickLookUI", kind = "framework")]
unsafe extern "C" {}

// The full-width editor starts at the document edge. AppKit adds 5 pt of
// line-fragment padding, aligning text at x=30 with the title label.
const EDITOR_INSET: f64 = 25.0;
const LINE_PADDING: f64 = 5.0;

pub unsafe fn make_editor(frame: NSRect) -> (Retained<NSScrollView>, Retained<NSTextView>) {
    install_grid_context_filter();
    let mtm = MainThreadMarker::new().expect("editor must be created on main thread");
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), frame);
    scroll.setHasVerticalScroller(true);
    crate::scrollbar::style(&scroll);
    scroll.setHasHorizontalScroller(false);
    scroll.setDrawsBackground(true);
    scroll.setBackgroundColor(&NSColor::whiteColor());
    scroll.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let allocated = MarkdownEditor::alloc(mtm).set_ivars(EditorState::default());
    let text: Retained<MarkdownEditor> =
        msg_send![super(allocated), initUsingTextLayoutManager:true];
    for name in ["NSUndoManagerDidUndoChangeNotification", "NSUndoManagerDidRedoChangeNotification"] {
        objc2_foundation::NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
            &text, sel!(stologyUndoChanged:), Some(&NSString::from_str(name)), None);
    }
    let text: Retained<NSTextView> = Retained::into_super(text);
    text.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
    text.setMinSize(NSSize::new(0.0, frame.size.height));
    text.setMaxSize(NSSize::new(f64::MAX, f64::MAX));
    text.setVerticallyResizable(true);
    text.setHorizontallyResizable(false);
    text.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    text.setTextContainerInset(NSSize::new(EDITOR_INSET, 18.0));
    if let Some(container) = text.textContainer() {
        container.setWidthTracksTextView(true);
        container.setLineFragmentPadding(LINE_PADDING);
        container.setContainerSize(NSSize::new(
            (frame.size.width - 2.0 * EDITOR_INSET).max(80.0),
            f64::MAX,
        ));
    }
    text.setRichText(false);
    text.setImportsGraphics(false);
    text.setAllowsUndo(true);
    text.setUsesFontPanel(false);
    text.setAutomaticQuoteSubstitutionEnabled(false);
    text.setAutomaticDashSubstitutionEnabled(false);
    text.setAutomaticTextReplacementEnabled(false);
    text.setAutomaticSpellingCorrectionEnabled(false);
    text.setAutomaticLinkDetectionEnabled(false);
    text.setAutomaticDataDetectionEnabled(false);
    text.setFont(Some(&crate::font::regular(16.0)));
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setLineSpacing(6.0);
    text.setDefaultParagraphStyle(Some(&paragraph));
    text.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.09, 0.094, 0.149, 1.0,
    )));
    text.setDrawsBackground(true);
    text.setBackgroundColor(&NSColor::whiteColor());
    // Pointer moves show the block handle beside the block under the pointer.
    let area = NSTrackingArea::initWithRect_options_owner_userInfo(
        NSTrackingArea::alloc(), NSRect::ZERO,
        NSTrackingAreaOptions::MouseMoved | NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::ActiveInKeyWindow | NSTrackingAreaOptions::InVisibleRect,
        Some(&text), None);
    text.addTrackingArea(&area);
    scroll.setDocumentView(Some(&text));
    (scroll, text)
}

#[derive(Debug, Clone, PartialEq)]
enum Treatment {
    Heading(u8),
    Bold,
    Italic,
    Strike,
    Underline,
    Code,
    CodeBlock,
    Quote,
    List,
    Table,
    Link(String),
    Image,
    Rule,
}
#[derive(Debug, Clone)]
struct Span {
    bytes: Range<usize>,
    treatment: Treatment,
}
fn spans(source: &str) -> Vec<Span> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH;
    let mut result: Vec<_> = Parser::new_ext(source, options)
        .into_offset_iter()
        .filter_map(|(event, bytes)| {
            let treatment = match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    let raw = &source[bytes.clone()];
                    // Only explicit ATX headings are styled. A following '-' or
                    // '=' line must never resize the paragraph being edited.
                    if heading_prefix(raw).is_none() {
                        // A block never depends on the one above it: `---`
                        // under a text line is still a rule.
                        return markdown_shortcuts::rule_under_text(source, &bytes)
                            .map(|bytes| Span { bytes, treatment: Treatment::Rule });
                    }
                    Treatment::Heading(level as u8)
                }
                Event::Start(Tag::Strong) => Treatment::Bold,
                Event::Start(Tag::Emphasis) => Treatment::Italic,
                Event::Start(Tag::Strikethrough) => Treatment::Strike,
                Event::Start(Tag::CodeBlock(_)) => Treatment::CodeBlock,
                Event::Code(_) => Treatment::Code,
                Event::Start(Tag::BlockQuote(_)) => Treatment::Quote,
                Event::Start(Tag::List(_)) => Treatment::List,
                Event::Start(Tag::Table(_)) => Treatment::Table,
                Event::Start(Tag::Link { dest_url, .. }) => Treatment::Link(dest_url.to_string()),
                Event::Start(Tag::Image { .. }) => Treatment::Image,
                Event::Rule => Treatment::Rule,
                _ => return None,
            };
            Some(Span { bytes, treatment })
        })
        .collect();
    let mut opening = None;
    for (event, bytes) in Parser::new_ext(source, options).into_offset_iter() {
        match event {
            Event::InlineHtml(html) if html.as_ref() == "<u>" => opening = Some(bytes.start),
            Event::InlineHtml(html) if html.as_ref() == "</u>" => {
                if let Some(start) = opening.take() {
                    result.push(Span {
                        bytes: start..bytes.end,
                        treatment: Treatment::Underline,
                    });
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_)) => opening = None,
            _ => {}
        }
    }
    result
}
thread_local! {
    /// Byte-to-UTF-16 offsets of the notes being styled, innermost last.
    static OFFSETS: RefCell<Vec<(*const u8, usize, Vec<u32>)>> = const { RefCell::new(Vec::new()) };
}
/// While this lives, `utf16_offset` answers for this exact string from a table
/// instead of counting from the start of the note on every call.
struct OffsetTable<'a>(std::marker::PhantomData<&'a str>);
impl<'a> OffsetTable<'a> {
    fn new(source: &'a str) -> Self {
        let mut table = vec![0u32; source.len() + 1];
        let mut units = 0u32;
        for (byte, c) in source.char_indices() {
            table[byte..byte + c.len_utf8()].fill(units);
            units += c.len_utf16() as u32;
        }
        table[source.len()] = units;
        OFFSETS.with(|tables| tables.borrow_mut().push((source.as_ptr(), source.len(), table)));
        Self(std::marker::PhantomData)
    }
}
impl Drop for OffsetTable<'_> {
    fn drop(&mut self) {
        OFFSETS.with(|tables| { tables.borrow_mut().pop(); });
    }
}
fn utf16_offset(source: &str, byte: usize) -> usize {
    let known = OFFSETS.with(|tables| {
        tables.borrow().iter().rev()
            .find(|(start, length, _)| *start == source.as_ptr() && *length == source.len())
            .map(|(_, _, table)| table[byte] as usize)
    });
    known.unwrap_or_else(|| source[..byte].encode_utf16().count())
}
fn utf16_range(source: &str, bytes: Range<usize>) -> NSRange {
    let start = utf16_offset(source, bytes.start);
    NSRange::new(start, utf16_offset(source, bytes.end) - start)
}

fn byte_for_utf16(source: &str, offset: usize) -> usize {
    let mut units = 0;
    for (byte, c) in source.char_indices() {
        if units >= offset {
            return byte;
        }
        units += c.len_utf16();
    }
    source.len()
}
fn active_block(source: &str, selection: NSRange) -> Range<usize> {
    let byte = byte_for_utf16(source, selection.location);
    let last = byte_for_utf16(source, selection.location.saturating_add(selection.length));
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        if matches!(event, Event::Start(Tag::Table(_) | Tag::CodeBlock(_)))
            && range.start <= byte
            && byte < range.end
            && last <= range.end
        {
            return range;
        }
    }
    // Scan original lines instead of normalizing the string: AppKit selections
    // and parser spans must retain their original UTF-8/UTF-16 offsets. A blank
    // Markdown line may contain spaces or tabs and may end in CRLF.
    let first = paragraph_block_at(source, byte);
    let last_character = if last > byte {
        source[..last]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(byte)
    } else {
        byte
    };
    let final_block = paragraph_block_at(source, last_character);
    // A selection across blocks (Select All, a long drag) is for copying or
    // deleting, not for editing one block: keep every block rendered.
    if last > byte && (final_block.start >= first.end || (byte == 0 && last >= source.len())) {
        return byte..byte;
    }
    first.start..final_block.end
}
fn paragraph_block_at(source: &str, position: usize) -> Range<usize> {
    let mut begin = 0;
    let mut line_start = 0;
    for line in source.split_inclusive('\n') {
        let line_end = line_start + line.len();
        if line.trim_matches([' ', '\t', '\r', '\n']).is_empty() {
            if line_start <= position && position < line_end {
                return position..position;
            }
            if line_end <= position {
                begin = line_end;
            } else if line_start > position {
                return begin..line_start;
            }
        }
        line_start = line_end;
    }
    begin..source.len()
}
fn intersects(range: &Range<usize>, active: Option<&Range<usize>>) -> bool {
    active.is_some_and(|a| range.start < a.end && a.start < range.end)
}
fn range_active(range: NSRange, source: &str, active: Option<&Range<usize>>) -> bool {
    intersects(
        &(byte_for_utf16(source, range.location)
            ..byte_for_utf16(source, range.location + range.length)),
        active,
    )
}
unsafe fn apply_font_trait(storage: &objc2_foundation::NSMutableAttributedString, range: NSRange, trait_mask: NSFontTraitMask) {
    let manager = NSFontManager::sharedFontManager(MainThreadMarker::new().unwrap());
    let mut cursor = range.location;
    let end = range.location + range.length;
    while cursor < end {
        let mut effective = NSRange::new(0, 0);
        let current: Option<Retained<NSFont>> = msg_send![storage,attribute:NSFontAttributeName,atIndex:cursor,effectiveRange:&mut effective];
        let next = (effective.location + effective.length)
            .min(end)
            .max(cursor + 1);
        let font = current.unwrap_or_else(|| crate::font::regular(16.0));
        let mut converted = manager.convertFont_toHaveTrait(&font, trait_mask);
        let run = NSRange::new(cursor, next - cursor);
        // SUIT has no italic face. AppKit then returns an upright font, so
        // provide a presentation-only slant without changing its family/weight.
        // Use a font transform because TextKit 2 ignores NSObliqueness.
        if trait_mask.contains(NSFontTraitMask::ItalicFontMask)
            && !manager.traitsOfFont(&converted).contains(NSFontTraitMask::ItalicFontMask)
        {
            let size = converted.pointSize();
            let transform = NSAffineTransform::new();
            transform.setTransformStruct(NSAffineTransformStruct {
                m11: size, m12: 0.0, m21: size * 0.2, m22: size, tX: 0.0, tY: 0.0,
            });
            let descriptor = converted.fontDescriptor().fontDescriptorWithMatrix(&transform);
            converted = NSFont::fontWithDescriptor_size(&descriptor, size)
                .unwrap_or(converted);
        }
        storage.addAttribute_value_range(NSFontAttributeName, &converted, run);
        cursor = next;
    }
}
/// The font of hidden syntax. Asking AppKit for it again for every hidden
/// range took a third of a styling pass in the running app.
fn concealed_font() -> Retained<NSFont> {
    thread_local! {
        static FONT: Retained<NSFont> = NSFont::systemFontOfSize(0.01);
    }
    FONT.with(Clone::clone)
}
unsafe fn conceal(storage: &objc2_foundation::NSMutableAttributedString, range: NSRange, collapse_lines: bool) {
    if range.length == 0 {
        return;
    }
    storage.addAttribute_value_range(NSFontAttributeName, &concealed_font(), range);
    storage.addAttribute_value_range(
        NSForegroundColorAttributeName,
        &NSColor::clearColor(),
        range,
    );
    storage.removeAttribute_range(NSBackgroundColorAttributeName, range);
    if collapse_lines {
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setMinimumLineHeight(0.01);
        paragraph.setMaximumLineHeight(0.01);
        paragraph.setParagraphSpacing(0.0);
        paragraph.setLineSpacing(0.0);
        storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
    }
}

// A writing line made entirely of hidden syntax still needs the font metrics
// of its normal text. Keep one transparent glyph at full size and cancel its
// advance so it does not move the typing position.
unsafe fn conceal_editable_syntax(storage: &objc2_foundation::NSMutableAttributedString, source: &str, mut hidden: Vec<Range<usize>>) {
    hidden.sort_unstable_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in hidden {
        if let Some(previous) = merged.last_mut().filter(|previous| range.start <= previous.end) {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }

    let mut anchors = Vec::new();
    let mut ranges = merged.iter().peekable();
    let mut start = 0;
    for line in source.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let end = start + content.len();
        while ranges.peek().is_some_and(|range| range.end <= start) {
            ranges.next();
        }
        if !content.is_empty() && ranges.peek().is_some_and(|range| range.start <= start && end <= range.end) {
            if let Some((offset, _)) = content.char_indices().next_back() {
                let bytes = start + offset..end;
                let anchor = utf16_range(source, bytes.clone());
                let font: Retained<NSFont> = msg_send![storage, attribute:NSFontAttributeName, atIndex:anchor.location, effectiveRange:std::ptr::null_mut::<NSRange>()];
                let attrs = NSDictionary::from_slices(&[NSFontAttributeName], &[&*font as &AnyObject]);
                let width = NSString::from_str(&source[bytes]).sizeWithAttributes(Some(&attrs)).width;
                anchors.push((anchor, font, width));
            }
        }
        start += line.len();
    }

    for range in merged {
        conceal(storage, utf16_range(source, range), false);
    }
    for (range, font, width) in anchors {
        storage.addAttribute_value_range(NSFontAttributeName, &font, range);
        storage.addAttribute_value_range(
            NSKernAttributeName,
            &objc2_foundation::NSNumber::new_f64(-width),
            range,
        );
    }
}

/// Byte offset where the note's trailing run of blank lines begins
/// (the source length when the last line has content).
fn trailing_blank_start(source: &str) -> usize {
    let mut start = 0;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        offset += line.len();
        if !line.trim().is_empty() {
            start = offset;
        }
    }
    start
}
/// Byte range of each line's last character through its line ending.
fn line_tail_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let body = line.trim_end_matches(['\n', '\r']);
        if let Some((last, _)) = body.char_indices().next_back() {
            ranges.push(offset + last..offset + line.len());
        } else if !line.is_empty() {
            ranges.push(offset..offset + line.len());
        }
        offset += line.len();
    }
    ranges
}

fn hidden_syntax(source: &str, active: Option<&Range<usize>>) -> Vec<Range<usize>> {
    let mut hidden = Vec::new();
    for span in spans(source) {
        if intersects(&span.bytes, active)
            && matches!(span.treatment, Treatment::Table | Treatment::Image)
        {
            continue;
        }
        let raw = &source[span.bytes.clone()];
        let start = span.bytes.start;
        let trimmed = raw.trim_end_matches(['\r', '\n']);
        match span.treatment {
            Treatment::Underline => {
                hidden.push(start..start + 3);
                hidden.push(span.bytes.end - 4..span.bytes.end);
            }
            Treatment::Heading(_) => {
                if let Some((prefix, _)) = heading_prefix(raw) {
                    hidden.push(start..start + prefix);
                } else if let Some(line) = trimmed.rfind('\n') {
                    hidden.push(start + line + 1..start + trimmed.len());
                }
                let end = trimmed.trim_end();
                let hashes = end.bytes().rev().take_while(|b| *b == b'#').count();
                if hashes > 0
                    && end.len() > hashes
                    && end.as_bytes()[end.len() - hashes - 1].is_ascii_whitespace()
                {
                    hidden.push(start + end.len() - hashes..start + end.len());
                }
            }
            Treatment::Bold | Treatment::Italic | Treatment::Strike => {
                let count = if matches!(span.treatment, Treatment::Bold | Treatment::Strike) {
                    2
                } else {
                    1
                };
                if raw.len() >= count * 2 {
                    hidden.push(start..start + count);
                    hidden.push(span.bytes.end - count..span.bytes.end);
                }
            }
            // Quote prefixes are concealed only by markdown_shortcuts::blocks,
            // which also supplies their indent and bar. CommonMark accepts a
            // bare `>` (including nested `>>`); hiding it here would erase typed
            // feedback before the space commits our editor's quote shortcut.
            Treatment::Quote => {}
            Treatment::Link(_) | Treatment::Image => {
                let prefix = if raw.starts_with("![") { 2 } else { 1 };
                if let Some(close) = raw.rfind("](") {
                    hidden.push(start..start + prefix);
                    hidden.push(start + close..span.bytes.end);
                }
            }
            Treatment::Code => {
                let marks = raw.bytes().take_while(|b| *b == b'`').count();
                if marks > 0 && raw.len() >= marks * 2 {
                    hidden.push(start..start + marks);
                    hidden.push(span.bytes.end - marks..span.bytes.end);
                }
            }
            // Fenced syntax is hidden from code_blocks' validated opener and
            // closer ranges below. A trailing ~ can be actual code.
            Treatment::CodeBlock => {}
            _ => {}
        }
    }
    for (event, range) in Parser::new_ext(source, Options::ENABLE_TASKLISTS).into_offset_iter() {
        if !matches!(event, Event::TaskListMarker(_)) {
            continue;
        }
        let line_start = source[..range.start]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &source[line_start..range.start];
        let leading = prefix
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        let start = if prefix[leading..].trim_end().ends_with(['-', '*', '+']) {
            line_start + leading
        } else {
            range.start
        };
        let end = range.end + usize::from(source.as_bytes().get(range.end) == Some(&b' '));
        hidden.push(start..end);
    }
    hidden
}

// Read layout-fragment coordinates rather than IME candidate rectangles.
// firstRectForCharacterRange may return an upstream visible glyph for a fully
// concealed run, which places overlays on the preceding paragraph.
unsafe fn layout_fragment(text: &NSTextView, index: usize) -> Option<Retained<NSTextLayoutFragment>> {
    let manager = text.textLayoutManager()?;
    let content = manager.textContentManager()?;
    let document = content.documentRange();
    let location = content.locationFromLocation_withOffset(&document.location(), index as isize)?;
    manager.textLayoutFragmentForLocation(&location)
}

unsafe fn layout_fragment_rect(text: &NSTextView, index: usize) -> Option<NSRect> {
    let frame = layout_fragment(text, index)?.layoutFragmentFrame();
    let origin = text.textContainerOrigin();
    Some(NSRect::new(
        NSPoint::new(origin.x + frame.origin.x, origin.y + frame.origin.y),
        NSSize::new(frame.size.width, frame.size.height),
    ))
}

/// Text that shares a row with an attachment: its extra indent and, in the
/// left column, where its lines end.
#[derive(Clone, Debug)]
struct SideText {
    bytes: Range<usize>,
    left: f64,
    right: Option<f64>,
    /// UTF-16 location of the attachment's anchor line and the room it needs.
    anchor: usize,
    reserve: f64,
}

fn standalone(source: &str, bytes: &Range<usize>) -> bool {
    let start = source[..bytes.start].rfind('\n').map_or(0, |n| n + 1);
    let end = source[bytes.end..].find('\n').map_or(source.len(), |n| bytes.end + n);
    source[start..bytes.start].trim().is_empty() && source[bytes.end..end].trim().is_empty()
}

/// The note as blocks, from the structures this editor draws itself.
fn note_structure(source: &str) -> Vec<note_blocks::Block> {
    use note_blocks::Kind;
    let mut structures: Vec<(Range<usize>, Kind)> =
        code_blocks(source).into_iter().map(|code| (code.bytes, Kind::Code)).collect();
    structures.extend(table_previews(source).into_iter().map(|table| (table.bytes, Kind::Table)));
    structures.extend(
        image_edit::markdown::images(source).into_iter()
            .filter(|image| standalone(source, &image.bytes))
            .map(|image| (image.bytes, Kind::Image)),
    );
    structures.extend(
        spans(source).into_iter()
            .filter(|span| span.treatment == Treatment::Rule)
            .map(|span| (span.bytes, Kind::Rule)),
    );
    structures.sort_by_key(|(range, _)| range.start);
    let mut end = 0;
    structures.retain(|(range, _)| {
        let outside = range.start >= end;
        if outside { end = range.end; }
        outside
    });
    note_blocks::blocks(source, &structures)
}

#[derive(Clone, Copy)]
enum Attachment {
    Image(usize),
    Table(usize),
}

/// Give every rendered attachment its column. A placed attachment shares its
/// row with the next block: another attachment moves beside it, and text is
/// returned so its paragraphs can be indented into the other column.
fn arrange_rows(
    source: &str,
    images: &mut [InlineImageSpec],
    tables: &mut [InlineTableSpec],
    body: f64,
    handle_room: f64,
) -> Vec<SideText> {
    use note_blocks::Kind;
    let note = note_structure(source);
    let attachments: Vec<Option<Attachment>> = note.iter().map(|block| match block.kind {
        Kind::Image => images.iter().position(|i| block.bytes.contains(&i.link.bytes.start)).map(Attachment::Image),
        Kind::Table => tables.iter().position(|t| block.bytes.contains(&t.bytes.start)).map(Attachment::Table),
        _ => None,
    }).collect();
    let rows = note_blocks::rows(&note, |index| attachments[index].map(|a| match a {
        Attachment::Image(i) => image_edit::placement(&images[i].link),
        Attachment::Table(t) => tables[t].placement(),
    }));
    let half = ((body + COLUMN_GAP) / 2.0).round();
    let narrow = (body - COLUMN_GAP - MIN_COLUMN_WIDTH).max(80.0);
    let mut sides = Vec::new();
    for row in rows {
        let Some(media) = attachments[row.media] else { continue };
        if let Attachment::Image(i) = media {
            let link = images[i].link.bytes.clone();
            let line = source[link.end..].find('\n').map_or(source.len(), |n| link.end + n + 1);
            images[i].separator = (line < note[row.media].bytes.end).then(|| line..note[row.media].bytes.end);
        }
        let mut fit = |attachment, x: f64, limit: f64| match attachment {
            Attachment::Image(i) => images[i].fit(x, limit),
            Attachment::Table(t) => tables[t].fit(x, limit, handle_room),
        };
        let beside = row.side.and_then(|side| attachments[side]);
        let mut text = None;
        match (row.placement, beside, row.side) {
            (Placement::Left, Some(side), _) => {
                let width = fit(media, 0.0, narrow);
                fit(side, width + COLUMN_GAP, body - width - COLUMN_GAP);
            }
            (Placement::Right, Some(side), _) => {
                let width = fit(side, 0.0, narrow);
                fit(media, width + COLUMN_GAP, body - width - COLUMN_GAP);
            }
            (Placement::Left, None, Some(side)) => {
                let width = fit(media, 0.0, narrow);
                text = Some((side, width + COLUMN_GAP, None));
            }
            (Placement::Right, None, side) => {
                fit(media, half, body - half);
                text = side.map(|side| (side, 0.0, Some(half - COLUMN_GAP - LINE_PADDING)));
            }
            _ => {
                fit(media, 0.0, body);
            }
        }
        let reserve = |attachment| match attachment {
            Attachment::Image(i) => images[i].size.height + IMAGE_PADDING * 2.0,
            Attachment::Table(t) => tables[t].reserve(handle_room),
        };
        // The row's last anchor line reserves the room; earlier ones collapse
        // so both columns start at the same height.
        let room = reserve(media).max(beside.map_or(0.0, reserve));
        let anchor = match media {
            Attachment::Image(i) => images[i].range.location,
            Attachment::Table(t) => tables[t].anchor(source).location,
        };
        let mut reserved = |attachment, spacing: f64| match attachment {
            Attachment::Image(i) => images[i].spacing = spacing,
            Attachment::Table(t) => tables[t].spacing = spacing,
        };
        reserved(media, if row.side.is_some() { 0.0 } else { room });
        if let Some(side) = beside {
            reserved(side, room);
        }
        if let Some((side, left, right)) = text {
            sides.push(SideText { bytes: note[side].bytes.clone(), left, right, anchor, reserve: room });
        }
    }
    sides
}

/// Change the paragraph style of each source line while keeping the rest of it.
unsafe fn adjust_paragraphs(
    storage: &objc2_foundation::NSMutableAttributedString,
    source: &str,
    bytes: Range<usize>,
    edit: impl Fn(&NSMutableParagraphStyle),
) {
    let mut offset = bytes.start;
    for line in source[bytes].split_inclusive('\n') {
        let range = utf16_range(source, offset..offset + line.len());
        offset += line.len();
        if range.length == 0 || range.location >= storage.length() {
            continue;
        }
        let current: Option<Retained<NSParagraphStyle>> = msg_send![storage, attribute:NSParagraphStyleAttributeName, atIndex:range.location, effectiveRange:std::ptr::null_mut::<NSRange>()];
        let style: Retained<NSMutableParagraphStyle> = match current {
            Some(current) => msg_send![&*current, mutableCopy],
            None => NSMutableParagraphStyle::new(),
        };
        edit(&style);
        storage.addAttribute_value_range(NSParagraphStyleAttributeName, &style, range);
    }
}

/// Apply live Markdown attributes only after the input method commits its text.
/// Call when the string, selection, or document width changes.
pub unsafe fn style(text: &NSTextView) {
    let is_native: bool = msg_send![text,isKindOfClass:MarkdownEditor::class()];
    if is_native {
        let native = &*(text as *const NSTextView as *const MarkdownEditor);
        *native.ivars().base_directory.borrow_mut() = None;
    }
    style_with_base(text, None);
}

/// Local Markdown image paths resolve relative to the saved document directory.
/// Network images are never fetched while typing.
pub unsafe fn style_with_base(text: &NSTextView, base_directory: Option<&Path>) {
    layout_markdown(text, base_directory, None);
}

/// Style only when the text, selection, focus or width differs from the last
/// pass. For callers that poll; after any other change call `style_with_base`.
pub unsafe fn style_if_changed(text: &NSTextView, base_directory: Option<&Path>) {
    let is_native: bool = msg_send![text, isKindOfClass:MarkdownEditor::class()];
    if is_native {
        let native = &*(text as *const NSTextView as *const MarkdownEditor);
        let source = text.string().to_string();
        // Without a directory of its own a pass keeps the remembered one.
        let same_base = base_directory.is_none_or(|base| native.ivars().base_directory.borrow().as_deref() == Some(base));
        if same_base && native.ivars().rendered.get() == Some(render_key(text, &source)) {
            return;
        }
    }
    style_with_base(text, base_directory);
}

unsafe fn editor_focused(text: &NSTextView) -> bool {
    text.window().is_some_and(|window| {
        let responder: *mut AnyObject = msg_send![&*window, firstResponder];
        responder == text as *const NSTextView as *mut AnyObject
    })
}

unsafe fn render_key(text: &NSTextView, source: &str) -> RenderKey {
    let selection: NSRange = msg_send![text, selectedRange];
    RenderKey {
        text: fingerprint(source),
        length: source.len(),
        selection: (selection.location, selection.length),
        focused: editor_focused(text),
        editable: text.isEditable(),
        windowed: text.window().is_some(),
        width: text.bounds().size.width,
    }
}

/// Copy the styled attributes onto the note, touching only the lines whose
/// attributes differ. TextKit lays out again whatever a storage edit covers,
/// even when the values are the same, so an untouched line keeps its layout.
unsafe fn apply_styles(live: &NSTextStorage, styled: &objc2_foundation::NSMutableAttributedString, source: &str) {
    if live.length() != styled.length() {
        return;
    }
    let keys = [
        NSFontAttributeName, NSForegroundColorAttributeName, NSBackgroundColorAttributeName,
        NSLinkAttributeName, NSUnderlineStyleAttributeName, NSStrikethroughStyleAttributeName,
        NSParagraphStyleAttributeName, NSKernAttributeName,
    ];
    // `wanted` holds the note's text from `shift` on.
    let matches = |wanted: &objc2_foundation::NSAttributedString, shift: usize, range: NSRange| {
        let end = range.location + range.length;
        let mut cursor = range.location;
        while cursor < end {
            let (mut wanted_run, mut current_run) = (NSRange::new(0, 0), NSRange::new(0, 0));
            let wanted: Retained<NSDictionary<NSString, AnyObject>> = msg_send![wanted, attributesAtIndex:cursor - shift, effectiveRange:&mut wanted_run];
            let current: Retained<NSDictionary<NSString, AnyObject>> = msg_send![live, attributesAtIndex:cursor, effectiveRange:&mut current_run];
            let same: bool = msg_send![&*wanted, isEqualToDictionary:&*current];
            // The note may carry attributes this editor does not manage.
            if !same && !keys.iter().all(|key| match (wanted.objectForKey(key), current.objectForKey(key)) {
                (None, None) => true,
                (Some(wanted), Some(current)) => msg_send![&*wanted, isEqual:&*current],
                _ => false,
            }) {
                return false;
            }
            cursor = (shift + wanted_run.location + wanted_run.length)
                .min(current_run.location + current_run.length)
                .max(cursor + 1);
        }
        true
    };
    // One editing group per run of changed lines: a group reports a single
    // range, and two distant lines in it would cover everything between them.
    let mut editing = false;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let range = utf16_range(source, offset..offset + line.len());
        offset += line.len();
        let mut same = matches(styled, 0, range);
        if !same {
            // The note's storage substitutes fonts for characters its font
            // lacks, some time after an edit. A line it has already fixed is
            // unchanged too; what gets applied stays the unfixed style.
            let fixed: Retained<objc2_foundation::NSMutableAttributedString> =
                msg_send![&*styled.attributedSubstringFromRange(range), mutableCopy];
            let _: () = msg_send![&*fixed, fixAttributesInRange:NSRange::new(0, range.length)];
            same = matches(&fixed, range.location, range);
        }
        if same {
            if std::mem::take(&mut editing) {
                live.endEditing();
            }
            continue;
        }
        if !std::mem::replace(&mut editing, true) {
            live.beginEditing();
        }
        for key in [
            NSBackgroundColorAttributeName, NSLinkAttributeName, NSUnderlineStyleAttributeName,
            NSStrikethroughStyleAttributeName, NSParagraphStyleAttributeName,
        ] {
            live.removeAttribute_range(key, range);
        }
        let end = range.location + range.length;
        let mut cursor = range.location;
        while cursor < end {
            let mut run = NSRange::new(0, 0);
            let wanted: Retained<NSDictionary<NSString, AnyObject>> = msg_send![styled, attributesAtIndex:cursor, effectiveRange:&mut run];
            let next = (run.location + run.length).min(end).max(cursor + 1);
            live.addAttributes_range(&wanted, NSRange::new(cursor, next - cursor));
            cursor = next;
        }
    }
    if editing {
        live.endEditing();
    }
}

/// A drag changes only the presentation. Keep all source ranges anchored to the
/// original document until mouse-up, including the paragraph after the table.
unsafe fn layout_markdown(text: &NSTextView, base_directory: Option<&Path>, preview: Option<TableSizePreview>) {
    if table_cell_is_editing(text) || image_edit::interacting() {
        return;
    }
    render_markdown(text, base_directory, preview);
}

// The drag preview has its own text storage and can render while the live
// editor holds its overlays stable for mouse tracking.
unsafe fn render_markdown(text: &NSTextView, base_directory: Option<&Path>, preview: Option<TableSizePreview>) {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked {
        return;
    }
    let is_native: bool = msg_send![text, isKindOfClass:MarkdownEditor::class()];
    let native = is_native.then(|| &*(text as *const NSTextView as *const MarkdownEditor));
    let remembered_base = native.and_then(|n| n.ivars().base_directory.borrow().clone());
    let base_directory = base_directory.or(remembered_base.as_deref());
    if let Some(native) = native {
        *native.ivars().base_directory.borrow_mut() = base_directory.map(Path::to_path_buf);
        native.ivars().code_panels.borrow_mut().clear();
        native.ivars().line_numbers.borrow_mut().clear();
        native.ivars().list_labels.borrow_mut().clear();
        native.ivars().quote_bars.borrow_mut().clear();
        native.ivars().rule_bars.borrow_mut().clear();
        native.ivars().block_cache.borrow_mut().take();
    }
    let source = text.string().to_string();
    let _offsets = OffsetTable::new(&source);
    let key = render_key(text, &source);
    let source_mark = key.text;
    let grid_focus = text.window().and_then(|w| w.firstResponder()).and_then(|r| r.downcast::<InlineTable>().ok()).filter(|v|v.superview().is_some_and(|p|std::ptr::eq(&*p,text as &NSView))).map(|v| {
        (v.ivars().range.get().location,v.cells().iter().filter(|c|c.ivars().selected.get()).map(|c|c.position()).collect::<Vec<_>>())
    });
    // Tables, checkboxes and code buttons from the last pass are used again
    // where they still fit; building them costs more than styling the text.
    let mut old_tables: Vec<Retained<InlineTable>> = Vec::new();
    let mut old_tasks: std::collections::VecDeque<Retained<TaskCheckbox>> = Default::default();
    let mut old_languages: Vec<Retained<CodeLanguageButton>> = Vec::new();
    let mut old_copies: std::collections::VecDeque<Retained<CodeCopyButton>> = Default::default();
    for child in text.subviews() {
        let child = match child.downcast::<InlineTable>() { Ok(view) => { old_tables.push(view); continue } Err(child) => child };
        let child = match child.downcast::<TaskCheckbox>() { Ok(view) => { old_tasks.push_back(view); continue } Err(child) => child };
        let child = match child.downcast::<CodeLanguageButton>() { Ok(view) => { old_languages.push(view); continue } Err(child) => child };
        let child = match child.downcast::<CodeCopyButton>() { Ok(view) => { old_copies.push_back(view); continue } Err(child) => child };
        let inline: bool = msg_send![&*child, isKindOfClass: InlineImage::class()];
        let placeholder: bool = msg_send![&*child, isKindOfClass: HeadingPlaceholder::class()];
        if inline || placeholder {
            child.removeFromSuperview();
        }
    }
    let Some(live) = text.textStorage() else {
        return;
    };
    // Style a detached copy; only the lines that come out different are
    // copied onto the note afterwards.
    let storage = objc2_foundation::NSMutableAttributedString::initWithString(
        objc2_foundation::NSMutableAttributedString::alloc(),
        &text.string(),
    );
    let selection: NSRange = msg_send![text, selectedRange];
    let focused = editor_focused(text);
    let active = if text.isEditable() && focused {
        Some(active_block(&source, selection))
    } else {
        None
    };
    let all = NSRange::new(0, source.encode_utf16().count());
    let base = crate::font::regular(16.0);
    let ink = NSColor::colorWithSRGBRed_green_blue_alpha(0.09, 0.094, 0.149, 1.0);
    let indigo = NSColor::colorWithSRGBRed_green_blue_alpha(0.31, 0.275, 0.898, 1.0);
    let secondary = NSColor::colorWithSRGBRed_green_blue_alpha(0.40, 0.42, 0.50, 1.0);
    let paper = NSColor::colorWithSRGBRed_green_blue_alpha(0.957, 0.961, 0.980, 1.0);
    let mono = NSFont::monospacedSystemFontOfSize_weight(14.0, 0.0);

    // Disable undo registration for presentation changes only. Do not clear the
    // undo manager, which contains the user's character edits and IME commits.
    let undo: Option<Retained<AnyObject>> = msg_send![text, undoManager];
    if let Some(undo) = &undo {
        let _: () = msg_send![undo, disableUndoRegistration];
    }
    storage.addAttribute_value_range(NSFontAttributeName, &base, all);
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setLineSpacing(6.0);
    paragraph.setParagraphSpacing(NOTE_PARAGRAPH_SPACING);
    storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, all);
    // Empty writing rows retain spacing without stretching the insertion cursor.
    let blank_paragraph = NSMutableParagraphStyle::new();
    blank_paragraph.setParagraphSpacing(6.0);
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        if line.trim().is_empty() {
            storage.addAttribute_value_range(
                NSParagraphStyleAttributeName,
                &blank_paragraph,
                utf16_range(&source, offset..offset + line.len()),
            );
        }
        offset += line.len();
    }
    storage.addAttribute_value_range(NSKernAttributeName, &objc2_foundation::NSNumber::new_f64(0.2), all);
    // Spacing after a line's last character leaves a hairline gap between the
    // text's selection highlight and the highlight that runs to the line end.
    for range in line_tail_ranges(&source) {
        storage.removeAttribute_range(NSKernAttributeName, utf16_range(&source, range));
    }
    storage.addAttribute_value_range(NSForegroundColorAttributeName, &ink, all);
    let blocks = markdown_shortcuts::blocks(&source);
    for block in blocks.iter().filter(|block| block.quote_depth > 0) {
        storage.addAttribute_value_range(
            NSForegroundColorAttributeName, &secondary,
            utf16_range(&source, block.bytes.clone()),
        );
    }
    for span in spans(&source) {
        let range = utf16_range(&source, span.bytes.clone());
        if range.length == 0 {
            continue;
        }
        match span.treatment {
            Treatment::Heading(level) => {
                let size = heading_size(level);
                storage.addAttribute_value_range(
                    NSFontAttributeName,
                    &crate::font::bold(size),
                    range,
                );
            }
            Treatment::Bold => apply_font_trait(&storage, range, NSFontTraitMask::BoldFontMask),
            Treatment::Italic => apply_font_trait(&storage, range, NSFontTraitMask::ItalicFontMask),
            Treatment::Strike | Treatment::Underline => {
                let one = objc2_foundation::NSNumber::new_isize(1);
                let key = if span.treatment == Treatment::Underline {
                    NSUnderlineStyleAttributeName
                } else {
                    NSStrikethroughStyleAttributeName
                };
                storage.addAttribute_value_range(key, &one, range);
            }
            Treatment::Code | Treatment::CodeBlock | Treatment::Table => {
                storage.addAttribute_value_range(NSFontAttributeName, &mono, range);
                storage.addAttribute_value_range(NSBackgroundColorAttributeName, &paper, range);
            }
            // Quote color is applied from committed visual blocks below the
            // base attributes, before inline colors, rather than parser spans.
            Treatment::Quote => {}
            Treatment::List => {}
            Treatment::Link(url) => {
                storage.addAttribute_value_range(NSForegroundColorAttributeName, &indigo, range);
                if url.starts_with("https://")
                    || url.starts_with("http://")
                    || url.starts_with("mailto:")
                {
                    storage.addAttribute_value_range(
                        NSLinkAttributeName,
                        &NSString::from_str(&url),
                        range,
                    );
                }
            }
            Treatment::Image => {
                storage.addAttribute_value_range(NSForegroundColorAttributeName, &indigo, range)
            }
            Treatment::Rule => {
                storage.addAttribute_value_range(NSForegroundColorAttributeName, &secondary, range)
            }
        }
    }
    // SUIT draws directional arrows as chevrons. Substitute only those glyphs,
    // preserving the surrounding run's size and weight.
    let manager = NSFontManager::sharedFontManager(MainThreadMarker::new().unwrap());
    for (offset, ch) in source.char_indices().filter(|(_, ch)| "→←↔⇒⇐⇔".contains(*ch)) {
        let range = utf16_range(&source, offset..offset + ch.len_utf8());
        let current: Retained<NSFont> = msg_send![&*storage, attribute:NSFontAttributeName, atIndex:range.location, effectiveRange:std::ptr::null_mut::<NSRange>()];
        let fallback = NSFont::systemFontOfSize(current.pointSize());
        let fallback = manager.convertFont_toHaveTrait(&fallback, manager.traitsOfFont(&current));
        storage.addAttribute_value_range(NSFontAttributeName, &fallback, range);
    }
    let rules: Vec<_> = spans(&source).into_iter()
        .filter(|span| span.treatment == Treatment::Rule).collect();
    for rule in &rules {
        let range = utf16_range(&source, rule.bytes.clone());
        conceal(&storage, range, false);
        // Hidden marker glyphs must not collapse the rule's line or pull the
        // following empty paragraph's caret onto the divider.
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setMinimumLineHeight(24.0);
        paragraph.setMaximumLineHeight(24.0);
        paragraph.setParagraphSpacing(8.0);
        storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
    }
    for token in code_highlight::highlight_markdown(&source) {
        let [r, g, b] = token.rgb;
        storage.addAttribute_value_range(
            NSForegroundColorAttributeName,
            &NSColor::colorWithSRGBRed_green_blue_alpha(
                r as f64 / 255.0,
                g as f64 / 255.0,
                b as f64 / 255.0,
                1.0,
            ),
            utf16_range(&source, token.bytes),
        );
    }
    let mut hidden = hidden_syntax(&source, active.as_ref());
    for (index, block) in blocks.iter().enumerate() {
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setFirstLineHeadIndent(block.indent);
        paragraph.setHeadIndent(block.indent);
        paragraph.setLineSpacing(6.0);
        if block.is_list {
            // Only reserve space when another item follows. Keeping it after
            // the paragraph also keeps custom markers on the first text row.
            if blocks.get(index + 1).is_some_and(|next| {
                next.is_list && next.bytes.start == block.bytes.end
            }) {
                paragraph.setParagraphSpacing(NOTE_LIST_SPACING);
            }
        } else {
            paragraph.setParagraphSpacing(NOTE_PARAGRAPH_SPACING);
        }
        storage.addAttribute_value_range(
            NSParagraphStyleAttributeName,
            &paragraph,
            utf16_range(&source, block.bytes.clone()),
        );
        hidden.push(block.prefix.clone());
    }
    conceal_editable_syntax(&storage, &source, hidden);
    let codes = code_blocks(&source);
    for code in &codes {
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setHeadIndent(48.0);
        paragraph.setFirstLineHeadIndent(48.0);
        paragraph.setTailIndent(-16.0);
        storage.addAttribute_value_range(
            NSParagraphStyleAttributeName,
            &paragraph,
            utf16_range(&source, code.bytes.clone()),
        );
        for (fence, height) in [(&code.open, 38.0), (&code.close, 14.0)] {
            if fence.is_empty() {
                continue;
            }
            let range = utf16_range(&source, fence.clone());
            conceal(&storage, range, false);
            let p = NSMutableParagraphStyle::new();
            p.setMinimumLineHeight(height);
            p.setMaximumLineHeight(height);
            storage.addAttribute_value_range(NSParagraphStyleAttributeName, &p, range);
        }
    }
    let body = (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0).max(80.0);
    let mut image_specs = inline_images(&source, base_directory, body);
    let mut tables = table_previews(&source)
        .into_iter()
        .filter(|table| preview.as_ref().is_some_and(|p| p.start == table.bytes.start) || !intersects(&table.bytes, active.as_ref()))
        .map(|mut table| {
            if let Some(size) = preview.as_ref().filter(|p| p.start == table.bytes.start) {
                size.apply(&mut table);
            }
            table
        })
        .collect::<Vec<_>>();
    let handle_room = if text.isEditable() { TABLE_HANDLE_SIZE } else { 0.0 };
    let sides = arrange_rows(&source, &mut image_specs, &mut tables, body, handle_room);
    for side in &sides {
        adjust_paragraphs(&storage, &source, side.bytes.clone(), |style| {
            style.setFirstLineHeadIndent(style.firstLineHeadIndent() + side.left);
            style.setHeadIndent(style.headIndent() + side.left);
            if let Some(right) = side.right {
                style.setTailIndent(right + style.tailIndent().min(0.0));
            }
        });
    }
    // The blank line Markdown needs after an attachment is not a writing row.
    // When it ends the note, TextKit still hangs the end-of-document caret
    // line below it; `terminal` lets that offset count toward the reserved room.
    let mut terminal = None;
    for spec in &image_specs {
        conceal(&storage, spec.range, true);
        style_anchor(&storage, spec.range, spec.spacing);
        if let Some(separator) = &spec.separator {
            let range = utf16_range(&source, separator.clone());
            if text.isEditable() && separator.end == source.len() && spec.spacing > 0.0 {
                terminal = Some((spec.range, spec.spacing, range.location));
            }
            conceal(&storage, range, true);
        }
    }
    for table in &tables {
        conceal(&storage, utf16_range(&source, table.bytes.clone()), true);
        if let Some(separator) = table.separator_line(&source) {
            let range = utf16_range(&source, separator.clone());
            if text.isEditable() && separator.end == source.len() && table.spacing > 0.0 {
                terminal = Some((table.anchor(&source), table.spacing, range.location));
            }
            conceal(&storage, range, true);
        }
        style_anchor(&storage, table.anchor(&source), table.spacing);
    }
    apply_styles(&live, &storage, &source);
    // Later corrections depend on the laid-out note and go straight to it.
    let storage = live;
    // Never inherit transparent/tiny attributes from a concealed syntax run.
    let caret = byte_for_utf16(&source, selection.location);
    let typing_font = if let Some((_, _, level)) = heading_at(&source, selection) {
        crate::font::bold(heading_size(level))
    } else if codes
        .iter()
        .any(|code| code.open.end <= caret && caret <= code.close.start.max(code.open.end))
    {
        mono.clone()
    } else {
        base.clone()
    };
    let typing = NSDictionary::from_slices(
        &[NSFontAttributeName, NSForegroundColorAttributeName],
        &[&*typing_font as &AnyObject, &*ink as &AnyObject],
    );
    text.setTypingAttributes(&typing);
    // Attribute edits can invalidate multiple downstream TextKit 2 fragments.
    // Complete that reflow synchronously before attaching views to fragments.
    if let Some(manager) = text.textLayoutManager() {
        if let Some(content) = manager.textContentManager() {
            let document = content.documentRange();
            manager.ensureLayoutForRange(&document);
            if let Some((anchor, spacing, separator)) = terminal {
                // The caret line hangs a full row below the hidden separator.
                // Subtract that from the reservation so the first typed
                // character stays where the empty caret was.
                if let Some(fragment) = layout_fragment(text, separator) {
                    let lines = fragment.textLineFragments();
                    if lines.count() > 1 {
                        // A typed line starts below its 6pt line spacing; the
                        // empty caret line does not.
                        let inset = lines.lastObject().unwrap().typographicBounds().origin.y - 6.0;
                        style_anchor(&storage, anchor, spacing - inset);
                        manager.ensureLayoutForRange(&document);
                    }
                }
            }
            // A side block shorter than its attachment still ends the row
            // below it. Later rows only move down, so one pass is enough.
            let mut grown = false;
            for side in &sides {
                let Some(top) = layout_fragment_rect(text, side.anchor) else { continue };
                let end = utf16_offset(&source, side.bytes.end);
                let bottom = if side.bytes.end < source.len() {
                    layout_fragment_rect(text, end).map(|r| r.origin.y)
                } else if source.ends_with('\n') {
                    // The note ends here: the empty caret line follows the
                    // spacing, 6pt above where a typed line would start.
                    layout_fragment(text, end.saturating_sub(1)).and_then(|fragment| {
                        let line = fragment.textLineFragments().lastObject()?.typographicBounds();
                        Some(text.textContainerOrigin().y + fragment.layoutFragmentFrame().origin.y + line.origin.y - 6.0)
                    })
                } else {
                    layout_fragment_rect(text, end.saturating_sub(1)).map(|r| r.origin.y + r.size.height)
                };
                let missing = bottom.map_or(0.0, |bottom| top.origin.y + side.reserve - bottom);
                if missing > 0.5 {
                    let content = source[side.bytes.clone()].trim_end_matches(['\r', '\n']);
                    let last = side.bytes.start + content.rfind('\n').map_or(0, |n| n + 1);
                    adjust_paragraphs(&storage, &source, last..side.bytes.end, |style| {
                        style.setParagraphSpacing(style.paragraphSpacing() + missing);
                    });
                    grown = true;
                }
            }
            if grown {
                manager.ensureLayoutForRange(&document);
            }
        }
    }
    if let Some(undo) = &undo {
        let _: () = msg_send![undo, enableUndoRegistration];
    }
    let shift = |byte: usize| sides.iter().find(|s| s.bytes.contains(&byte)).map_or(0.0, |s| s.left);
    // Left edge and width of the column that holds this byte.
    let column = |byte: usize| {
        let full = (text.bounds().size.width - 2.0 * EDITOR_INSET).max(80.0);
        match sides.iter().find(|s| s.bytes.contains(&byte)) {
            Some(s) => (EDITOR_INSET + s.left, s.right.map_or(full - s.left, |right| right + LINE_PADDING)),
            None => (EDITOR_INSET, full),
        }
    };
    if let Some(native) = native {
        for rule in &rules {
            if let Some(frame) = layout_fragment_rect(text, utf16_offset(&source, rule.bytes.start)) {
                native.ivars().rule_bars.borrow_mut().push(NSRect::new(
                    NSPoint::new(EDITOR_INSET + LINE_PADDING, frame.origin.y + frame.size.height / 2.0),
                    NSSize::new((text.bounds().size.width - 2.0 * (EDITOR_INSET + LINE_PADDING)).max(1.0), 1.0),
                ));
            }
        }
        for block in &blocks {
            let Some(frame) =
                layout_fragment_rect(text, utf16_offset(&source, block.prefix.start))
            else {
                continue;
            };
            if let Some(label) = &block.label {
                let size = if label == "•" { (28.0_f64 - 4.0 * block.list_depth as f64).max(16.0) } else { 16.0 };
                let width = label.chars().count() as f64 * 9.0;
                let line_inset = layout_fragment(text, utf16_offset(&source, block.prefix.start))
                    .and_then(|fragment| fragment.textLineFragments().firstObject())
                    .map_or(0.0, |line| line.typographicBounds().origin.y);
                native.ivars().list_labels.borrow_mut().push((
                    label.clone(),
                    NSPoint::new(
                        EDITOR_INSET + LINE_PADDING + shift(block.bytes.start) + block.indent - width - 4.0,
                        frame.origin.y + if label == "•" { line_inset - 3.0 - (size - 16.0) / 2.0 } else { 5.0 },
                    ),
                    size,
                ));
            }
            for depth in 0..block.quote_depth {
                native.ivars().quote_bars.borrow_mut().push(NSRect::new(
                    NSPoint::new(
                        EDITOR_INSET + LINE_PADDING + shift(block.bytes.start) + depth as f64 * 20.0,
                        frame.origin.y + 2.0,
                    ),
                    NSSize::new(3.0, (frame.size.height - 4.0).max(20.0)),
                ));
            }
        }
    }
    let image_frames: Vec<Option<NSRect>> = image_specs.iter().map(|spec| {
        layout_fragment_rect(text, spec.range.location).map(|local| NSRect::new(
            NSPoint::new(EDITOR_INSET + spec.x, (local.origin.y + IMAGE_PADDING).max(text.textContainerOrigin().y)),
            spec.size,
        ))
    }).collect();
    if text.window().is_some() {
        let mtm = MainThreadMarker::new().unwrap();
        for code in &codes {
            let Some(top) =
                layout_fragment_rect(text, utf16_range(&source, code.open.clone()).location)
            else {
                continue;
            };
            let bottom_index = utf16_range(&source, code.bytes.clone()).location
                + utf16_range(&source, code.bytes.clone())
                    .length
                    .saturating_sub(1);
            let bottom = layout_fragment_rect(text, bottom_index).unwrap_or(top);
            // A terminal closing fence and the extra editable EOF line share
            // one layout fragment. Keep the following caret outside the panel.
            let footer_height = if !code.close.is_empty() {
                layout_fragment(text, bottom_index)
                    .and_then(|f| f.textLineFragments().firstObject())
                    .map_or(bottom.size.height, |line| line.typographicBounds().size.height)
            } else { bottom.size.height };
            let (left, width) = column(code.bytes.start);
            let panel = NSRect::new(
                NSPoint::new(left, top.origin.y),
                NSSize::new(width, bottom.origin.y + footer_height - top.origin.y),
            );
            if let Some(native) = native {
                native.ivars().code_panels.borrow_mut().push(panel);
            }
            let canonical = code_highlight::canonical_language(&code.language);
            let listed = CODE_LANGUAGES.iter().find(|(id, _)| canonical == *id).map(|(_, title)| *title);
            let language_range = utf16_range(&source, code.open.clone());
            let language_frame = NSRect::new(NSPoint::new(panel.origin.x + 12.0, top.origin.y + 4.0), NSSize::new(160.0_f64.min(panel.size.width - 80.0).max(60.0), 28.0));
            // A menu already lists what its own language needs.
            let kept = old_languages.iter().position(|old| old.ivars().language == code.language);
            let button = match kept {
                Some(index) => {
                    let button = old_languages.remove(index);
                    button.ivars().range.set(language_range);
                    button.ivars().fingerprint.set(source_mark);
                    *button.ivars().fence.borrow_mut() = code.fence.clone();
                    if button.frame() != language_frame {
                        button.setFrame(language_frame);
                    }
                    button
                }
                None => {
                    let allocated = CodeLanguageButton::alloc(mtm).set_ivars(CodeLanguageState {
                        text: Weak::new(text),
                        range: std::cell::Cell::new(language_range),
                        fingerprint: std::cell::Cell::new(source_mark),
                        fence: RefCell::new(code.fence.clone()),
                        language: code.language.clone(),
                    });
                    let button: Retained<CodeLanguageButton> = msg_send![super(allocated), initWithFrame:language_frame, pullsDown:false];
                    for (_, title) in CODE_LANGUAGES {
                        button.addItemWithTitle(&NSString::from_str(title));
                    }
                    if listed.is_none() {
                        button.addItemWithTitle(&NSString::from_str(&code.language));
                    }
                    button.setBordered(false);
                    button.setFont(Some(&crate::font::regular(13.0)));
                    button.setTarget(Some(&button));
                    button.setAction(Some(sel!(changeLanguage:)));
                    text.addSubview(&button);
                    button
                }
            };
            // A change the editor refused leaves the menu on another item.
            let title = listed.unwrap_or(&code.language);
            if button.titleOfSelectedItem().is_none_or(|selected| selected.to_string() != title) {
                button.selectItemWithTitle(&NSString::from_str(title));
            }
            if button.isEnabled() != text.isEditable() {
                button.setEnabled(text.isEditable());
            }
            let content_end = if code.close.is_empty() {
                code.bytes.end
            } else {
                code.close.start
            };
            let body = &source[code.open.end..content_end];
            let body = body.strip_suffix('\n').unwrap_or(body);
            let copy_frame = NSRect::new(NSPoint::new(panel.origin.x + panel.size.width - 64.0, top.origin.y + 6.0), NSSize::new(52.0, 24.0));
            match old_copies.pop_front() {
                Some(copy) => {
                    if *copy.ivars().code.borrow() != body {
                        *copy.ivars().code.borrow_mut() = body.to_string();
                    }
                    if copy.frame() != copy_frame {
                        copy.setFrame(copy_frame);
                    }
                    // Styling again has always put "복사됨" back to "복사".
                    if copy.title().to_string() != "복사" {
                        copy.setTitle(&NSString::from_str("복사"));
                    }
                }
                None => {
                    let allocated = CodeCopyButton::alloc(mtm).set_ivars(CodeCopyState { code: RefCell::new(body.to_string()) });
                    let copy: Retained<CodeCopyButton> = msg_send![super(allocated), initWithFrame:copy_frame];
                    copy.setTitle(&NSString::from_str("복사"));
                    copy.setBordered(false);
                    copy.setFont(Some(&crate::font::regular(12.0)));
                    copy.setTarget(Some(&copy));
                    copy.setAction(Some(sel!(copyCode:)));
                    text.addSubview(&copy);
                }
            }
            let mut offset = code.open.end;
            for (number, line) in body.split_inclusive('\n').enumerate() {
                if let (Some(native), Some(frame)) = (
                    native,
                    layout_fragment_rect(text, utf16_offset(&source, offset)),
                ) {
                    native.ivars().line_numbers.borrow_mut().push((
                        (number + 1).to_string(),
                        NSPoint::new(panel.origin.x + 12.0, frame.origin.y + 1.0),
                    ));
                }
                offset += line.len();
            }
        }
        if let Some((_, content, level)) = heading_at(&source, selection) {
            if content.is_empty() && focused {
                if let Some(frame) =
                    layout_fragment_rect(text, selection.location.saturating_sub(1))
                {
                    // Place the placeholder on the text row with the caret.
                    let line_inset = layout_fragment(text, selection.location.saturating_sub(1))
                        .and_then(|fragment| fragment.textLineFragments().firstObject())
                        .map_or(0.0, |line| line.typographicBounds().origin.y);
                    let allocated = HeadingPlaceholder::alloc(mtm).set_ivars(());
                    let label: Retained<HeadingPlaceholder> = msg_send![super(allocated), initWithFrame:NSRect::new(NSPoint::new(EDITOR_INSET + LINE_PADDING + shift(caret), frame.origin.y + line_inset), NSSize::new(240.0, heading_size(level) + 10.0))];
                    label.setStringValue(&NSString::from_str(&format!("제목 {level}")));
                    label.setFont(Some(&crate::font::bold(heading_size(level))));
                    label.setTextColor(Some(&NSColor::placeholderTextColor()));
                    label.setBezeled(false);
                    label.setDrawsBackground(false);
                    label.setEditable(false);
                    label.setSelectable(false);
                    text.addSubview(&label);
                }
            }
        }
        text.setNeedsDisplay(true);
        if native.is_some() { mark_overlay(text).setNeedsDisplay(true); }
        for (event, bytes) in Parser::new_ext(&source, Options::ENABLE_TASKLISTS).into_offset_iter()
        {
            let Event::TaskListMarker(checked) = event else {
                continue;
            };
            let byte = bytes.start;
            let range = utf16_range(&source, bytes);
            let Some(local) = layout_fragment_rect(text, range.location) else {
                continue;
            };
            let indent = blocks
                .iter()
                .find(|b| b.bytes.start <= byte && byte < b.bytes.end)
                .map_or(0.0, |b| b.indent);
            let frame = NSRect::new(
                NSPoint::new(
                    EDITOR_INSET + LINE_PADDING + shift(byte) + indent - 24.0,
                    local.origin.y + 1.0,
                ),
                NSSize::new(18.0, 18.0),
            );
            let checkbox = match old_tasks.pop_front() {
                Some(checkbox) => {
                    checkbox.ivars().range.set(range);
                    checkbox.ivars().fingerprint.set(source_mark);
                    if checkbox.frame() != frame {
                        checkbox.setFrame(frame);
                    }
                    checkbox
                }
                None => {
                    let allocated = TaskCheckbox::alloc(mtm).set_ivars(CheckboxState {
                        text: Weak::new(text),
                        range: std::cell::Cell::new(range),
                        fingerprint: std::cell::Cell::new(source_mark),
                    });
                    let checkbox: Retained<TaskCheckbox> = msg_send![super(allocated),initWithFrame:frame];
                    checkbox.setTitle(&NSString::from_str(""));
                    checkbox.setButtonType(NSButtonType::Switch);
                    checkbox.setTarget(Some(&checkbox));
                    checkbox.setAction(Some(sel!(toggle:)));
                    text.addSubview(&checkbox);
                    checkbox
                }
            };
            let state = if checked { 1 } else { 0 };
            if checkbox.state() != state {
                checkbox.setState(state);
            }
            if checkbox.isEnabled() != text.isEditable() {
                checkbox.setEnabled(text.isEditable());
            }
        }
        let mut reach: f64 = 0.0;
        for table in tables {
            let Some(local) = layout_fragment_rect(text, table.anchor(&source).location) else {
                continue;
            };
            let widths = table.widths(handle_room);
            let grid_width = widths.iter().sum::<f64>();
            let width = grid_width + handle_room;
            let frame = NSRect::new(
                NSPoint::new(EDITOR_INSET + table.x, local.origin.y + TABLE_TOP_PADDING),
                NSSize::new(width, table.height() + handle_room),
            );
            let table_range = utf16_range(&source, table.bytes.clone());
            let built_from = table.built_from(&source, handle_room, text.isEditable());
            // A drag preview shows sizes the source does not have, and a cell
            // that was opened for editing still shows its Markdown.
            let kept = old_tables.iter().enumerate()
                .filter(|(_, old)| {
                    preview.is_none()
                        && old.ivars().built_from == built_from
                        && old.ivars().base_directory.as_deref() == base_directory
                        && !old.cells().iter().any(|cell| cell.ivars().needs_layout.get())
                })
                .min_by_key(|(_, old)| old.ivars().range.get().location.abs_diff(table_range.location))
                .map(|(index, _)| index);
            if let Some(view) = kept.map(|index| old_tables.swap_remove(index)) {
                view.ivars().range.set(table_range);
                if view.frame().origin != frame.origin {
                    view.setFrameOrigin(frame.origin);
                }
                for cell in view.cells() {
                    let (row, column) = cell.position();
                    cell.ivars().range.set(utf16_range(&source, table.cell_ranges[row][column].clone()));
                    cell.ivars().fingerprint.set(source_mark);
                }
                // As when it was built again: only the grid that holds the
                // keyboard keeps its selected cells.
                let holds_keyboard = view.window().and_then(|w| w.firstResponder())
                    .is_some_and(|r| std::ptr::eq(Retained::as_ptr(&r) as *const AnyObject, Retained::as_ptr(&view) as *const AnyObject));
                if !holds_keyboard {
                    view.clear_selection();
                }
                reach = reach.max(frame.origin.y + frame.size.height);
                continue;
            }
            let allocated = InlineTable::alloc(mtm).set_ivars(InlineTableState {
                text: Weak::new(text),
                range: std::cell::Cell::new(table_range),
                base_directory: base_directory.map(Path::to_path_buf),
                built_from,
            });
            let view: Retained<InlineTable> = msg_send![super(allocated),initWithFrame:frame];
            if text.isEditable() {
                add_table_handle(&view, true, NSRect::new(NSPoint::new(grid_width, 0.0), NSSize::new(handle_room, table.height())));
                add_table_handle(&view, false, NSRect::new(NSPoint::new(0.0, table.height()), NSSize::new(grid_width, handle_room)));
            }
            let mut y = 0.;
            let mut previous_cell: Option<Retained<TableCell>> = None;
            for (row_index, row) in table.rows.iter().enumerate() {
                let mut x = 0.;
                let height = table.row_height(row_index);
                for (column, contents) in row.iter().enumerate() {
                    let cell_range = table.cell_ranges[row_index][column].clone();
                    let allocated = TableCell::alloc(mtm).set_ivars(TableCellState {
                        text: Weak::new(text),
                        range: std::cell::Cell::new(utf16_range(&source, cell_range.clone())),
                        fingerprint: std::cell::Cell::new(source_mark),
                        row: row_index, column, selected: Default::default(),
                        original_display: RefCell::new(contents.clone()),
                        editing_markdown: Default::default(),
                        markdown: RefCell::new(grid_editable_source(&source[cell_range.clone()])),
                        committing: std::cell::Cell::new(false),
                        needs_layout: std::cell::Cell::new(false),
                    });
                    let cell: Retained<TableCell> = msg_send![super(allocated), initWithFrame:NSRect::new(
                        NSPoint::new(x, y),
                        NSSize::new(widths[column] - 1.0, height - 1.0),
                    )];
                    x += widths[column];
                    let centered: Retained<GridFieldCell> = msg_send![GridFieldCell::alloc(mtm), initTextCell:&*NSString::from_str(contents)];
                    centered.setUsesSingleLineMode(false);
                    centered.setWraps(true);
                    cell.setCell(Some(&centered));
                    cell.setAlignment(table.alignment(row_index, column));
                    cell.setStringValue(&NSString::from_str(contents));
                    cell.setBezeled(false);
                    // Parser-inserted missing cells have no actual source slot.
                    // Keep those selectable instead of corrupting another row.
                    cell.setEditable(
                        text.isEditable() && ((row_index == 0 && column == 0 && table.marker.is_some()) || table_cell_has_source_slot(&source, &cell_range)),
                    );
                    cell.setSelectable(true);
                    cell.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*cell)));
                    cell.setTarget(Some(&cell));
                    cell.setAction(Some(sel!(commitCell:)));
                    cell.setDrawsBackground(true);
                    cell.setBackgroundColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
                        if row_index == 0 { 0.91 } else { 0.97 },
                        if row_index == 0 { 0.92 } else { 0.975 },
                        0.99,
                        1.0,
                    )));
                    let font = if row_index == 0 {
                        crate::font::bold(13.0)
                    } else {
                        crate::font::regular(13.0)
                    };
                    cell.setFont(Some(&font));
                    cell.setTextColor(Some(&ink));
                    render_grid_cell(&cell, &table);
                    if let Some(previous) = &previous_cell {
                        previous.setNextKeyView(Some(&cell));
                    }
                    previous_cell = Some(cell.clone());
                    view.addSubview(&cell);
                }
                y += height;
            }
            if text.isEditable() { add_grid_dividers(&view, &widths, &table); }
            text.addSubview(&view);
            reach = reach.max(frame.origin.y + frame.size.height);
            if let Some((_,selected)) = grid_focus.as_ref().filter(|(at,_)|*at == view.ivars().range.get().location) {
                if let (Some(a),Some(b)) = (selected.first(),selected.last()) {
                    view.select(*a,*b);
                    if let Some(w) = view.window() {w.makeFirstResponder(Some(&view));}
                }
            }
        }
        for (spec, frame) in image_specs.into_iter().zip(image_frames) {
            let Some(frame) = frame else {
                continue;
            };
            let allocated = InlineImage::alloc(mtm).set_ivars(ImageState {
                selected: Default::default(),
                text: Weak::new(text),
                range: spec.range,
                limit: spec.limit,
                base: base_directory.map(Path::to_path_buf),
                link: spec.link,
                fingerprint: source_mark,
                targets: Default::default(),
                preview: Default::default(),
                drag: std::cell::Cell::new(None),
            });
            let image_view: Retained<InlineImage> = msg_send![super(allocated),initWithFrame:frame];
            image_view.setImage(Some(&spec.image));
            image_view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            let image_label = if image_view.diagram_link().is_some() {
                let name = spec.image.accessibilityDescription().map(|s| s.to_string()).unwrap_or_default();
                format!("다이어그램 {name} · 모서리와 가장자리로 크기 조절 · 우클릭으로 편집 또는 삭제")
            } else {
                "노트 이미지 · 모서리와 가장자리로 크기 조절 · 우클릭으로 삭제".to_owned()
            };
            image_view.setAccessibilityLabel(Some(&NSString::from_str(&image_label)));
            text.addSubview(&image_view);
            image_view.sync_selection();
            reach = reach.max(frame.origin.y + frame.size.height);
        }
        // An attachment taller than the note's last line must remain scrollable.
        if reach + 30.0 > text.frame().size.height {
            let mut size = text.frame().size;
            size.height = reach + 30.0;
            text.setFrameSize(size);
        }
    }
    for view in old_tables { view.removeFromSuperview(); }
    for view in old_tasks { view.removeFromSuperview(); }
    for view in old_languages { view.removeFromSuperview(); }
    for view in old_copies { view.removeFromSuperview(); }
    if let Some(native) = native {
        *native.ivars().sides.borrow_mut() = sides;
        native.refresh_blocks();
        // A drag preview shows sizes the source does not have yet.
        native.ivars().rendered.set(preview.is_none().then_some(key));
    }
}

// Clicking an inactive preview explicitly enters the corresponding source block.
struct BlockPreviewState {
    text: Weak<NSTextView>,
    range: NSRange,
    base_directory: Option<PathBuf>,
}
unsafe fn enter_source(state: &BlockPreviewState) {
    let Some(text) = state.text.load() else {
        return;
    };
    if !text.isEditable() {
        return;
    }
    let marked: bool = msg_send![&*text, hasMarkedText];
    if marked {
        return;
    }
    text.setSelectedRange(NSRange::new(state.range.location, 0));
    let base = crate::font::regular(16.0);
    let ink = NSColor::colorWithSRGBRed_green_blue_alpha(0.09, 0.094, 0.149, 1.0);
    text.setTypingAttributes(&NSDictionary::from_slices(
        &[NSFontAttributeName, NSForegroundColorAttributeName],
        &[&*base as &AnyObject, &*ink as &AnyObject],
    ));
    if let Some(window) = text.window() {
        window.makeFirstResponder(Some(&text));
    }
    // Reveal synchronously before AppKit can accept the next keyboard/IME
    // event. Waiting for the polling timer can leave the source concealed once
    // marked text starts (styling correctly refuses to touch marked text).
    style_with_base(&text, state.base_directory.as_deref());
}
struct CheckboxState {
    text: Weak<NSTextView>,
    range: std::cell::Cell<NSRange>,
    fingerprint: std::cell::Cell<u64>,
}
fn fingerprint(source: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut state = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut state);
    state.finish()
}
define_class!(
    #[unsafe(super(NSButton,NSControl,NSView,NSResponder))]
    #[name = "StologyMarkdownTaskCheckbox"]
    #[thread_kind = MainThreadOnly]
    #[ivars = CheckboxState]
    struct TaskCheckbox;
    impl TaskCheckbox {
        #[unsafe(method(toggle:))]
        fn toggle(&self,_sender:Option<&AnyObject>) { unsafe {
            let Some(text)=self.ivars().text.load() else {return;};
            if !text.isEditable() {return;}
            let marked:bool=msg_send![&*text,hasMarkedText];
            if marked {return;}
            let source=text.string().to_string();
            if fingerprint(&source)!=self.ivars().fingerprint.get() {return;}
            let selection=text.selectedRanges();
            let replacement=NSString::from_str(if self.state()==1 {"[x]"}else{"[ ]"});
            let _:()=msg_send![&*text,insertText:&*replacement,replacementRange:self.ivars().range.get()];
            text.setSelectedRanges(&selection);
        }}
    }
);

// Cell edits replace only the corresponding Markdown cell, so delimiter rows,
// alignment, adjacent inline source, and native document undo are preserved.
use objc2_foundation::NSObjectProtocol;

struct TableCellState {
    row: usize, column: usize, selected: std::cell::Cell<bool>,
    text: Weak<NSTextView>,
    range: std::cell::Cell<NSRange>,
    fingerprint: std::cell::Cell<u64>,
    original_display: RefCell<String>,
    markdown: RefCell<String>,
    editing_markdown: std::cell::Cell<bool>,
    committing: std::cell::Cell<bool>,
    needs_layout: std::cell::Cell<bool>,
}
unsafe fn table_cells(text: &NSTextView) -> Vec<Retained<TableCell>> {
    let mut cells = Vec::new();
    for table in text.subviews() {
        if !table.isKindOfClass(InlineTable::class()) {
            continue;
        }
        for cell in table.subviews() {
            if cell.isKindOfClass(TableCell::class()) {
                cells.push(Retained::retain(Retained::as_ptr(&cell) as *mut TableCell).unwrap());
            }
        }
    }
    cells
}
/// Restyling must not destroy the native field editor while a cell is composing.
pub unsafe fn table_cell_is_editing(text: &NSTextView) -> bool {
    table_cells(text)
        .iter()
        .any(|cell| cell.currentEditor().is_some() || cell.ivars().committing.get())
}
/// A commit can occur while Tab moves to another cell. Rebuild previews once
/// the last native cell editor releases focus, even if text selection is stable.
pub unsafe fn table_cells_need_layout(text: &NSTextView) -> bool {
    table_cells(text)
        .iter()
        .any(|cell| cell.ivars().needs_layout.get())
}
fn table_cell_has_source_slot(source: &str, range: &Range<usize>) -> bool {
    if !range.is_empty() {
        return true;
    }
    // A short GFM row gets synthetic cells. Only a real pair of pipes can
    // receive an empty-cell insertion without changing the table structure.
    source[..range.start]
        .trim_end_matches([' ', '\t'])
        .ends_with('|')
        && source[range.end..]
            .trim_start_matches([' ', '\t'])
            .starts_with('|')
}
fn escaped_table_cell(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace("\r\n", "\n")
        .replace(['\r', '\n'], "<br>")
}
fn table_cell_replacement(original: &str, value: &str) -> String {
    let escaped = escaped_table_cell(value);
    // Preserve a simple full-cell inline style. More complex inline Markdown
    // remains untouched until that cell is actually changed.
    for mark in ["**", "__", "~~", "*", "_", "`"] {
        if original.len() > mark.len() * 2
            && original.starts_with(mark)
            && original.ends_with(mark)
            && spans(original).iter().any(|span| {
                span.bytes == (0..original.len())
                    && matches!(
                        span.treatment,
                        Treatment::Bold | Treatment::Italic | Treatment::Strike | Treatment::Code
                    )
            })
        {
            return format!("{mark}{escaped}{mark}");
        }
    }
    escaped
}
unsafe fn commit_table_cell(cell: &TableCell) {
    let _keep_alive = Retained::retain(cell as *const TableCell as *mut TableCell).unwrap();
    let Some(text) = cell.ivars().text.load() else {
        return;
    };
    if !text.isEditable() || cell.ivars().committing.get() {
        return;
    }
    if let Some(editor) = cell.currentEditor() {
        let marked: bool = msg_send![&*editor, hasMarkedText];
        if marked {
            return;
        }
    }
    let value = cell.stringValue().to_string();
    let editing = cell.ivars().editing_markdown.get();
    if (editing && value == *cell.ivars().markdown.borrow()) || (!editing && value == *cell.ivars().original_display.borrow()) {
        return;
    }
    let source = text.string().to_string();
    if fingerprint(&source) != cell.ivars().fingerprint.get() {
        return;
    }
    let range = cell.ivars().range.get();
    let begin = byte_for_utf16(&source, range.location);
    let end = byte_for_utf16(&source, range.location + range.length);
    let replacement = if editing { grid_encode_source(&value) } else { table_cell_replacement(&source[begin..end], &value) };
    let length = replacement.encode_utf16().count();
    let delta = length as isize - range.length as isize;
    let siblings = table_cells(&text);
    let selection: NSRange = msg_send![&*text, selectedRange];
    cell.ivars().committing.set(true);
    let _: () =
        msg_send![&*text, insertText:&*NSString::from_str(&replacement), replacementRange:range];
    let updated = text.string().to_string();
    let new_fingerprint = fingerprint(&updated);
    for sibling in siblings {
        let mut r = sibling.ivars().range.get();
        if std::ptr::eq(&*sibling, cell) {
            r.length = length;
        } else if r.location >= range.location + range.length {
            r.location = r.location.saturating_add_signed(delta);
        }
        sibling.ivars().range.set(r);
        sibling.ivars().fingerprint.set(new_fingerprint);
    }
    // Table-margin source navigation also needs current offsets while Tab
    // keeps another cell editor alive and layout is intentionally deferred.
    for view in text.subviews() {
        if !view.isKindOfClass(InlineTable::class()) {
            continue;
        }
        let table = &*(Retained::as_ptr(&view) as *const InlineTable);
        let mut r = table.ivars().range.get();
        if r.location <= range.location && range.location + range.length <= r.location + r.length {
            r.length = r.length.saturating_add_signed(delta);
        } else if r.location >= range.location + range.length {
            r.location = r.location.saturating_add_signed(delta);
        }
        table.ivars().range.set(r);
    }
    // insertText uses the main editor's typing attributes. Keep the backing
    // source concealed until the next layout pass without disturbing its row
    // spacing or the active cell's field editor.
    if let Some(storage) = text.textStorage() {
        let undo: Option<Retained<AnyObject>> = msg_send![&*text, undoManager];
        if let Some(undo) = &undo {
            let _: () = msg_send![undo, disableUndoRegistration];
        }
        conceal(&storage, NSRange::new(range.location, length), false);
        if let Some(undo) = &undo {
            let _: () = msg_send![undo, enableUndoRegistration];
        }
    }
    let remap = |offset: usize| {
        if offset >= range.location + range.length {
            offset.saturating_add_signed(delta)
        } else if offset > range.location {
            range.location + length
        } else {
            offset
        }
    };
    let start = remap(selection.location);
    let end = remap(selection.location + selection.length);
    text.setSelectedRange(NSRange::new(start, end.saturating_sub(start)));
    *cell.ivars().markdown.borrow_mut() = grid_editable_source(&replacement);
    *cell.ivars().original_display.borrow_mut() = value;
    cell.ivars().needs_layout.set(true);
    cell.ivars().committing.set(false);
}
define_class!(
    #[unsafe(super(NSTextField, NSControl, NSView, NSResponder))]
    #[name = "StologyMarkdownTableCell"]
    #[thread_kind = MainThreadOnly]
    #[ivars = TableCellState]
    struct TableCell;
    unsafe impl NSObjectProtocol for TableCell {}
    unsafe impl NSControlTextEditingDelegate for TableCell {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn command(&self, _: &NSControl, editor: &NSTextView, command: objc2::runtime::Sel) -> bool {
            let marked:bool = msg_send![editor,hasMarkedText];
            if !marked && command == sel!(deleteBackward:) && inline_backspace(editor,grid_inline_syntax) {
                true
            } else if !marked && (command == sel!(moveLeft:) || command == sel!(moveRight:)) {
                let left = command == sel!(moveLeft:);
                if left { editor.moveLeft(None); } else { editor.moveRight(None); }
                skip_inline_marker(editor,left,grid_inline_syntax);
                true
            } else if command == sel!(insertNewline:) && !marked {
                editor.insertNewlineIgnoringFieldEditor(None);
                true
            } else { false }
        }
        #[unsafe(method(controlTextDidChange:))]
        fn grid_text_changed(&self, _: &objc2_foundation::NSNotification) {unsafe {
            if let Some(editor) = self.currentEditor().and_then(|e|e.downcast::<NSTextView>().ok()) {
                restyle_grid_editor(self,&editor);
            }
        }}
        #[unsafe(method(controlTextDidEndEditing:))]
        fn did_end_editing(&self, _notification: &objc2_foundation::NSNotification) {
            unsafe {
                commit_table_cell(self);
                if self.ivars().editing_markdown.replace(false) {
                    if let (Some(view),Some(text)) = (self.grid(),self.ivars().text.load()) {
                        let source = text.string().to_string();
                        if let Some(table) = table_previews(&source).into_iter().find(|t| utf16_range(&source,t.bytes.clone()) == view.ivars().range.get()) {
                            render_grid_cell(self,&table);
                        }
                    }
                }
            }
        }
    }
    unsafe impl NSTextFieldDelegate for TableCell {}
    impl TableCell {
        #[unsafe(method(drawRect:))]
        fn draw_cell(&self, dirty:NSRect) {unsafe {
            if let Some(background) = self.backgroundColor() {
                background.setFill();
                NSBezierPath::bezierPathWithRect(self.bounds()).fill();
            }
            let _:() = msg_send![super(self),drawRect:dirty];
        }}
        #[unsafe(method(mouseDown:))]
        fn grid_down(&self, event: &NSEvent) { unsafe {
            if event.clickCount() >= 2 && self.isEditable() {
                self.selectText(None);
            } else if self.ivars().text.load().is_some_and(|t| t.isEditable()) {
                track_grid_selection(self);
            } else { let _: () = msg_send![super(self), mouseDown:event]; }
        }}
        #[unsafe(method(rightMouseDown:))]
        fn context(&self, event: &NSEvent) { unsafe { grid_menu(self,event); }}
        #[unsafe(method(gridAlign:))]
        fn align_cells(&self,item:&NSMenuItem) {unsafe { apply_grid_alignment(self,item.tag()); }}
        #[unsafe(method(gridPlace:))]
        fn place_table(&self,item:&NSMenuItem) {unsafe { apply_grid_placement(self,item.tag()); }}
        #[unsafe(method(commitCell:))]
        fn commit_cell(&self, _sender: Option<&AnyObject>) {
            unsafe { commit_table_cell(self); }
        }
    }
);

include!("table_controls.rs");
include!("table_grid.rs");
include!("block_caret.rs");
include!("block_edit.rs");
include!("block_handles.rs");

const TABLE_COLUMN_WIDTH: f64 = 140.0;

struct InlineTableState {
    text: Weak<NSTextView>,
    range: std::cell::Cell<NSRange>,
    base_directory: Option<PathBuf>,
    /// What this grid was built from; see `InlineTableSpec::built_from`.
    built_from: u64,
}
define_class!(
    #[unsafe(super = NSView)]
    #[name = "StologyInlineMarkdownTable"]
    #[thread_kind = MainThreadOnly]
    #[ivars = InlineTableState]
    struct InlineTable;
    impl InlineTable {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts(&self) -> bool { true }
        #[unsafe(method(keyDown:))]
        fn grid_key(&self,event:&NSEvent) { unsafe {
            if !move_grid_selection(self,event.keyCode()) { let _: () = msg_send![super(self),keyDown:event]; }
        }}
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self,_event:Option<&NSEvent>) {unsafe {
            // Restyling removes this preview from its superview. Keep the
            // receiver and its ivars alive until the callback has returned.
            let _keep_alive=Retained::retain(self as *const Self as *mut Self).expect("live preview receiver");
            enter_source(&BlockPreviewState {
                text: self.ivars().text.clone(),
                range: self.ivars().range.get(),
                base_directory: self.ivars().base_directory.clone(),
            });
        }}
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool { true }
    }
);
#[derive(Debug)]
struct InlineTableSpec {
    styles: std::collections::HashMap<(usize,usize),Vec<Span>>,
    layout: serde_json::Value, marker: Option<Range<usize>>,
    alignments: Vec<pulldown_cmark::Alignment>,
    bytes: Range<usize>,
    rows: Vec<Vec<String>>,
    cell_ranges: Vec<Vec<Range<usize>>>,
    /// Left edge within the note body and the width its column allows.
    x: f64,
    limit: f64,
    /// Room below the anchor line.
    spacing: f64,
}
impl InlineTableSpec {
    fn widths(&self, handle_room: f64) -> Vec<f64> {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let available = (self.limit - handle_room).max(40.0);
        let cell_width = TABLE_COLUMN_WIDTH.min(available / columns as f64);
        let mut widths: Vec<_> = (0..columns).map(|c| self.column_width(c, cell_width)).collect();
        let scale = (available / widths.iter().sum::<f64>()).min(1.);
        for width in &mut widths { *width *= scale; }
        widths
    }

    fn fit(&mut self, x: f64, limit: f64, handle_room: f64) -> f64 {
        self.x = x;
        self.limit = limit;
        self.widths(handle_room).iter().sum::<f64>() + handle_room
    }

    fn placement(&self) -> Placement {
        if self.layout["wrap"].as_bool() != Some(true) {
            Placement::Full
        } else if self.layout["side"].as_str() == Some("right") {
            Placement::Right
        } else {
            Placement::Left
        }
    }

    /// Everything a grid's views depend on apart from where the grid sits.
    /// A grid built from the same values is the same grid.
    fn built_from(&self, source: &str, handle_room: f64, editable: bool) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut state = std::collections::hash_map::DefaultHasher::new();
        source[self.bytes.clone()].hash(&mut state);
        (self.limit.to_bits(), handle_room.to_bits(), editable).hash(&mut state);
        state.finish()
    }

    /// The space a table and its controls take below the anchor line.
    fn reserve(&self, handle_room: f64) -> f64 {
        self.height() + TABLE_TOP_PADDING + handle_room + if handle_room > 0.0 { 2.0 } else { 6.0 }
    }

    fn height(&self) -> f64 {
        (0..self.rows.len()).map(|r| self.row_height(r)).sum()
    }
    fn anchor(&self, source: &str) -> NSRange {
        let content = source[self.bytes.clone()].trim_end_matches(['\r', '\n']);
        let start = self.bytes.start + content.rfind('\n').map(|n| n + 1).unwrap_or(0);
        utf16_range(source, start..self.bytes.start + content.len())
    }

    fn separator_line(&self, source: &str) -> Option<Range<usize>> {
        table_separator(source, self.bytes.end)
    }
}

fn table_separator(source: &str, start: usize) -> Option<Range<usize>> {
    let line = source[start..].split_inclusive('\n').next()?;
    (source[..start].ends_with('\n') && line.ends_with('\n') && line.trim_matches([' ', '\t', '\r', '\n']).is_empty())
        .then_some(start..start + line.len())
}
const TABLE_TOP_PADDING: f64 = 6.0;
const TABLE_HANDLE_SIZE: f64 = 22.0;

const IMAGE_PADDING: f64 = 6.0;
const COLUMN_GAP: f64 = 24.0;
const MIN_COLUMN_WIDTH: f64 = 160.0;

/// An attachment hangs below a collapsed anchor line that reserves its room.
unsafe fn style_anchor(storage: &objc2_foundation::NSMutableAttributedString, range: NSRange, spacing: f64) {
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setMinimumLineHeight(0.01);
    paragraph.setMaximumLineHeight(0.01);
    paragraph.setParagraphSpacing(spacing.max(0.0));
    storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
}

fn table_previews(source: &str) -> Vec<InlineTableSpec> {
    let mut tables = Vec::new();
    let mut table: Option<InlineTableSpec> = None;
    let mut row = Vec::new();
    let mut row_ranges = Vec::new();
    let mut cell_range = 0..0;
    let mut cell = String::new();
    let mut open_styles = Vec::new();
    let mut runs = Vec::new();
    for (event, bytes) in Parser::new_ext(source, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).into_offset_iter() {
        match event {
            Event::Start(Tag::Table(alignments)) => {
                table = Some(InlineTableSpec {
                    styles:Default::default(), alignments, layout: serde_json::json!({}), marker: None,
                    bytes,
                    rows: Vec::new(),
                    cell_ranges: Vec::new(),
                    x: 0.0,
                    limit: f64::MAX,
                    spacing: 0.0,
                })
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                row.clear();
                row_ranges.clear();
            }
            Event::Start(Tag::TableCell) => {
                cell.clear(); open_styles.clear(); runs.clear();
                let raw = &source[bytes.clone()];
                let leading = raw.len() - raw.trim_start_matches([' ', '\t']).len();
                let trimmed = raw.trim_matches([' ', '\t']);
                cell_range = bytes.start + leading..bytes.start + leading + trimmed.len();
            }
            Event::Start(tag @ (Tag::Strong | Tag::Emphasis | Tag::Strikethrough)) if table.is_some() => {
                let kind = match tag { Tag::Strong => Treatment::Bold, Tag::Emphasis => Treatment::Italic, _ => Treatment::Strike };
                open_styles.push((kind,cell.len()));
            }
            Event::End(tag @ (TagEnd::Strong | TagEnd::Emphasis | TagEnd::Strikethrough)) if table.is_some() => {
                let kind = match tag { TagEnd::Strong => Treatment::Bold, TagEnd::Emphasis => Treatment::Italic, _ => Treatment::Strike };
                finish_grid_run(&mut open_styles,&mut runs,kind,cell.len());
            }
            Event::InlineHtml(value) if table.is_some() => {
                if let Some((kind,closing)) = grid_html_style(&value) {
                    if closing { finish_grid_run(&mut open_styles,&mut runs,kind,cell.len()); }
                    else { open_styles.push((kind,cell.len())); }
                    continue;
                }
                if matches!(value.to_ascii_lowercase().as_str(), "<br>"|"<br/>"|"<br />") { cell.push('\n'); }
                else if let Some(t) = table.as_mut() {
                    if t.rows.is_empty() && row.is_empty() && bytes.start == cell_range.start {
                        if let Some(json) = value.strip_prefix("<!--stology-grid ").and_then(|s| s.strip_suffix("-->")) {
                            if let Ok(v @ serde_json::Value::Object(_)) = serde_json::from_str(json) {
                                t.layout = v; cell_range.start = bytes.end; t.marker = Some(bytes);
                            }
                        }
                    }
                }
            }
            Event::Text(value) if table.is_some() => cell.push_str(&value),
            Event::Code(value) if table.is_some() => {
                let start = cell.len(); cell.push_str(&value);
                runs.push(Span{bytes:start..cell.len(), treatment:Treatment::Code});
            }
            Event::SoftBreak | Event::HardBreak if table.is_some() => cell.push(' '),
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = &mut table {
                    if !runs.is_empty() { t.styles.insert((t.rows.len(),row.len()), std::mem::take(&mut runs)); }
                }
                row.push(cell.clone());
                row_ranges.push(cell_range.clone());
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                if let Some(t) = &mut table {
                    t.rows.push(row.clone());
                    t.cell_ranges.push(row_ranges.clone());
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    if t.rows.len() <= 100 && t.rows.iter().all(|r| r.len() <= 16) {
                        tables.push(t);
                    }
                }
            }
            _ => {}
        }
    }
    tables
}

struct InlineImageSpec {
    link: image_edit::markdown::ImageLink,
    range: NSRange,
    image: Retained<NSImage>,
    size: NSSize,
    /// Left edge within the note body, the widest its column allows, and the
    /// room below the anchor line.
    x: f64,
    limit: f64,
    spacing: f64,
    /// The blank line after a standalone image, hidden like a table's.
    separator: Option<Range<usize>>,
}
impl InlineImageSpec {
    fn fit(&mut self, x: f64, limit: f64) -> f64 {
        let natural = self.image.size();
        let width = self.link.width.unwrap_or(natural.width.min(320.0)).min(limit.max(32.0));
        self.size = NSSize::new(width, natural.height * width / natural.width);
        self.x = x;
        self.limit = limit;
        width
    }
}
unsafe fn inline_images(source: &str, base: Option<&Path>, max_width: f64) -> Vec<InlineImageSpec> {
    let mut images = Vec::new();
    for link in image_edit::markdown::images(source) {
        let dest_url = &link.destination;
        let bytes = link.bytes.clone();
        if dest_url.contains("://") || dest_url.starts_with("data:") {
            continue;
        }
        let target = Path::new(dest_url);
        let path = if target.is_absolute() {
            target.to_path_buf()
        } else if let Some(base) = base {
            base.join(target)
        } else {
            continue;
        };
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if !metadata.is_file() || metadata.len() > 20 * 1024 * 1024 {
            continue;
        }
        let Some(image) = image_edit::load_image(&path, &metadata) else {
            continue;
        };
        let natural = image.size();
        if natural.width <= 0.0 || natural.height <= 0.0 {
            continue;
        }
        let mut spec = InlineImageSpec {
            link,
            range: utf16_range(source, bytes),
            image,
            size: natural,
            x: 0.0,
            limit: max_width,
            spacing: 0.0,
            separator: None,
        };
        spec.fit(0.0, max_width);
        spec.spacing = spec.size.height + IMAGE_PADDING * 2.0;
        images.push(spec);
    }
    images
}

/// Jump to a UTF-8 source quote without splitting surrogate pairs.
pub unsafe fn locate(text: &NSTextView, quote: &str) {
    if quote.is_empty() {
        return;
    }
    let source = text.string().to_string();
    if let Some(start) = source.find(quote) {
        let range = utf16_range(&source, start..start + quote.len());
        text.setSelectedRange(range);
        text.scrollRangeToVisible(range);
    }
}

/// Navigate only within the recorded paragraph, so repeated quotes do not
/// accidentally select an unrelated occurrence. Uses extraction's CRLF and
/// blank-paragraph numbering, including gaps for empty paragraphs.
pub unsafe fn locate_paragraph(text: &NSTextView, quote: &str, index: usize) -> bool {
    if quote.is_empty() || index == 0 {
        return false;
    }
    let source = text.string().to_string();
    let Some(bytes) = paragraph_quote_range(&source, quote, index) else {
        return false;
    };
    let range = utf16_range(&source, bytes);
    text.setSelectedRange(range);
    text.scrollRangeToVisible(range);
    true
}
fn paragraph_quote_range(source: &str, quote: &str, index: usize) -> Option<Range<usize>> {
    if index == 0 || quote.is_empty() {
        return None;
    }
    let mut normalized = String::new();
    let mut offsets = Vec::with_capacity(source.len() + 1);
    for (offset, c) in source.char_indices() {
        if c == '\r' && source.as_bytes().get(offset + 1) == Some(&b'\n') {
            continue;
        }
        normalized.push(c);
        for byte in 0..c.len_utf8() {
            offsets.push(offset + byte);
        }
    }
    offsets.push(source.len());
    let mut begin = 0;
    for (i, paragraph) in normalized.split("\n\n").enumerate() {
        if i + 1 == index {
            let start = begin + paragraph.find(quote)?;
            return Some(offsets[start]..offsets[start + quote.len() - 1] + 1);
        }
        begin += paragraph.len() + 2;
    }
    None
}

/// In-window original viewer. PDF page numbers are one-based. Quick Look's
/// public view API does not provide PPTX slide navigation; callers show the
/// evidence slide number as a label instead of claiming a direct jump.
pub unsafe fn preview(
    path: &str,
    frame: NSRect,
    page: Option<usize>,
) -> Result<Retained<NSView>, String> {
    let mtm = MainThreadMarker::new().ok_or("미리보기는 메인 스레드에서 열어야 합니다.")?;
    let file = Path::new(path);
    if !file.is_file() {
        return Err("원본 파일을 찾을 수 없습니다.".into());
    }
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let url = NSURL::fileURLWithPath(&NSString::from_str(path));
    let view: Retained<NSView> = match ext.as_str() {
        "pdf" => {
            let allocated: Allocated<AnyObject> = msg_send![class!(PDFDocument), alloc];
            let document: Option<Retained<AnyObject>> = msg_send![allocated, initWithURL: &*url];
            let document = document.ok_or("PDF가 손상되었거나 읽을 수 없습니다.")?;
            let locked: bool = msg_send![&*document, isLocked];
            if locked {
                return Err("암호로 잠긴 PDF입니다. 암호를 해제한 파일을 가져와 주세요.".into());
            }
            let count: usize = msg_send![&*document, pageCount];
            if count == 0 {
                return Err("표시할 페이지가 없는 PDF입니다.".into());
            }
            let allocated: Allocated<NSView> = msg_send![class!(PDFView), alloc];
            let view: Retained<NSView> = msg_send![allocated, initWithFrame: frame];
            let _: () = msg_send![&*view, setDocument: &*document];
            let _: () = msg_send![&*view, setAutoScales: true];
            crate::scrollbar::style_descendants(&view);
            if let Some(number) = page.filter(|n| *n > 0 && *n <= count) {
                let target: Option<Retained<AnyObject>> =
                    msg_send![&*document, pageAtIndex: number - 1];
                if let Some(target) = target {
                    let _: () = msg_send![&*view, goToPage: &*target];
                }
            }
            view
        }
        "png" | "jpg" | "jpeg" | "heic" | "heif" => {
            let image =
                NSImage::initWithContentsOfFile(NSImage::alloc(), &NSString::from_str(path))
                    .ok_or("이미지가 손상되었거나 macOS가 지원하지 않는 형식입니다.")?;
            let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), frame);
            view.setImage(Some(&image));
            view.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
            Retained::into_super(Retained::into_super(view))
        }
        "pptx" => {
            let allocated: Allocated<NSView> = msg_send![class!(QLPreviewView), alloc];
            let view: Retained<NSView> = msg_send![allocated, initWithFrame: frame, style: 0isize];
            let _: () = msg_send![&*view, setAutostarts: true];
            let _: () = msg_send![&*view, setPreviewItem: &*url];
            view
        }
        _ => {
            return Err(format!(
                "이 파일 형식의 원본 미리보기는 지원하지 않습니다: {ext}"
            ))
        }
    };
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    Ok(view)
}

/// Open the managed working PDF with editable standard annotations. Evidence
/// callers continue using `preview`, which never installs annotation controls.
pub unsafe fn editable_pdf(
    path: &str,
    frame: NSRect,
    document_id: &str,
) -> Result<Retained<NSView>, String> {
    let pdf = preview(path, NSRect::new(NSPoint::new(0.0, 0.0), frame.size), None)?;
    let document: Option<Retained<AnyObject>> = msg_send![&*pdf, document];
    let document = document.ok_or("PDF 문서를 열 수 없습니다")?;
    let allows: bool = msg_send![&*document, allowsCommenting];
    if !allows {
        return Err(
            "이 PDF는 주석 작성을 허용하지 않습니다. 권한이 있는 PDF를 사용해 주세요.".into(),
        );
    }
    // PDF signatures become invalid after any rewrite, and interactive forms
    // may carry calculation/signing behavior. Warn before first editing entry.
    let count: usize = msg_send![&*document, pageCount];
    let mut has_form = false;
    for index in 0..count {
        let page: Option<Retained<AnyObject>> = msg_send![&*document, pageAtIndex:index];
        if let Some(page) = page {
            let annotations: Retained<objc2_foundation::NSArray<AnyObject>> =
                msg_send![&*page, annotations];
            for annotation in annotations.iter() {
                let kind: Option<Retained<NSString>> = msg_send![&*annotation, type];
                if kind.is_some_and(|k| k.to_string() == "Widget") {
                    has_form = true;
                    break;
                }
            }
        }
        if has_form {
            break;
        }
    }
    if has_form {
        let alert = NSAlert::new(MainThreadMarker::new().unwrap());
        alert.setMessageText(&NSString::from_str("서명·양식이 포함된 PDF입니다"));
        alert.setInformativeText(&NSString::from_str("필기를 저장하면 디지털 서명이 무효화되거나 양식 동작이 달라질 수 있습니다. 같은 PDF를 갱신하며 깨끗한 원본 사본은 자동 보관하지 않습니다."));
        alert.addButtonWithTitle(&NSString::from_str("읽기만 하기"));
        alert.addButtonWithTitle(&NSString::from_str("필기 시작"));
        if alert.runModal() != 1001 {
            return Err("서명·양식 PDF를 읽기 전용으로 열었습니다.".into());
        }
    }
    let callback = crate::pdf_session::connect(document_id, document)?;
    let view = crate::pdf_annotations::create(&pdf, frame, callback);
    crate::pdf_session::attach(document_id, &view);
    Ok(view)
}

/// Release Quick Look renderer resources before removing a source preview.
pub unsafe fn close_preview(view: &NSView) {
    crate::pdf_session::detach(view);
    let is_quicklook: bool = msg_send![view, isKindOfClass: class!(QLPreviewView)];
    if is_quicklook {
        let _: () = msg_send![view, close];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table_cell_ranges_preserve_unedited_markdown() {
        let source = "| **열** | 값 |\r\n|:---|---:|\r\n| 한글 \\| 값 | 😀 |\r\n";
        let tables = table_previews(source);
        let table = &tables[0];
        assert_eq!(table.rows, vec![vec!["열", "값"], vec!["한글 | 값", "😀"]]);
        assert_eq!(&source[table.cell_ranges[0][0].clone()], "**열**");
        assert_eq!(&source[table.cell_ranges[1][0].clone()], "한글 \\| 값");
        let mut edited = source.to_string();
        edited.replace_range(
            table.cell_ranges[1][1].clone(),
            &escaped_table_cell("새 값 | 2"),
        );
        assert_eq!(
            edited,
            "| **열** | 값 |\r\n|:---|---:|\r\n| 한글 \\| 값 | 새 값 \\| 2 |\r\n"
        );
        assert_eq!(table_cell_replacement("**열**", "제목"), "**제목**");
        assert_eq!(table_cell_replacement("값", "a\nb"), "a<br>b");
    }
    #[test]
    fn table_empty_cells_have_source_slots() {
        let source = "| A | B |\n|---|---|\n|   | 값 |\n";
        let table = table_previews(source).remove(0);
        let range = table.cell_ranges[1][0].clone();
        assert_eq!(&source[range.clone()], "");
        assert!(range.start >= source.find("|   |").unwrap());
        let mut edited = source.to_string();
        edited.replace_range(range, "새 값");
        assert_eq!(table_previews(&edited)[0].rows[1][0], "새 값");
    }
    #[test]
    fn active_table_is_a_whole_editing_block() {
        let source = "# 제목\n\n| A | B |\n|---|---|\n| 한글 | 😀 |\n\n마지막";
        let byte = source.find("한글").unwrap();
        let active = active_block(
            source,
            NSRange::new(source[..byte].encode_utf16().count(), 0),
        );
        assert!(source[active.clone()].starts_with("| A"));
        assert!(source[active].contains("😀"));
    }
    #[test]
    fn active_blocks_preserve_crlf_and_whitespace_blank_lines() {
        for blank in ["\r\n", "  \r\n", "\t \r\n", " \n"] {
            let source = format!("# 앞 제목\r\n{blank}| 열 | 값 |\r\n|---|---|\r\n| 한글 | 😀 |\r\n{blank}나중 **문단**\r\n{blank}마지막");
            let byte = source.find("문단").unwrap();
            let active = active_block(
                &source,
                NSRange::new(source[..byte].encode_utf16().count(), 0),
            );
            assert!(source[active.clone()].starts_with("나중"));
            assert!(!source[active.clone()].contains("#"));
            assert!(!source[active.clone()].contains('|'));
            assert!(!source[active.clone()].contains("마지막"));
            assert!(source.is_char_boundary(active.start) && source.is_char_boundary(active.end));
            let table = table_previews(&source).remove(0);
            assert!(!intersects(&table.bytes, Some(&active)));
            assert!(hidden_syntax(&source, Some(&active))
                .iter()
                .any(|r| &source[r.clone()] == "# "));
        }
    }
    #[test]
    fn whitespace_line_caret_does_not_activate_adjacent_blocks() {
        let source = "# 제목\r\n  \r\n**다음**";
        let byte = source.find("  ").unwrap() + 1;
        let active = active_block(
            source,
            NSRange::new(source[..byte].encode_utf16().count(), 0),
        );
        assert!(active.is_empty());
    }
    #[test]
    fn trailing_blank_lines_start_after_last_content_line() {
        assert_eq!(trailing_blank_start("본문\n\n---\n\n\n"), "본문\n\n---\n".len());
        assert_eq!(trailing_blank_start("본문\r\n  \r\n"), "본문\r\n".len());
        assert_eq!(trailing_blank_start("본문"), "본문".len());
        assert_eq!(trailing_blank_start("\n\n"), 0);
    }
    #[test]
    fn line_tails_cover_last_character_and_line_ending() {
        let source = "가나\r\n\nROE 😀\n끝";
        let tails: Vec<&str> = line_tail_ranges(source).into_iter().map(|r| &source[r]).collect();
        assert_eq!(tails, ["나\r\n", "\n", "😀\n", "끝"]);
    }
    #[test]
    fn multi_block_or_whole_selection_keeps_formatting() {
        let source = "# 제목\n\n**굵게** 문단\n\n- 항목";
        let all = NSRange::new(0, source.encode_utf16().count());
        let active = active_block(source, all);
        assert!(active.is_empty());
        let hidden = hidden_syntax(source, Some(&active));
        assert!(hidden.iter().any(|r| &source[r.clone()] == "# "));
        assert!(hidden.iter().any(|r| &source[r.clone()] == "**"));
        let single = "**굵게** 문단";
        assert!(active_block(single, NSRange::new(0, single.encode_utf16().count())).is_empty());
        let start = source[..source.find("굵게").unwrap()].encode_utf16().count();
        let inside = active_block(source, NSRange::new(start, 2));
        assert!(source[inside].starts_with("**굵게**"));
    }
    #[test]
    fn syntax_stays_concealed_while_heading_is_edited() {
        let source = "# 제목\n\n**굵게**\n\n- [x] 완료";
        let active = active_block(source, NSRange::new(0, 0));
        let hidden = hidden_syntax(source, Some(&active));
        assert!(hidden.iter().any(|r| &source[r.clone()] == "# "));
        assert!(hidden.iter().any(|r| source[r.clone()].contains("[x]")));
        for range in hidden {
            assert!(source.is_char_boundary(range.start) && source.is_char_boundary(range.end));
        }
    }
    #[test]
    fn heading_requires_space_and_never_restyles_code() {
        for raw in ["#", "##", "##제목", "####### 제목"] {
            assert!(!spans(raw)
                .iter()
                .any(|s| matches!(s.treatment, Treatment::Heading(_))));
        }
        for level in 1..=6 {
            let raw = format!("{} ", "#".repeat(level));
            assert!(spans(&raw)
                .iter()
                .any(|s| s.treatment == Treatment::Heading(level as u8)));
            assert_eq!(
                hidden_syntax(&raw, Some(&(0..raw.len()))),
                vec![0..raw.len()]
            );
        }
        assert!(!spans("```md\n# literal\n```")
            .iter()
            .any(|s| matches!(s.treatment, Treatment::Heading(_))));
    }
    #[test]
    fn underline_markers_never_restyle_the_previous_paragraph() {
        for ending in ["\n", "\r\n"] {
            for marker in ["-", "--", "---", "=", "==="] {
                let source = format!("한글 😀 본문{ending}{marker}");
                assert!(!spans(&source).iter().any(|span| matches!(span.treatment, Treatment::Heading(_))));
                assert!(hidden_syntax(&source, None).is_empty());
            }
        }
        assert!(spans("## 명시적 제목").iter().any(|span| span.treatment == Treatment::Heading(2)));
        assert!(spans("본문\n\n---").iter().any(|span| span.treatment == Treatment::Rule));
        // A rule does not depend on the block above it.
        for (source, rule) in [("본문\n---\n뒤", "---\n"), ("본문\r\n---", "---"), ("![a](a.png)\n---\n", "---\n"), ("- 항목\n  ---\n", "  ---\n")] {
            let found: Vec<_> = spans(source).into_iter().filter(|span| span.treatment == Treatment::Rule).collect();
            assert_eq!(found.len(), 1, "{source:?}");
            assert_eq!(&source[found[0].bytes.clone()], rule);
        }
        assert!(note_structure("본문\n---\n뒤").iter().map(|b| b.kind)
            .eq([note_blocks::Kind::Text, note_blocks::Kind::Rule, note_blocks::Kind::Text]));
    }

    #[test]
    fn code_panel_respects_indentation_and_real_closing_fence() {
        let source = "  ```java\n  int value = 1;\n  ```\n";
        let block = code_blocks(source).remove(0);
        assert_eq!(block.language, "java");
        assert_eq!(source[block.close.clone()].trim(), "```");
        for tail in ["~", "~~", "~~~", "``"] {
            let source = format!("```rust\nlet x = 1;\n{tail}");
            let block = code_blocks(&source).remove(0);
            assert!(block.close.is_empty());
            assert!(hidden_syntax(&source, None).is_empty());
        }
    }
    #[test]
    fn quote_prefix_visibility_matches_visual_block_commit() {
        for source in [">", ">>", ">text", "a >", "\\>", "> >", "> >>", "한글 😀\n>"] {
            assert!(hidden_syntax(source, None).is_empty(), "{source}");
            for block in markdown_shortcuts::blocks(source) {
                let prefix = &source[block.prefix];
                assert!(prefix.ends_with(' '), "uncommitted prefix: {source}");
            }
        }
        for source in ["> ", "> 한글 😀", "> > nested", "  > quote", "> quote\r\n> next"] {
            assert!(hidden_syntax(source, None).is_empty(), "duplicate concealment: {source}");
            let blocks = markdown_shortcuts::blocks(source);
            assert!(!blocks.is_empty(), "missing quote: {source}");
            assert!(blocks.iter().all(|block| block.quote_depth > 0));
        }
        assert_eq!(markdown_shortcuts::blocks("> >")[0].prefix, 0..2);
        assert_eq!(markdown_shortcuts::blocks("> > nested")[0].prefix, 0..4);
        assert!(markdown_shortcuts::blocks("```\n> code\n```").is_empty());
    }
    #[test]
    fn korean_surrogates_use_utf16_native_ranges() {
        let source = "앞 😀 한글 받침 뒤";
        let start = source.find("한글 받침").unwrap();
        let r = utf16_range(source, start..start + "한글 받침".len());
        assert_eq!((r.location, r.length), (5, 5));
    }
    #[test]
    fn source_navigation_uses_recorded_paragraph() {
        let source = "같은 구절\r\n\r\n둘째 같은 구절\r\n\r\n\r\n\r\n마지막";
        let found = paragraph_quote_range(source, "같은 구절", 2).unwrap();
        assert_eq!(&source[found.clone()], "같은 구절");
        assert_eq!(found.start, source.find("둘째").unwrap() + "둘째 ".len());
        assert!(paragraph_quote_range(source, "같은 구절", 3).is_none());
        assert!(paragraph_quote_range(source, "마지막", 4).is_some());
    }
    #[test]
    fn table_preview_has_cells_and_last_row_anchor() {
        let source = "| 열 | 값 |\n|---|---|\n| 한글 | **둘** |\n";
        let tables = table_previews(source);
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].rows, vec![vec!["열", "값"], vec!["한글", "둘"]]);
        let r = tables[0].anchor(source);
        assert_eq!(
            r.location,
            source[..source.find("| 한글").unwrap()]
                .encode_utf16()
                .count()
        );
    }
    #[test]
    fn table_preview_collapses_only_the_first_blank_line_after_the_table() {
        for separator in ["\n", "  \r\n", "\t\n"] {
            let source = format!("| 열 | 값 |\n|---|---|\n| 한글 | 😀 |\n{separator}\n본문");
            let table = table_previews(&source).remove(0);
            assert_eq!(&source[table.separator_line(&source).unwrap()], separator);
        }
        let source = "| 열 | 값 |\n|---|---|\n| 한글 | 😀 |\n";
        assert_eq!(table_previews(source)[0].separator_line(source), None);
    }
    #[test]
    fn markdown_coverage_and_byte_boundaries() {
        let source = "# 제목\n\n**굵게** *기울임* [링크](https://example.com) ![그림](image.png)\n\n- [ ] 체크\n\n> 인용\n\n```rust\nfn main() {}\n```\n\n| 열 | 값 |\n|---|---|\n| 한글 | 1 |\n";
        let s = spans(source);
        for needle in [
            Treatment::Heading(1),
            Treatment::Bold,
            Treatment::Italic,
            Treatment::Image,
            Treatment::List,
            Treatment::Quote,
            Treatment::CodeBlock,
            Treatment::Table,
        ] {
            assert!(
                s.iter().any(|s| s.treatment == needle),
                "missing {needle:?}"
            );
        }
        for span in s {
            let r = utf16_range(source, span.bytes);
            assert!(r.length > 0);
        }
    }
}

pub unsafe fn insert_diagram(
    text: &NSTextView,
    selection: NSRange,
    on_caret_line: bool,
    destination: &str,
    base: &Path,
    width: f64,
) {
    if on_caret_line {
        if let Some(editor) = text.downcast_ref::<MarkdownEditor>() {
            *editor.ivars().base_directory.borrow_mut() = Some(base.to_owned());
        }
        let markup = image_edit::attachment_markup("다이어그램", destination, width);
        insert_note_block(text, selection, &markup, markup.len());
        return;
    }
    image_edit::insert_attachment(text,NSRange::new(selection.location,0),"다이어그램",destination,base,width);
}
