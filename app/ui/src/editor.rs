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
#[path = "smart_edit.rs"]
mod smart_edit;
#[path = "diagram_paint.rs"]
pub mod diagram_paint;
pub use image_edit::{set_image_importer, set_diagram_opener};
use image_edit::{ImageState, InlineImage};

unsafe fn input_command(text: &NSTextView, command: markdown_input::Command) -> bool {
    let marked: bool = msg_send![text, hasMarkedText];
    if marked || !text.isEditable() {
        return false;
    }
    let selection: NSRange = msg_send![text, selectedRange];
    if selection.length != 0 {
        return false;
    }
    let source = text.string().to_string();
    let at = byte_for_utf16(&source, selection.location);
    let Some(edit) = markdown_input::command(&source, at, command).or_else(|| {
        if command == markdown_input::Command::Backspace {
            backspace_below_float(text, &source, at, selection)
        } else {
            None
        }
    }) else {
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
    range: NSRange,
    fingerprint: u64,
    fence: String,
    language: String,
}
struct CodeCopyState {
    code: String,
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
            pasteboard.setString_forType(&NSString::from_str(&self.ivars().code), NSPasteboardTypeString);
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
            if fingerprint(&source) != state.fingerprint { return; }
            let index = self.indexOfSelectedItem() as usize;
            let language = CODE_LANGUAGES.get(index).map(|(id, _)| *id).unwrap_or(&state.language);
            let replacement = format!("{}{}\n", state.fence, language);
            let selection: NSRange = msg_send![&*text, selectedRange];
            let delta = replacement.encode_utf16().count() as isize - state.range.length as isize;
            let _: () = msg_send![&*text, insertText:&*NSString::from_str(&replacement), replacementRange:state.range];
            if selection.location >= state.range.location + state.range.length {
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
            let _: () = msg_send![super(self), moveLeft:sender];
            skip_heading_marker(self, true);
            skip_block_marker(self, true);
            skip_inline_marker(self, true,inline_syntax);
            skip_code_fence(self, true);
        }}
        #[unsafe(method(moveRight:))]
        fn move_right(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveRight:sender];
            skip_heading_marker(self, false);
            skip_block_marker(self, false);
            skip_inline_marker(self, false,inline_syntax);
            skip_code_fence(self, false);
        }}
        #[unsafe(method(moveUp:))]
        fn move_up(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveUp:sender];
            visible_line_start(self); skip_code_fence(self, true);
        }}
        #[unsafe(method(moveDown:))]
        fn move_down(&self, sender: Option<&AnyObject>) { unsafe {
            let _: () = msg_send![super(self), moveDown:sender];
            visible_line_start(self); skip_code_fence(self, false);
        }}
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            image_edit::select_image(self, None);
            self.ivars().empty_text_selection.set(None);
            if self.place_block_caret(event) { return; }
            let _: () = msg_send![super(self), mouseDown:event];
            visible_line_start(self); skip_code_fence(self, false);
        }}
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
            NSColor::colorWithSRGBRed_green_blue_alpha(0.957, 0.961, 0.980, 1.0).setFill();
            for frame in self.ivars().code_panels.borrow().iter() {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(*frame, 10.0, 10.0).fill();
            }
            if let Some(y) = self.ivars().image_drop_line.get() {
                NSColor::controlAccentColor().setFill();
                NSBezierPath::bezierPathWithRect(NSRect::new(NSPoint::new(EDITOR_INSET, y), NSSize::new(self.bounds().size.width - 2. * EDITOR_INSET, 2.))).fill();
            }
            NSColor::separatorColor().setFill();
            for frame in self.ivars().rule_bars.borrow().iter() {
                NSBezierPath::bezierPathWithRect(*frame).fill();
            }
            let font = NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0);
            let color = NSColor::secondaryLabelColor();
            let attrs = NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font as &AnyObject, &*color as &AnyObject]);
            for (number, point) in self.ivars().line_numbers.borrow().iter() {
                NSString::from_str(number).drawAtPoint_withAttributes(*point, Some(&attrs));
            }
            for (label, point, size) in self.ivars().list_labels.borrow().iter() {
                let font = crate::font::regular(*size);
                let attrs = NSDictionary::from_slices(&[NSFontAttributeName, NSForegroundColorAttributeName], &[&*font as &AnyObject, &*color as &AnyObject]);
                NSString::from_str(label).drawAtPoint_withAttributes(*point, Some(&attrs));
            }
            NSColor::colorWithSRGBRed_green_blue_alpha(0.48, 0.49, 0.54, 1.0).setFill();
            for bar in self.ivars().quote_bars.borrow().iter() {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(*bar, 1.5, 1.5).fill();
            }
        }}
        #[unsafe(method(didChangeText))]
        fn did_change_text(&self) { unsafe {
            self.ivars().selected_image.set(None);
            let _: () = msg_send![super(self), didChangeText];
            let base = self.ivars().base_directory.borrow().clone();
            style_with_base(self, base.as_deref());
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
    text.setTextColor(Some(&NSColor::colorWithSRGBRed_green_blue_alpha(
        0.09, 0.094, 0.149, 1.0,
    )));
    text.setDrawsBackground(true);
    text.setBackgroundColor(&NSColor::whiteColor());
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
                        return None;
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
fn utf16_range(source: &str, bytes: Range<usize>) -> NSRange {
    NSRange::new(
        source[..bytes.start].encode_utf16().count(),
        source[bytes].encode_utf16().count(),
    )
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
unsafe fn conceal(storage: &NSTextStorage, range: NSRange, collapse_lines: bool) {
    if range.length == 0 {
        return;
    }
    storage.addAttribute_value_range(NSFontAttributeName, &NSFont::systemFontOfSize(0.01), range);
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

unsafe fn add_exclusion(text: &NSTextView, paths: &mut Vec<Retained<NSBezierPath>>, rect: NSRect) {
    paths.push(NSBezierPath::bezierPathWithRect(rect));
    if let Some(container) = text.textContainer() {
        container.setExclusionPaths(&objc2_foundation::NSArray::from_retained_slice(paths));
    }
    if let Some(manager) = text.textLayoutManager() {
        if let Some(content) = manager.textContentManager() {
            manager.ensureLayoutForRange(&content.documentRange());
        }
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
    if let Some(container) = text.textContainer() {
        container.setExclusionPaths(&objc2_foundation::NSArray::new());
    }
    if let Some(native) = native {
        *native.ivars().base_directory.borrow_mut() = base_directory.map(Path::to_path_buf);
        native.ivars().code_panels.borrow_mut().clear();
        native.ivars().line_numbers.borrow_mut().clear();
        native.ivars().list_labels.borrow_mut().clear();
        native.ivars().quote_bars.borrow_mut().clear();
        native.ivars().rule_bars.borrow_mut().clear();
    }
    let source = text.string().to_string();
    let grid_focus = text.window().and_then(|w| w.firstResponder()).and_then(|r| r.downcast::<InlineTable>().ok()).filter(|v|v.superview().is_some_and(|p|std::ptr::eq(&*p,text as &NSView))).map(|v| {
        (v.ivars().range.get().location,v.cells().iter().filter(|c|c.ivars().selected.get()).map(|c|c.position()).collect::<Vec<_>>())
    });
    for child in text.subviews() {
        let inline: bool = msg_send![&*child, isKindOfClass: InlineImage::class()];
        let table: bool = msg_send![&*child, isKindOfClass: InlineTable::class()];
        let task: bool = msg_send![&*child, isKindOfClass: TaskCheckbox::class()];
        let language: bool = msg_send![&*child, isKindOfClass: CodeLanguageButton::class()];
        let placeholder: bool = msg_send![&*child, isKindOfClass: HeadingPlaceholder::class()];
        let copy: bool = msg_send![&*child, isKindOfClass: CodeCopyButton::class()];
        if inline || table || task || language || placeholder || copy {
            child.removeFromSuperview();
        }
    }
    let Some(storage) = text.textStorage() else {
        return;
    };
    let selection: NSRange = msg_send![text, selectedRange];
    let focused = text.window().is_some_and(|window| {
        let responder: *mut AnyObject = msg_send![&*window, firstResponder];
        responder == text as *const NSTextView as *mut AnyObject
    });
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
    storage.beginEditing();
    for key in [
        NSBackgroundColorAttributeName,
        NSLinkAttributeName,
        NSUnderlineStyleAttributeName,
        NSStrikethroughStyleAttributeName,
        NSParagraphStyleAttributeName,
    ] {
        storage.removeAttribute_range(key, all);
    }
    storage.addAttribute_value_range(NSFontAttributeName, &base, all);
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setMinimumLineHeight(NOTE_LINE_HEIGHT);
    storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, all);
    storage.addAttribute_value_range(NSKernAttributeName, &objc2_foundation::NSNumber::new_f64(0.2), all);
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
                let paragraph = NSMutableParagraphStyle::new();
                paragraph.setMinimumLineHeight(size + 6.0);
                storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, range);
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
    for range in hidden_syntax(&source, active.as_ref()) {
        conceal(&storage, utf16_range(&source, range), false);
    }
    for block in &blocks {
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setFirstLineHeadIndent(block.indent);
        paragraph.setHeadIndent(block.indent);
        paragraph.setMinimumLineHeight(24.0);
        storage.addAttribute_value_range(
            NSParagraphStyleAttributeName,
            &paragraph,
            utf16_range(&source, block.bytes.clone()),
        );
        conceal(&storage, utf16_range(&source, block.prefix.clone()), false);
    }
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
    let image_specs = inline_images(
        &source,
        base_directory,
        (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0).max(80.0),
    );
    for spec in &image_specs {
        conceal(&storage, spec.range, true);
        let paragraph = NSMutableParagraphStyle::new();
        paragraph.setMinimumLineHeight(0.01);
        paragraph.setMaximumLineHeight(0.01);
        paragraph.setParagraphSpacing(if spec.link.wrap {
            6.0
        } else {
            spec.size.height + 12.0
        });
        storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, spec.range);
    }
    let tables = table_previews(&source)
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
    let mut terminal_separator = None;
    for table in &tables {
        conceal(&storage, utf16_range(&source, table.bytes.clone()), true);
        // GFM needs a blank separator before prose. Reserve space for the
        // table and its controls, but don't draw that separator as a text row.
        if let Some(separator) = table.separator_line(&source) {
            if text.isEditable() && separator.end == source.len() && !table.wraps(text, handle_room) {
                terminal_separator = Some((table, utf16_range(&source, separator.clone()).location));
            }
            conceal(&storage, utf16_range(&source, separator), true);
        }
        if !table.wraps(text, handle_room) {
            style_table_anchor(&storage, &source, table, handle_room, 0.0);
        }
    }
    storage.endEditing();
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
            if let Some((table, separator)) = terminal_separator {
                // TextKit places the extra line for the end-of-document caret
                // below a terminal separator even when that separator is
                // collapsed. Count its offset toward the row handle's space.
                if let Some(fragment) = layout_fragment(text, separator) {
                    let lines = fragment.textLineFragments();
                    if lines.count() > 1 {
                        let inset = lines.lastObject().unwrap().typographicBounds().origin.y;
                        style_table_anchor(&storage, &source, table, handle_room, inset);
                        manager.ensureLayoutForRange(&document);
                    }
                }
            }
        }
    }
    if let Some(undo) = &undo {
        let _: () = msg_send![undo, enableUndoRegistration];
    }
    if let Some(native) = native {
        for rule in &rules {
            if let Some(frame) = layout_fragment_rect(text, source[..rule.bytes.start].encode_utf16().count()) {
                native.ivars().rule_bars.borrow_mut().push(NSRect::new(
                    NSPoint::new(EDITOR_INSET + LINE_PADDING, frame.origin.y + frame.size.height / 2.0),
                    NSSize::new((text.bounds().size.width - 2.0 * (EDITOR_INSET + LINE_PADDING)).max(1.0), 1.0),
                ));
            }
        }
        for block in &blocks {
            let Some(frame) =
                layout_fragment_rect(text, source[..block.prefix.start].encode_utf16().count())
            else {
                continue;
            };
            if let Some(label) = &block.label {
                let size = if label == "•" { (28.0_f64 - 4.0 * block.list_depth as f64).max(16.0) } else { 16.0 };
                let width = label.chars().count() as f64 * 9.0;
                native.ivars().list_labels.borrow_mut().push((
                    label.clone(),
                    NSPoint::new(
                        EDITOR_INSET + LINE_PADDING + block.indent - width - 8.0,
                        frame.origin.y + if label == "•" { 3.0 - (size - 16.0) / 2.0 } else { 1.0 },
                    ),
                    size,
                ));
            }
            for depth in 0..block.quote_depth {
                native.ivars().quote_bars.borrow_mut().push(NSRect::new(
                    NSPoint::new(
                        EDITOR_INSET + LINE_PADDING + depth as f64 * 20.0,
                        frame.origin.y + 2.0,
                    ),
                    NSSize::new(3.0, (frame.size.height - 4.0).max(20.0)),
                ));
            }
        }
    }
    let mut image_frames: Vec<Option<NSRect>> = Vec::new();
    let mut exclusions = Vec::new();
    // Install floating obstacles in source order. Each exclusion can reflow
    // the anchors of the images and tables that follow it.
    let mut floating_tables = tables.iter().filter(|t| t.wraps(text, handle_room)).peekable();
    for next_image in image_specs.iter().map(Some).chain(std::iter::once(None)) {
        while let Some(&table) = floating_tables.peek() {
            if next_image.is_some_and(|image| image.link.bytes.start < table.bytes.start) { break; }
            floating_tables.next();
            if let Some(anchor) = layout_fragment_rect(text, table.anchor(&source).location) {
                let width = table.widths(text, handle_room).iter().sum::<f64>() + handle_room;
                let frame = NSRect::new(
                    NSPoint::new(0., anchor.origin.y - text.textContainerOrigin().y + TABLE_TOP_PADDING),
                    NSSize::new(width + LINE_PADDING, table.height() + handle_room),
                );
                add_exclusion(text, &mut exclusions, frame);
            }
        }
        let Some(spec) = next_image else { break };
        let Some(local) = layout_fragment_rect(text, spec.range.location) else {
            image_frames.push(None);
            continue;
        };
        let available =
            (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0 - spec.size.width).max(0.0);
        let mut frame = NSRect::new(
            NSPoint::new(EDITOR_INSET + available * spec.link.x, (local.origin.y + 6.0 + spec.link.y).max(text.textContainerOrigin().y)),
            spec.size,
        );
        // A later image anchor clears an earlier image when their rectangles overlap.
        let bottom = image_frames
            .iter()
            .flatten()
            .filter(|previous| {
                frame.origin.x < previous.origin.x + previous.size.width
                    && previous.origin.x < frame.origin.x + frame.size.width
                    && previous.origin.y < frame.origin.y + frame.size.height + 12.0
                    && frame.origin.y < previous.origin.y + previous.size.height + 12.0
            })
            .map(|r| r.origin.y + r.size.height + 12.0)
            .fold(frame.origin.y, f64::max);
        if bottom > frame.origin.y {
            let p = NSMutableParagraphStyle::new();
            p.setMinimumLineHeight(0.01);
            p.setMaximumLineHeight(0.01);
            p.setParagraphSpacing(if spec.link.wrap {
                6.0
            } else {
                spec.size.height + 12.0
            });
            p.setParagraphSpacingBefore(bottom - frame.origin.y);
            if let Some(undo) = &undo {
                let _: () = msg_send![undo, disableUndoRegistration];
            }
            storage.addAttribute_value_range(NSParagraphStyleAttributeName, &p, spec.range);
            if let Some(undo) = &undo {
                let _: () = msg_send![undo, enableUndoRegistration];
            }
            if let Some(manager) = text.textLayoutManager() {
                if let Some(content) = manager.textContentManager() {
                    manager.ensureLayoutForRange(&content.documentRange());
                }
            }
            if let Some(local) = layout_fragment_rect(text, spec.range.location) {
                frame.origin.y = local.origin.y + 6.0 + spec.link.y;
            }
            frame.origin.y = frame.origin.y.max(bottom);
        }
        image_frames.push(Some(frame));
        if spec.link.wrap {
            let origin = text.textContainerOrigin();
            let container_width = (text.bounds().size.width - 2.0 * EDITOR_INSET).max(80.0);
            let mut left = (frame.origin.x - origin.x - 8.0).max(0.0);
            let mut right =
                (frame.origin.x - origin.x + frame.size.width + 8.0).min(container_width);
            // Avoid one-character columns when a freely positioned image leaves a tiny sliver.
            if left < 80.0 {
                left = 0.0;
            }
            if container_width - right < 80.0 {
                right = container_width;
            }
            let rect = NSRect::new(
                NSPoint::new(left, frame.origin.y - origin.y),
                NSSize::new(right - left, frame.size.height + 8.0),
            );
            add_exclusion(text, &mut exclusions, rect);
        }
    }
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
            let panel = NSRect::new(
                NSPoint::new(EDITOR_INSET, top.origin.y),
                NSSize::new(
                    (text.bounds().size.width - 2.0 * EDITOR_INSET).max(80.0),
                    bottom.origin.y + footer_height - top.origin.y,
                ),
            );
            if let Some(native) = native {
                native.ivars().code_panels.borrow_mut().push(panel);
            }
            let allocated = CodeLanguageButton::alloc(mtm).set_ivars(CodeLanguageState {
                text: Weak::new(text),
                range: utf16_range(&source, code.open.clone()),
                fingerprint: fingerprint(&source),
                fence: code.fence.clone(),
                language: code.language.clone(),
            });
            let button: Retained<CodeLanguageButton> = msg_send![super(allocated), initWithFrame:NSRect::new(NSPoint::new(EDITOR_INSET + 12.0, top.origin.y + 4.0), NSSize::new(160.0, 28.0)), pullsDown:false];
            for (id, title) in CODE_LANGUAGES {
                button.addItemWithTitle(&NSString::from_str(title));
                if code_highlight::canonical_language(&code.language) == *id {
                    button.selectItemWithTitle(&NSString::from_str(title));
                }
            }
            if !CODE_LANGUAGES
                .iter()
                .any(|(id, _)| code_highlight::canonical_language(&code.language) == *id)
            {
                button.addItemWithTitle(&NSString::from_str(&code.language));
                button.selectItemWithTitle(&NSString::from_str(&code.language));
            }
            button.setBordered(false);
            button.setFont(Some(&crate::font::regular(13.0)));
            button.setEnabled(text.isEditable());
            button.setTarget(Some(&button));
            button.setAction(Some(sel!(changeLanguage:)));
            text.addSubview(&button);
            let content_end = if code.close.is_empty() {
                code.bytes.end
            } else {
                code.close.start
            };
            let body = &source[code.open.end..content_end];
            let allocated = CodeCopyButton::alloc(mtm).set_ivars(CodeCopyState {
                code: body.strip_suffix('\n').unwrap_or(body).to_string(),
            });
            let copy: Retained<CodeCopyButton> = msg_send![super(allocated), initWithFrame:NSRect::new(NSPoint::new(panel.origin.x + panel.size.width - 64.0, top.origin.y + 6.0), NSSize::new(52.0, 24.0))];
            copy.setTitle(&NSString::from_str("복사"));
            copy.setBordered(false);
            copy.setFont(Some(&crate::font::regular(12.0)));
            copy.setTarget(Some(&copy));
            copy.setAction(Some(sel!(copyCode:)));
            text.addSubview(&copy);
            let mut offset = code.open.end;
            for (number, line) in body.split_inclusive('\n').enumerate() {
                if let (Some(native), Some(frame)) = (
                    native,
                    layout_fragment_rect(text, source[..offset].encode_utf16().count()),
                ) {
                    native.ivars().line_numbers.borrow_mut().push((
                        (number + 1).to_string(),
                        NSPoint::new(EDITOR_INSET + 12.0, frame.origin.y + 1.0),
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
                    let allocated = HeadingPlaceholder::alloc(mtm).set_ivars(());
                    let label: Retained<HeadingPlaceholder> = msg_send![super(allocated), initWithFrame:NSRect::new(NSPoint::new(EDITOR_INSET + LINE_PADDING, frame.origin.y), NSSize::new(240.0, heading_size(level) + 10.0))];
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
        for (event, bytes) in Parser::new_ext(&source, Options::ENABLE_TASKLISTS).into_offset_iter()
        {
            let Event::TaskListMarker(checked) = event else {
                continue;
            };
            let range = utf16_range(&source, bytes);
            let Some(local) = layout_fragment_rect(text, range.location) else {
                continue;
            };
            let byte = byte_for_utf16(&source, range.location);
            let indent = blocks
                .iter()
                .find(|b| b.bytes.start <= byte && byte < b.bytes.end)
                .map_or(0.0, |b| b.indent);
            let frame = NSRect::new(
                NSPoint::new(
                    EDITOR_INSET + LINE_PADDING + indent - 24.0,
                    local.origin.y + 1.0,
                ),
                NSSize::new(18.0, 18.0),
            );
            let allocated = TaskCheckbox::alloc(mtm).set_ivars(CheckboxState {
                text: Weak::new(text),
                range,
                fingerprint: fingerprint(&source),
            });
            let checkbox: Retained<TaskCheckbox> = msg_send![super(allocated),initWithFrame:frame];
            checkbox.setTitle(&NSString::from_str(""));
            checkbox.setButtonType(NSButtonType::Switch);
            checkbox.setState(if checked { 1 } else { 0 });
            checkbox.setEnabled(text.isEditable());
            checkbox.setTarget(Some(&checkbox));
            checkbox.setAction(Some(sel!(toggle:)));
            text.addSubview(&checkbox);
        }
        for table in tables {
            let Some(local) = layout_fragment_rect(text, table.anchor(&source).location) else {
                continue;
            };
            let widths = table.widths(text, handle_room);
            let grid_width = widths.iter().sum::<f64>();
            let width = grid_width + handle_room;
            let frame = NSRect::new(
                NSPoint::new(EDITOR_INSET, local.origin.y + TABLE_TOP_PADDING),
                NSSize::new(width, table.height() + handle_room),
            );
            let allocated = InlineTable::alloc(mtm).set_ivars(InlineTableState {
                text: Weak::new(text),
                range: std::cell::Cell::new(utf16_range(&source, table.bytes.clone())),
                base_directory: base_directory.map(Path::to_path_buf),
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
                        fingerprint: std::cell::Cell::new(fingerprint(&source)),
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
                base: base_directory.map(Path::to_path_buf),
                link: spec.link,
                fingerprint: fingerprint(&source),
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
            // A floating image at the end of a short note must remain scrollable.
            if frame.origin.y + frame.size.height + 30.0 > text.frame().size.height {
                let mut size = text.frame().size;
                size.height = frame.origin.y + frame.size.height + 30.0;
                text.setFrameSize(size);
            }
        }
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
    range: NSRange,
    fingerprint: u64,
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
            if fingerprint(&source)!=self.ivars().fingerprint {return;}
            let selection=text.selectedRanges();
            let replacement=NSString::from_str(if self.state()==1 {"[x]"}else{"[ ]"});
            let _:()=msg_send![&*text,insertText:&*replacement,replacementRange:self.ivars().range];
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
        #[unsafe(method(commitCell:))]
        fn commit_cell(&self, _sender: Option<&AnyObject>) {
            unsafe { commit_table_cell(self); }
        }
    }
);

include!("table_controls.rs");
include!("table_grid.rs");
include!("block_caret.rs");

const TABLE_COLUMN_WIDTH: f64 = 140.0;

struct InlineTableState {
    text: Weak<NSTextView>,
    range: std::cell::Cell<NSRange>,
    base_directory: Option<PathBuf>,
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
}
impl InlineTableSpec {
    fn widths(&self, text: &NSTextView, handle_room: f64) -> Vec<f64> {
        let columns = self.rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
        let available = (text.bounds().size.width - 2.0 * EDITOR_INSET - 8.0).max(80.0) - handle_room;
        let cell_width = TABLE_COLUMN_WIDTH.min(available / columns as f64);
        let mut widths: Vec<_> = (0..columns).map(|c| self.column_width(c, cell_width)).collect();
        let scale = (available / widths.iter().sum::<f64>()).min(1.);
        for width in &mut widths { *width *= scale; }
        widths
    }

    fn wraps(&self, text: &NSTextView, handle_room: f64) -> bool {
        self.layout["wrap"].as_bool() == Some(true)
            && self.widths(text, handle_room).iter().sum::<f64>() + handle_room + LINE_PADDING * 2.
                < text.bounds().size.width - EDITOR_INSET * 2.
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

unsafe fn style_table_anchor(
    storage: &NSTextStorage,
    source: &str,
    table: &InlineTableSpec,
    handle_room: f64,
    caret_inset: f64,
) {
    let paragraph = NSMutableParagraphStyle::new();
    paragraph.setMinimumLineHeight(0.01);
    paragraph.setMaximumLineHeight(0.01);
    let bottom_padding = if handle_room > 0.0 { 2.0 } else { 6.0 };
    paragraph.setParagraphSpacing(
        table.height() + TABLE_TOP_PADDING + (handle_room + bottom_padding - caret_inset).max(0.0),
    );
    storage.addAttribute_value_range(NSParagraphStyleAttributeName, &paragraph, table.anchor(source));
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
        let desired = link.width.unwrap_or(natural.width.min(320.0));
        let scale = desired.min(max_width) / natural.width;
        images.push(InlineImageSpec {
            link,
            range: utf16_range(source, bytes),
            image,
            size: NSSize::new(natural.width * scale, natural.height * scale),
        });
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
