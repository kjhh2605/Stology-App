//! Native document outline. UI callbacks enqueue intents; the application owns persistence.
use crate::{mtm, rect, strv, theme};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
    sel, DefinedClass, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
    path::PathBuf,
};

const FILE_URL_TYPE: &str = "public.file-url";
const DRAG_TYPE: &str = "app.stology.local.document";
#[derive(Clone, Debug)]
pub enum Event {
    Open(Value),
    OpenFinder(PathBuf),
    Analyze(Value),
    Reread(Value),
    Annotate(Value),
    Reconvert(Value),
    ToggleFolder(String, bool),
    Rename(Value),
    CommitRename(Value, String),
    Move(Value, String),
    CopyFiles(Vec<String>, String),
    NewFolder(String),
    RenameFolder(String),
    NewNote(String),
}
thread_local! { static EVENTS: RefCell<Vec<Event>> = const { RefCell::new(Vec::new()) }; }
fn enqueue(event: Event) {
    EVENTS.with(|q| q.borrow_mut().push(event));
}
pub fn drain() -> Vec<Event> {
    EVENTS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

struct Node {
    key: Retained<NSString>,
    path: String,
    document: Option<Value>,
}
struct RenameSession {
    document: Value,
    field: Retained<NSTextField>,
    original: String,
    ink: Option<Retained<NSColor>>,
}
impl Node {
    fn folder(&self) -> bool {
        self.document.is_none()
    }
}
pub struct TreeState {
    nodes: Vec<Node>,
    root_path: Option<PathBuf>,
    // Data source, delegate and menu targets are non-owning AppKit references.
    outline: Retained<NSOutlineView>,
    restoring: Cell<bool>,
    context_document: RefCell<Option<Value>>,
    context_parent: RefCell<String>,
    active_document: RefCell<Option<String>>,
    rename: RefCell<Option<RenameSession>>,
    visible_paths: RefCell<Option<HashSet<String>>>,
    analysis_busy: Cell<bool>,
}
fn supports_analysis(document: &Value) -> bool {
    matches!(
        document["kind"].as_str(),
        Some("md" | "pdf" | "pptx" | "png" | "jpg" | "jpeg" | "heic")
    )
}
fn analysis_enabled(document: &Value, busy: bool) -> bool {
    supports_analysis(document)
        && document["missing"] != true
        && document["analysis_state"] != "analysed"
        && !busy
}
fn analysis_label(document: &Value) -> &'static str {
    match document["analysis_state"].as_str() {
        Some("analysed") => "분석됨",
        Some("stale") => "정리 필요",
        _ => "미분석",
    }
}
/// Reading again only makes sense for a material that has been read.
fn reread_enabled(document: &Value, busy: bool) -> bool {
    supports_analysis(document)
        && document["missing"] != true
        && matches!(document["analysis_state"].as_str(), Some("analysed" | "stale"))
        && !busy
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}
pub(crate) fn file_extension(document: &Value) -> String {
    document["path"]
        .as_str()
        .and_then(|path| std::path::Path::new(path).extension())
        .and_then(|ext| ext.to_str())
        .filter(|ext| !ext.is_empty())
        .or_else(|| document["kind"].as_str().filter(|kind| !kind.is_empty()))
        .unwrap_or("FILE")
        .to_uppercase()
}
unsafe fn key(item: &AnyObject) -> String {
    let s: Retained<NSString> = msg_send![item, description];
    s.to_string()
}
struct DocumentCellState {
    title: Retained<NSString>,
    badge: Option<Retained<NSTextField>>,
}
define_class!(
    #[unsafe(super(NSTableCellView))]
    #[name="StologyDocumentCell"]
    #[thread_kind=MainThreadOnly]
    #[ivars=DocumentCellState]
    struct DocumentCell;
    unsafe impl NSObjectProtocol for DocumentCell {}
    impl DocumentCell {
        #[unsafe(method(layout))]
        fn layout_cell(&self) { unsafe {
            let _: () = msg_send![super(self), layout];
            let width = self.bounds().size.width;
            let available = (width - 6.).max(0.);
            let badge_width = if self.ivars().badge.is_some() { theme::FILE_TYPE_BADGE_WIDTH.min(available) } else { 0. };
            let right = if badge_width > 0. { badge_width + 12. } else { 4. };
            let title_width = (width - 28. - right).max(0.);
            if let Some(label) = self.textField() {
                label.setFrame(rect(28., 9., title_width, 20.));
            }
            if let Some(badge) = &self.ivars().badge {
                badge.setFrame(rect((width - badge_width - 6.).max(0.), 10., badge_width, theme::FILE_TYPE_BADGE_HEIGHT));
            }
            // At extreme nesting, preserve the filetype badge before decoration.
            if let Some(icon) = self.imageView() { icon.setHidden(width - right < 26.); }
        }}
        #[unsafe(method(setBackgroundStyle:))]
        fn set_background(&self, _style: NSBackgroundStyle) { unsafe {
            let _: () = msg_send![super(self), setBackgroundStyle:NSBackgroundStyle::Normal];
        }}
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> { unsafe {
            if self.textField().is_some_and(|field| field.isEditable()) {
                msg_send![super(self), hitTest:point]
            } else { None }
        }}
    }
);
define_class!(
    #[unsafe(super(NSOutlineView))]
    #[name="StologyDocumentOutline"]
    #[thread_kind=MainThreadOnly]
    #[ivars=Cell<Option<isize>>]
    struct DocumentOutline;
    unsafe impl NSObjectProtocol for DocumentOutline {}
    impl DocumentOutline {
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self, event: &NSEvent) { unsafe { self.show_context(event); } }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                self.show_context(event);
            } else {
                let _: () = msg_send![super(self), mouseDown: event];
            }
        }}
    }
);
impl DocumentOutline {
    unsafe fn show_context(&self, event: &NSEvent) {
        let Some(menu) = self.menu() else { return };
        let index = self.rowAtPoint(self.convertPoint_fromView(event.locationInWindow(), None));
        let row = (index >= 0).then(|| self.rowViewAtRow_makeIfNecessary(index, true)).flatten();
        self.ivars().set(Some(index));
        if let Some(row) = &row {
            let _: () = msg_send![row, setContextOutlined: true];
            row.displayIfNeeded();
        }
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
        if let Some(row) = row {
            let _: () = msg_send![&row, setContextOutlined: false];
        }
        self.ivars().set(None);
    }
}


define_class!(
    #[unsafe(super(NSButton))]
    #[name="StologyWorkspaceRow"]
    #[thread_kind=MainThreadOnly]
    pub(crate) struct WorkspaceRow;
    unsafe impl NSObjectProtocol for WorkspaceRow {}
    impl WorkspaceRow {
        #[unsafe(method(rightMouseDown:))]
        fn right_down(&self, event: &NSEvent) { unsafe { self.show_context(event); } }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) { unsafe {
            if event.modifierFlags().contains(NSEventModifierFlags::Control) {
                self.show_context(event);
            } else {
                let _: () = msg_send![super(self), mouseDown: event];
            }
        }}
    }
);
impl WorkspaceRow {
    unsafe fn show_context(&self, event: &NSEvent) {
        if let Some(target) = self.target() {
            let _: () = msg_send![&target, workspaceContextWillOpen: self];
        }
        let Some(menu) = self.menu() else { return };
        let Some(container) = self.superview() else { return };
        let outline = NSView::initWithFrame(NSView::alloc(mtm()), container.bounds());
        outline.setWantsLayer(true);
        if let Some(layer) = outline.layer() {
            let _: () = msg_send![&layer,setCornerRadius:theme::WORKSPACE_SELECTION_RADIUS];
            let _: () = msg_send![&layer,setBorderWidth:1.0_f64];
            let color = NSColor::colorWithSRGBRed_green_blue_alpha(138./255.,123./255.,223./255.,1.).CGColor();
            let _: () = msg_send![&layer,setBorderColor:&*color];
        }
        container.addSubview(&outline);
        container.displayIfNeeded();
        NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self);
        outline.removeFromSuperview();
    }
}
struct RowState {
    outlined: Cell<bool>,
    active: Cell<bool>,
}
define_class!(
    #[unsafe(super(NSTableRowView))]
    #[name="StologyDocumentRow"]
    #[thread_kind=MainThreadOnly]
    #[ivars=RowState]
    struct DocumentRow;
    unsafe impl NSObjectProtocol for DocumentRow {}
    impl DocumentRow {
        #[unsafe(method(interiorBackgroundStyle))]
        fn interior_style(&self) -> NSBackgroundStyle { NSBackgroundStyle::Normal }
        #[unsafe(method(drawBackgroundInRect:))]
        fn draw_background(&self, _rect: NSRect) {
            // The current file keeps the former folder-selection wash even
            // when keyboard focus moves elsewhere. Folders have no highlight.
            if self.ivars().active.get() {
                theme::document_selection_color().setFill();
                let b = self.bounds();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(3., 3., (b.size.width - 6.).max(0.), (b.size.height - 6.).max(0.)), 10., 10.
                ).fill();
            }
            if self.ivars().outlined.get() {
                let b = self.bounds();
                NSColor::colorWithSRGBRed_green_blue_alpha(138./255.,123./255.,223./255.,1.).setStroke();
                let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    rect(3.5, 3.5, (b.size.width - 7.).max(0.), (b.size.height - 7.).max(0.)), 7., 7.);
                path.setLineWidth(1.);
                path.stroke();
            }
        }
        #[unsafe(method(setContextOutlined:))]
        fn set_outlined(&self, outlined: bool) {
            self.ivars().outlined.set(outlined);
            self.setNeedsDisplay(true);
        }
        #[unsafe(method(drawSelectionInRect:))]
        fn draw_selection(&self, _rect: NSRect) {}
        #[unsafe(method(setDocumentActive:))]
        fn set_active(&self, active: bool) {
            self.ivars().active.set(active);
            self.setNeedsDisplay(true);
        }
        #[unsafe(method(isDocumentActive))]
        fn active(&self) -> bool { self.ivars().active.get() }
    }
);
define_class!(
    #[unsafe(super(NSObject))]
    #[name="StologyDocumentTree"]
    #[thread_kind=MainThreadOnly]
    #[ivars=TreeState]
    pub struct DocumentTree;
    unsafe impl NSObjectProtocol for DocumentTree {}
    unsafe impl NSControlTextEditingDelegate for DocumentTree {
        #[unsafe(method(control:textView:doCommandBySelector:))]
        unsafe fn rename_command(&self, control: &NSControl, text: &NSTextView, command: Sel) -> bool { (|| {
            if !self.is_rename_field(control) { return false; }
            // Let the input method finish/cancel its composition before treating
            // Return or Escape as a command for the filename itself.
            let marked: bool = msg_send![text, hasMarkedText];
            if marked { return false; }
            let commit = command == sel!(insertNewline:);
            if !commit && command != sel!(cancelOperation:) { return false; }
            let window = control.window();
            self.finish_rename(commit);
            if let Some(window) = window { window.makeFirstResponder(Some(&self.ivars().outline)); }
            true
        })() }
        #[unsafe(method(controlTextDidEndEditing:))]
        fn rename_ended(&self, notification: &NSNotification) { unsafe {
            let own = notification.object().is_some_and(|object| {
                self.ivars().rename.borrow().as_ref().is_some_and(|rename|
                    Retained::as_ptr(&object) == Retained::as_ptr(&rename.field).cast::<AnyObject>()
                )
            });
            if own { self.finish_rename(true); }
        }}
    }
    unsafe impl NSTextFieldDelegate for DocumentTree {}
    unsafe impl NSOutlineViewDataSource for DocumentTree {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        unsafe fn count(&self, _outline: &NSOutlineView, item: Option<&AnyObject>) -> isize { self.children(item).len() as isize }
        #[unsafe(method_id(outlineView:child:ofItem:))]
        unsafe fn child(&self, _outline: &NSOutlineView, index: isize, item: Option<&AnyObject>) -> Retained<AnyObject> {
            Retained::cast_unchecked(self.children(item)[index as usize].key.clone())
        }
        #[unsafe(method(outlineView:isItemExpandable:))]
        unsafe fn expandable(&self, _outline: &NSOutlineView, item: &AnyObject) -> bool { self.node(item).is_some_and(Node::folder) }
        #[unsafe(method_id(outlineView:objectValueForTableColumn:byItem:))]
        unsafe fn display(&self, _outline: &NSOutlineView, _column: Option<&NSTableColumn>, item: Option<&AnyObject>) -> Option<Retained<AnyObject>> { (|| {
            let n = self.node(item?)?;
            let title = self.title(n);
            Some(Retained::cast_unchecked(NSString::from_str(&title)))
        })() }
        #[unsafe(method_id(outlineView:pasteboardWriterForItem:))]
        unsafe fn writer(&self, _outline: &NSOutlineView, item: &AnyObject) -> Option<Retained<ProtocolObject<dyn NSPasteboardWriting>>> { (|| {
            if self.ivars().rename.borrow().is_some() { return None; }
            let d = self.node(item)?.document.as_ref()?;
            if d["missing"] == true { return None; }
            let p = NSPasteboardItem::new();
            p.setString_forType(&NSString::from_str(&strv(d,"id")), &NSString::from_str(DRAG_TYPE));
            Some(ProtocolObject::from_retained(p))
        })() }
        #[unsafe(method(outlineView:draggingSession:willBeginAtPoint:forItems:))]
        unsafe fn begin_drag(&self, _outline: &NSOutlineView, _session: &NSDraggingSession, _point: NSPoint, _items: &NSArray) {
            // A drag must not open the pressed row as a side effect of starting it.
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
        }
        #[unsafe(method(outlineView:draggingSession:endedAtPoint:operation:))]
        fn end_drag(&self, _outline: &NSOutlineView, _session: &NSDraggingSession, _point: NSPoint, _operation: NSDragOperation) { unsafe {
            let active = self.ivars().active_document.borrow().clone();
            self.select_document(active.as_deref());
        }}
        #[unsafe(method(outlineView:validateDrop:proposedItem:proposedChildIndex:))]
        unsafe fn validate_drop(&self, outline: &NSOutlineView, info: &ProtocolObject<dyn NSDraggingInfo>, item: Option<&AnyObject>, _index: isize) -> NSDragOperation {
            if let Some((_, destination)) = self.external_drop(info) {
                let target = self.ivars().nodes.iter().find(|n| n.folder() && n.path == destination).unwrap();
                outline.setDropItem_dropChildIndex(Some(&target.key), -1);
                return NSDragOperation::Copy;
            }
            if self.drop_destination(info, item).is_some() {
                let root: &AnyObject = &self.ivars().nodes[0].key;
                outline.setDropItem_dropChildIndex(item.or(Some(root)), -1);
                NSDragOperation::Move
            } else { NSDragOperation::None }
        }
        #[unsafe(method(outlineView:acceptDrop:item:childIndex:))]
        unsafe fn accept_drop(&self, _outline: &NSOutlineView, info: &ProtocolObject<dyn NSDraggingInfo>, item: Option<&AnyObject>, _index: isize) -> bool {
            if let Some((paths, destination)) = self.external_drop(info) {
                enqueue(Event::CopyFiles(paths, destination));
                true
            } else if let Some((document, destination)) = self.drop_destination(info, item) { enqueue(Event::Move(document, destination)); true } else { false }
        }
    }
    unsafe impl NSOutlineViewDelegate for DocumentTree {
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        unsafe fn cell_view(&self, outline: &NSOutlineView, column: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> { (|| {
            let n = self.node(item)?;
            let active = self.active(n);
            let width = column.map(|c| c.width()).unwrap_or(outline.bounds().size.width).max(80.);
            let extension = n.document.as_ref().map(file_extension);
            let badge = extension.as_deref().filter(|ext| *ext != "MD").map(|ext| theme::file_type_badge(ext, rect(0., 10., theme::FILE_TYPE_BADGE_WIDTH, theme::FILE_TYPE_BADGE_HEIGHT)));
            let cell: Retained<DocumentCell> = msg_send![super(DocumentCell::alloc(mtm()).set_ivars(DocumentCellState { badge: badge.clone(), title: NSString::from_str(&n.document.as_ref().map(|d| strv(d, "title")).unwrap_or_else(|| self.title(n))) })), initWithFrame:rect(0., 0., width, 38.)];
            let title = self.title(n);
            let visible_title = n.document.as_ref().map(|d| {
                format!("{}{}", strv(d, "title"), if d["missing"] == true { " · 원본 없음" } else { "" })
            }).unwrap_or_else(|| title.clone());
            let label = NSTextField::labelWithString(&NSString::from_str(&visible_title), mtm());
            // Emphasize every matching file, independently of the open document.
            // Ancestor folders keep their regular weight during a search.
            let search_result = n.document.is_some() && self.ivars().visible_paths.borrow().is_some();
            let font = if search_result {
                crate::font::bold(13.)
            } else {
                crate::font::weighted(13., if active { 0.3 } else { 0. })
            };
            label.setFont(Some(&font));
            let ink = if active { theme::indigo() } else { NSColor::colorWithSRGBRed_green_blue_alpha(0.16, 0.17, 0.27, 1.) };
            label.setTextColor(Some(&ink));
            label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            label.setMaximumNumberOfLines(1);
            label.setSelectable(false);
            cell.addSubview(&label);
            cell.setTextField(Some(&label));
            if let Some(badge) = badge { cell.addSubview(&badge); }
            let symbol = if n.folder() {
                if outline.isItemExpanded(Some(item)) { "folder-open" } else { "folder" }
            } else { theme::file_type_symbol(extension.as_deref().unwrap_or("")) };
            if let Some(icon) = theme::icon_view(symbol, rect(3., 9., 20., 20.)) {
                let tint = if let Some(ext) = extension.as_deref().filter(|ext| *ext != "MD") {
                    theme::file_type_tint(ext)
                } else if active { theme::indigo() } else { NSColor::colorWithSRGBRed_green_blue_alpha(0.36, 0.38, 0.52, 1.) };
                icon.setContentTintColor(Some(&tint));
                cell.addSubview(&icon);
                cell.setImageView(Some(&icon));
            }
            let _: () = msg_send![&cell, setAccessibilityLabel: &*NSString::from_str(&title)];
            let _: () = msg_send![&cell, setAccessibilityValue: &*NSString::from_str(if active { "현재 문서" } else { "" })];
            cell.setNeedsLayout(true);
            cell.layoutSubtreeIfNeeded();
            Some(cell.into_super().into_super())
        })() }
        #[unsafe(method_id(outlineView:rowViewForItem:))]
        unsafe fn row_view(&self, outline: &NSOutlineView, item: &AnyObject) -> Option<Retained<NSTableRowView>> { (|| {
            let n = self.node(item)?;
            let row: Retained<DocumentRow> = msg_send![super(DocumentRow::alloc(mtm()).set_ivars(RowState {
                outlined: Cell::new(false), active: Cell::new(self.active(n))
            })), initWithFrame: rect(0., 0., outline.bounds().size.width, 38.)];
            row.setBackgroundColor(&NSColor::clearColor());
            Some(row.into_super())
        })() }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_changed(&self, _notification: &NSNotification) { unsafe {
            if self.ivars().restoring.get() || self.ivars().rename.borrow().is_some() { return; }
            let outline = &self.ivars().outline;
            if let Some(item) = outline.itemAtRow(outline.selectedRow()) {
                if let Some(d) = self.node(&item).and_then(|n| n.document.clone()) { enqueue(Event::Open(d)); }
            }
        }}
        #[unsafe(method(outlineViewItemDidExpand:))]
        fn expanded(&self, notification: &NSNotification) { unsafe { self.folder_changed(notification, true); } }
        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn collapsed(&self, notification: &NSNotification) { unsafe { self.folder_changed(notification, false); } }
    }
    unsafe impl NSMenuDelegate for DocumentTree {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_update(&self, menu: &NSMenu) { unsafe {
            let outline = &self.ivars().outline;
            let row = outline.downcast_ref::<DocumentOutline>().and_then(|o| o.ivars().get()).unwrap_or_else(|| outline.clickedRow());
            let item = outline.itemAtRow(row);
            self.populate_context_menu(menu, item.as_deref());
        }}
    }
    impl DocumentTree {
        #[unsafe(method(openFinder:))]
        fn open_finder(&self, sender: &NSMenuItem) {
            if let Some(path) = sender.representedObject().and_then(|object| object.downcast::<NSString>().ok()) {
                enqueue(Event::OpenFinder(PathBuf::from(path.to_string())));
            }
        }
        #[unsafe(method(newNote:))]
        fn new_note(&self, _sender: Option<&AnyObject>) { enqueue(Event::NewNote(self.ivars().context_parent.borrow().clone())); }
        #[unsafe(method(renameFolder:))]
        fn rename_folder(&self, _sender: Option<&AnyObject>) { enqueue(Event::RenameFolder(self.ivars().context_parent.borrow().clone())); }
        #[unsafe(method(newFolder:))]
        fn new_folder(&self, _sender: Option<&AnyObject>) { enqueue(Event::NewFolder(self.ivars().context_parent.borrow().clone())); }
        #[unsafe(method(renameDocument:))]
        fn rename(&self, _sender: Option<&AnyObject>) {
            // A context action targets the clicked file without opening it.
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
            if let Some(d) = self.ivars().context_document.borrow().clone() { enqueue(Event::Rename(d)); }
        }
        #[unsafe(method(analyzeDocument:))]
        fn analyze(&self, _sender: Option<&AnyObject>) {
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
            if let Some(d) = self.ivars().context_document.borrow().clone().filter(|d| analysis_enabled(d, self.ivars().analysis_busy.get())) {
                enqueue(Event::Analyze(d));
            }
        }
        #[unsafe(method(rereadDocument:))]
        fn reread(&self, _sender: Option<&AnyObject>) {
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
            if let Some(d) = self.ivars().context_document.borrow().clone().filter(|d| reread_enabled(d, self.ivars().analysis_busy.get())) {
                enqueue(Event::Reread(d));
            }
        }
        #[unsafe(method(annotateDocument:))]
        fn annotate(&self, _sender: Option<&AnyObject>) {
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
            if let Some(d) = self.ivars().context_document.borrow().clone().filter(|d| d["kind"] == "pptx" && d["missing"] != true) {
                enqueue(Event::Annotate(d));
            }
        }
        #[unsafe(method(reconvertDocument:))]
        fn reconvert(&self, _sender: Option<&AnyObject>) {
            EVENTS.with(|q| q.borrow_mut().retain(|e| !matches!(e, Event::Open(_))));
            if let Some(d) = self.ivars().context_document.borrow().clone().filter(|d| d["kind"] == "pptx" && d["missing"] != true) {
                enqueue(Event::Reconvert(d));
            }
        }
        #[unsafe(method(toggleFolder:))]
        fn toggle(&self, _sender: Option<&AnyObject>) { unsafe {
            let outline = &self.ivars().outline;
            if let Some(item) = outline.itemAtRow(outline.clickedRow()) {
                if self.node(&item).is_some_and(Node::folder) {
                    // The animator proxy uses AppKit's native disclosure animation.
                    // Direct expand/collapse calls change the rows immediately.
                    let animator: Retained<AnyObject> = msg_send![outline, animator];
                    if outline.isItemExpanded(Some(&item)) {
                        let _: () = msg_send![&animator, collapseItem: &*item];
                    } else {
                        let _: () = msg_send![&animator, expandItem: &*item];
                    }
                }
            }
        }}
    }
);
impl DocumentTree {
    /// A single analysis job runs for the application, across all workspaces.
    pub fn set_analysis_busy(&self, busy: bool) {
        self.ivars().analysis_busy.set(busy);
    }
    /// Preserve the live native field editor across same-workspace refreshes.
    pub fn renaming_workspace(&self) -> Option<String> {
        self.ivars()
            .rename
            .borrow()
            .as_ref()
            .and_then(|rename| rename.document["workspace_id"].as_str().map(str::to_owned))
    }
    fn is_rename_field(&self, control: &NSControl) -> bool {
        self.ivars()
            .rename
            .borrow()
            .as_ref()
            .is_some_and(|rename| std::ptr::eq(control, &*rename.field as &NSControl))
    }
    unsafe fn finish_rename(&self, commit: bool) {
        // Take the session before resigning focus. AppKit can synchronously send
        // an end-editing notification for Return, Escape or a replacement view.
        let Some(rename) = self.ivars().rename.borrow_mut().take() else {
            return;
        };
        let value = rename
            .field
            .currentEditor()
            .map(|editor| editor.string().to_string())
            .unwrap_or_else(|| rename.field.stringValue().to_string());
        rename.field.setDelegate(None);
        rename.field.abortEditing();
        rename.field.setEditable(false);
        rename.field.setSelectable(false);
        rename.field.setBordered(false);
        rename.field.setDrawsBackground(false);
        rename.field.setFocusRingType(NSFocusRingType::None);
        rename.field.setTextColor(rename.ink.as_deref());
        rename.field.setStringValue(&NSString::from_str(if commit {
            &value
        } else {
            &rename.original
        }));
        if commit && value != rename.original {
            enqueue(Event::CommitRename(rename.document, value));
        }
    }
    /// Edit the existing title label using AppKit's shared native field editor.
    /// The caller flushes its current document first; context rename never opens
    /// the target. Retry text and error stay in this same field after a failure.
    pub unsafe fn begin_rename(
        &self,
        document: &Value,
        attempted: Option<&str>,
        error: Option<&str>,
    ) {
        let Some(node) = self.ivars().nodes.iter().find(|node| {
            node.document
                .as_ref()
                .is_some_and(|d| d["id"] == document["id"] && d["missing"] != true)
        }) else {
            return;
        };
        self.finish_rename(false);
        let restoring = self.ivars().restoring.replace(true);
        let outline = &self.ivars().outline;
        outline.expandItem(Some(&self.ivars().nodes[0].key));
        let mut ancestors: Vec<_> = self
            .ivars()
            .nodes
            .iter()
            .filter(|folder| {
                folder.folder()
                    && !folder.path.is_empty()
                    && node.path.starts_with(&format!("{}/", folder.path))
            })
            .collect();
        ancestors.sort_by_key(|folder| folder.path.len());
        for folder in ancestors {
            outline.expandItem(Some(&folder.key));
        }
        let row = outline.rowForItem(Some(&node.key));
        if row >= 0 {
            outline.scrollRowToVisible(row);
            if let Some(view) = outline.viewAtColumn_row_makeIfNecessary(0, row, true) {
                let cell: Retained<NSTableCellView> = Retained::cast_unchecked(view);
                if let Some(field) = cell.textField() {
                    let original = strv(node.document.as_ref().unwrap(), "title");
                    let rename = RenameSession {
                        document: document.clone(),
                        field: field.clone(),
                        original: original.clone(),
                        ink: field.textColor(),
                    };
                    field.setStringValue(&NSString::from_str(attempted.unwrap_or(&original)));
                    field.setEditable(true);
                    field.setSelectable(true);
                    field.setBordered(true);
                    field.setDrawsBackground(true);
                    field.setBackgroundColor(Some(&NSColor::controlBackgroundColor()));
                    field.setFocusRingType(NSFocusRingType::Exterior);
                    if error.is_some() {
                        field.setTextColor(Some(&NSColor::systemRedColor()));
                    }
                    field.setDelegate(Some(ProtocolObject::from_ref(self)));
                    *self.ivars().rename.borrow_mut() = Some(rename);
                    // The field contains the title stem only; its extension badge
                    // remains visible and the persistence layer preserves it.
                    field.selectText(None);
                }
            }
        }
        self.ivars().restoring.set(restoring);
    }
    /// Resolve the context target once, so menu actions never use editor selection.
    pub(crate) unsafe fn populate_context_menu(&self, menu: &NSMenu, item: Option<&AnyObject>) {
        menu.removeAllItems();
        let node = item.and_then(|i| self.node(i));
        let destination = node
            .map(|n| {
                if n.folder() {
                    n.path.clone()
                } else {
                    parent(&n.path).into()
                }
            })
            .unwrap_or_default();
        *self.ivars().context_parent.borrow_mut() = destination;
        let document = node.and_then(|n| n.document.clone());
        *self.ivars().context_document.borrow_mut() = document.clone();
        if document.is_none() {
            if node.is_some_and(|n| n.folder() && !n.path.is_empty()) {
                let rename = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()), &NSString::from_str("폴더 이름 변경…"),
                    Some(sel!(renameFolder:)), &NSString::from_str(""));
                rename.setTarget(Some(self));
                rename.setImage(theme::icon("pencil", 13.).as_deref());
                menu.addItem(&rename);
            }
            let note = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm()),
                &NSString::from_str("새 노트"),
                Some(sel!(newNote:)),
                &NSString::from_str(""),
            );
            note.setTarget(Some(self));
            note.setEnabled(true);
            note.setImage(theme::icon("note-plus", 13.).as_deref());
            menu.addItem(&note);
            let create = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm()),
                &NSString::from_str("새 폴더…"),
                Some(sel!(newFolder:)),
                &NSString::from_str(""),
            );
            create.setTarget(Some(self));
            create.setEnabled(true);
            create.setImage(theme::icon("folder-plus", 13.).as_deref());
            menu.addItem(&create);

        }
        if let Some(d) = document {
            if supports_analysis(&d) {
                let analyze = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &NSString::from_str("분석"),
                    Some(sel!(analyzeDocument:)),
                    &NSString::from_str(""),
                );
                analyze.setTarget(Some(self));
                analyze.setImage(theme::icon("sparkle", 13.).as_deref());
                analyze.setEnabled(analysis_enabled(&d, self.ivars().analysis_busy.get()));
                menu.addItem(&analyze);
                let reread = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &NSString::from_str("꼼꼼하게 다시 정리"),
                    Some(sel!(rereadDocument:)),
                    &NSString::from_str(""),
                );
                reread.setTarget(Some(self));
                reread.setImage(theme::icon("review-document", 13.).as_deref());
                reread.setEnabled(reread_enabled(&d, self.ivars().analysis_busy.get()));
                menu.addItem(&reread);
            }
            if d["kind"] == "pptx" {
                let annotate = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &NSString::from_str("PDF로 필기"),
                    Some(sel!(annotateDocument:)),
                    &NSString::from_str(""),
                );
                annotate.setTarget(Some(self));
                annotate.setImage(theme::icon("markup", 13.).as_deref());
                annotate.setEnabled(d["missing"] != true);
                menu.addItem(&annotate);
                let reconvert = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm()),
                    &NSString::from_str("새 PDF로 변환"),
                    Some(sel!(reconvertDocument:)),
                    &NSString::from_str(""),
                );
                reconvert.setTarget(Some(self));
                reconvert.setImage(theme::icon("convert-document", 13.).as_deref());
                reconvert.setEnabled(d["missing"] != true);
                menu.addItem(&reconvert);
            }
            let rename = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm()),
                &NSString::from_str("이름 변경"),
                Some(sel!(renameDocument:)),
                &NSString::from_str(""),
            );
            rename.setTarget(Some(self));
            rename.setImage(theme::icon("pencil", 13.).as_deref());
            rename.setEnabled(d["missing"] != true);
            menu.addItem(&rename);
        }
        if let Some(node) = node {
            let path = if let Some(document) = &node.document {
                document["absolute_path"].as_str().map(PathBuf::from)
            } else {
                self.ivars().root_path.as_ref().map(|root| root.join(&node.path))
            };
            let finder = NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm()),
                &NSString::from_str("Finder 열기"),
                Some(sel!(openFinder:)),
                &NSString::from_str(""),
            );
            finder.setTarget(Some(self));
            finder.setImage(theme::icon("folder-open", 13.).as_deref());
            finder.setEnabled(path.is_some() && node.document.as_ref().is_none_or(|d| d["missing"] != true));
            if let Some(path) = path {
                finder.setRepresentedObject(Some(&NSString::from_str(&path.to_string_lossy())));
            }
            menu.addItem(&finder);
        }
    }
    pub unsafe fn reveal_folder(&self, path: &str) {
        self.ivars().restoring.set(true);
        let outline = &self.ivars().outline;
        outline.expandItem(Some(&self.ivars().nodes[0].key));
        let mut ancestors: Vec<_> = self
            .ivars()
            .nodes
            .iter()
            .filter(|n| {
                n.folder()
                    && !n.path.is_empty()
                    && (path == n.path || path.starts_with(&format!("{}/", n.path)))
            })
            .collect();
        ancestors.sort_by_key(|n| n.path.len());
        for n in ancestors {
            outline.expandItem(Some(&n.key));
        }
        if let Some(n) = self
            .ivars()
            .nodes
            .iter()
            .find(|n| n.folder() && n.path == path)
        {
            let row = outline.rowForItem(Some(&n.key));
            if row >= 0 {
                outline.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(row as usize),
                    false,
                );
                outline.scrollRowToVisible(row);
            }
        }
        self.ivars().restoring.set(false);
    }
    fn active(&self, node: &Node) -> bool {
        node.document
            .as_ref()
            .is_some_and(|d| self.ivars().active_document.borrow().as_deref() == d["id"].as_str())
    }
    fn title(&self, node: &Node) -> String {
        match &node.document {
            None if node.path.is_empty() => "과목 루트".into(),
            None => node.path.rsplit('/').next().unwrap_or(&node.path).into(),
            Some(d) => {
                let ext = file_extension(d);
                let suffix = if ext != "MD" {
                    format!(" · {ext}")
                } else {
                    String::new()
                };
                format!(
                    "{}{suffix}{}{}",
                    strv(d, "title"),
                    if d["missing"] == true {
                        " · 원본 없음"
                    } else {
                        ""
                    },
                    if supports_analysis(d) {
                        format!(" · {}", analysis_label(d))
                    } else {
                        String::new()
                    }
                )
            }
        }
    }
    unsafe fn node(&self, item: &AnyObject) -> Option<&Node> {
        let k = key(item);
        self.ivars().nodes.iter().find(|n| n.key.to_string() == k)
    }
    fn visible(&self, node: &Node) -> bool {
        self.ivars()
            .visible_paths
            .borrow()
            .as_ref()
            .is_none_or(|paths| paths.contains(&node.path))
    }
    /// Filter filenames while keeping their ancestor folders. Reuse the outline
    /// so typing never replaces AppKit's search field editor.
    pub unsafe fn filter(&self, query: &str, open_folders: &HashSet<String>) -> usize {
        let query = query.trim().to_lowercase();
        let mut paths = HashSet::new();
        let mut count = 0;
        for node in &self.ivars().nodes {
            if let Some(document) = &node.document {
                let filename = node.path.rsplit('/').next().unwrap_or(&node.path);
                if query.is_empty()
                    || strv(document, "title").to_lowercase().contains(&query)
                    || filename.to_lowercase().contains(&query)
                {
                    count += 1;
                    paths.insert(node.path.clone());
                    let mut ancestor = parent(&node.path);
                    while !ancestor.is_empty() {
                        paths.insert(ancestor.to_owned());
                        ancestor = parent(ancestor);
                    }
                }
            }
        }
        paths.insert(String::new());
        self.ivars()
            .visible_paths
            .replace(if query.is_empty() { None } else { Some(paths) });
        self.ivars().restoring.set(true);
        let outline = &self.ivars().outline;
        outline.reloadData();
        outline.collapseItem_collapseChildren(None, true);
        outline.expandItem(Some(&self.ivars().nodes[0].key));
        let mut folders: Vec<_> = self
            .ivars()
            .nodes
            .iter()
            .filter(|n| {
                n.folder()
                    && self.visible(n)
                    && (!query.is_empty() || open_folders.contains(&n.path))
            })
            .collect();
        folders.sort_by_key(|n| n.path.len());
        for folder in folders {
            outline.expandItem(Some(&folder.key));
        }
        let active = self.ivars().active_document.borrow().clone();
        self.select_document(active.as_deref());
        count
    }
    unsafe fn children(&self, item: Option<&AnyObject>) -> Vec<&Node> {
        match item {
            None => vec![&self.ivars().nodes[0]],
            Some(i) => match self.node(i) {
                Some(n) if n.folder() => self
                    .ivars()
                    .nodes
                    .iter()
                    .skip(1)
                    .filter(|c| parent(&c.path) == n.path && self.visible(c))
                    .collect(),
                _ => vec![],
            },
        }
    }
    unsafe fn folder_changed(&self, notification: &NSNotification, expanded: bool) {
        if self.ivars().restoring.get() {
            return;
        }
        if let Some(info) = notification.userInfo() {
            let item: Option<Retained<AnyObject>> =
                msg_send![&info, objectForKey: &*NSString::from_str("NSObject")];
            if let Some(n) = item.as_ref().and_then(|i| self.node(i)) {
                enqueue(Event::ToggleFolder(n.path.clone(), expanded));
                let restoring = self.ivars().restoring.replace(true);
                self.ivars().outline.reloadItem(Some(&n.key));
                self.ivars().restoring.set(restoring);
            }
        }
    }
    /// Resolve the row under the pointer, rather than AppKit's insertion proposal:
    /// a file means its parent, a folder means itself, empty space means root.
    unsafe fn external_drop(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
    ) -> Option<(Vec<String>, String)> {
        if self.ivars().rename.borrow().is_some()
            || !info
                .draggingSourceOperationMask()
                .contains(NSDragOperation::Copy)
        {
            return None;
        }
        let pasteboard = info.draggingPasteboard();
        // Internal/foreign Stology drags must still use the guarded move path.
        if pasteboard
            .stringForType(&NSString::from_str(DRAG_TYPE))
            .is_some()
        {
            return None;
        }
        let items = pasteboard.pasteboardItems()?;
        let mut paths = Vec::new();
        for item in items.iter() {
            let value = item.stringForType(&NSString::from_str(FILE_URL_TYPE))?;
            let url = NSURL::URLWithString(&value)?;
            if !url.isFileURL() {
                return None;
            }
            let path = url.path()?.to_string();
            if !std::path::Path::new(&path).is_file() {
                return None;
            }
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        if paths.is_empty() {
            return None;
        }
        let outline = &self.ivars().outline;
        let point = outline.convertPoint_fromView(info.draggingLocation(), None);
        let row = outline.rowAtPoint(point);
        let destination = outline
            .itemAtRow(row)
            .and_then(|item| self.node(&item))
            .map(|node| {
                if node.folder() {
                    node.path.clone()
                } else {
                    parent(&node.path).to_owned()
                }
            })
            .unwrap_or_default();
        Some((paths, destination))
    }
    unsafe fn drop_destination(
        &self,
        info: &ProtocolObject<dyn NSDraggingInfo>,
        item: Option<&AnyObject>,
    ) -> Option<(Value, String)> {
        if self.ivars().rename.borrow().is_some() {
            return None;
        }
        // A custom pasteboard type alone is insufficient: accept only our live outline's drag.
        let source = info.draggingSource()?;
        if Retained::as_ptr(&source) != Retained::as_ptr(&self.ivars().outline).cast::<AnyObject>()
        {
            return None;
        }
        let id = info
            .draggingPasteboard()
            .stringForType(&NSString::from_str(DRAG_TYPE))?
            .to_string();
        let d = self
            .ivars()
            .nodes
            .iter()
            .filter_map(|n| n.document.as_ref())
            .find(|d| strv(d, "id") == id)?;
        if d["missing"] == true {
            return None;
        }
        let destination = match item {
            Some(i) => {
                let n = self.node(i)?;
                if !n.folder() {
                    return None;
                }
                n.path.clone()
            }
            None => String::new(),
        };
        if parent(&strv(d, "path")) == destination {
            return None;
        }
        Some((d.clone(), destination))
    }
    /// Keep selection synchronized without reopening the document or replacing the focused outline.
    pub unsafe fn select_document(&self, selected_id: Option<&str>) {
        self.ivars().restoring.set(true);
        let previous = self
            .ivars()
            .active_document
            .replace(selected_id.map(str::to_owned));
        let outline = &self.ivars().outline;
        let selected = selected_id.and_then(|id| {
            self.ivars().nodes.iter().find(|n| {
                self.visible(n) && n.document.as_ref().is_some_and(|d| strv(d, "id") == id)
            })
        });
        if let Some(n) = selected {
            outline.expandItem(Some(&self.ivars().nodes[0].key));
            let mut ancestors: Vec<_> = self
                .ivars()
                .nodes
                .iter()
                .filter(|f| {
                    f.folder() && !f.path.is_empty() && n.path.starts_with(&format!("{}/", f.path))
                })
                .collect();
            ancestors.sort_by_key(|f| f.path.len());
            for f in ancestors {
                outline.expandItem(Some(&f.key));
            }
            let row = outline.rowForItem(Some(&n.key));
            if row >= 0 {
                outline.selectRowIndexes_byExtendingSelection(
                    &NSIndexSet::indexSetWithIndex(row as usize),
                    false,
                );
                outline.scrollRowToVisible(row);
            }
        } else {
            outline.deselectAll(None);
        }
        if previous.as_deref() != selected_id {
            for n in &self.ivars().nodes {
                if n.document.as_ref().is_some_and(|d| {
                    d["id"].as_str() == previous.as_deref() || d["id"].as_str() == selected_id
                }) {
                    outline.reloadItem(Some(&n.key));
                    let row = outline.rowForItem(Some(&n.key));
                    if row >= 0 {
                        if let Some(view) = outline.rowViewAtRow_makeIfNecessary(row, false) {
                            let _: () = msg_send![&view, setDocumentActive: self.active(n)];
                        }
                    }
                }
            }
        }
        self.ivars().restoring.set(false);
    }
    pub unsafe fn scroll_position(&self) -> NSPoint {
        self.ivars().outline.visibleRect().origin
    }
    pub unsafe fn restore_scroll(&self, point: NSPoint) {
        self.ivars().outline.scrollPoint(point);
    }
}

pub unsafe fn build(
    tree: &Value,
    selected_id: Option<&str>,
    open_folders: &HashSet<String>,
    frame: NSRect,
) -> (Retained<NSScrollView>, Retained<DocumentTree>) {
    let native: Retained<DocumentOutline> = msg_send![super(DocumentOutline::alloc(mtm()).set_ivars(Cell::new(None))), initWithFrame: rect(0., 0., frame.size.width, frame.size.height)];
    let outline = native.into_super();
    let column = NSTableColumn::initWithIdentifier(
        NSTableColumn::alloc(mtm()),
        &NSString::from_str("document"),
    );
    column.setWidth((frame.size.width - 16.).max(80.));
    column.setMinWidth(60.);
    column.setEditable(false);
    let cell: Retained<NSCell> = msg_send![&column, dataCell];
    cell.setFont(Some(&crate::font::regular(12.)));
    cell.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    outline.addTableColumn(&column);
    outline.setOutlineTableColumn(Some(&column));
    outline.setHeaderView(None);
    outline.setRowHeight(38.);
    outline.setIndentationPerLevel(14.);
    // Expansion must not widen the single column beyond its scroll viewport.
    // Otherwise AppKit adds indentation to the width and clips the selected pill.
    outline.setAutoresizesOutlineColumn(false);
    outline.setColumnAutoresizingStyle(
        NSTableViewColumnAutoresizingStyle::LastColumnOnlyAutoresizingStyle,
    );
    outline.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    outline.setAllowsMultipleSelection(false);
    outline.setAllowsEmptySelection(true);
    outline.setAllowsColumnReordering(false);
    outline.setAllowsColumnResizing(false);
    outline.setBackgroundColor(&NSColor::clearColor());
    outline.setStyle(NSTableViewStyle::Plain);
    outline.setBackgroundColor(&NSColor::clearColor());
    outline.setSelectionHighlightStyle(NSTableViewSelectionHighlightStyle::Regular);
    outline.setUsesAlternatingRowBackgroundColors(false);
    outline.setFocusRingType(NSFocusRingType::None);
    let _: () =
        msg_send![&outline, setAccessibilityLabel: &*NSString::from_str("자료 보관함 폴더 트리")];
    let mut nodes = vec![Node {
        key: NSString::from_str("folder:"),
        path: String::new(),
        document: None,
    }];
    if let Some(folders) = tree["folders"].as_array() {
        for f in folders {
            let p = strv(f, "path");
            if !p.is_empty() {
                nodes.push(Node {
                    key: NSString::from_str(&format!("folder:{p}")),
                    path: p,
                    document: None,
                });
            }
        }
    }
    if let Some(entries) = tree["entries"].as_array() {
        for d in entries {
            if d["path"].is_string() {
                nodes.push(Node {
                    key: NSString::from_str(&format!("doc:{}", strv(d, "id"))),
                    path: strv(d, "path"),
                    document: Some(d.clone()),
                });
            }
        }
    }
    nodes[1..].sort_by(|a, b| {
        b.folder()
            .cmp(&a.folder())
            .then_with(|| a.path.to_lowercase().cmp(&b.path.to_lowercase()))
    });
    let controller: Retained<DocumentTree> = msg_send![
        super(DocumentTree::alloc(mtm()).set_ivars(TreeState {
            nodes,
            root_path: tree["root_path"].as_str().map(PathBuf::from),
            outline: outline.clone(),
            restoring: Cell::new(true),
            context_document: RefCell::new(None),
            context_parent: RefCell::new(String::new()),
            active_document: RefCell::new(None),
            rename: RefCell::new(None),
            visible_paths: RefCell::new(None),
            analysis_busy: Cell::new(false)
        })),
        init
    ];
    outline.setDataSource(Some(ProtocolObject::from_ref(&*controller)));
    outline.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    outline.setTarget(Some(&controller));
    // Use the row's single-click action so folder icons and labels toggle too.
    // Selection changes stay separate for keyboard navigation and file opening.
    outline.setAction(Some(sel!(toggleFolder:)));
    outline.registerForDraggedTypes(&NSArray::from_retained_slice(&[
        NSString::from_str(DRAG_TYPE),
        NSString::from_str(FILE_URL_TYPE),
    ]));
    outline.setDraggingSourceOperationMask_forLocal(NSDragOperation::Move, true);
    outline.setDraggingSourceOperationMask_forLocal(NSDragOperation::None, false);
    let menu = NSMenu::new(mtm());
    menu.setAutoenablesItems(false);
    menu.setDelegate(Some(ProtocolObject::from_ref(&*controller)));
    outline.setMenu(Some(&menu));
    outline.reloadData();
    outline.expandItem(Some(&controller.ivars().nodes[0].key));
    let mut folders: Vec<_> = controller
        .ivars()
        .nodes
        .iter()
        .filter(|n| n.folder() && open_folders.contains(&n.path))
        .collect();
    folders.sort_by_key(|n| n.path.len());
    for f in folders {
        outline.expandItem(Some(&f.key));
    }
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), frame);
    scroll.setHasVerticalScroller(true);
    crate::scrollbar::style(&scroll);
    scroll.setHasHorizontalScroller(false);
    scroll.setAutohidesScrollers(true);
    scroll.setDrawsBackground(false);
    scroll.contentView().setDrawsBackground(false);
    scroll.setDocumentView(Some(&outline));
    outline.setFrameSize(NSSize::new(
        scroll.contentSize().width,
        outline.frame().size.height,
    ));
    outline.sizeLastColumnToFit();
    controller.select_document(selected_id);
    (scroll, controller)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn analysis_menu_gates_all_supported_files_consistently() {
        for kind in ["md", "pdf", "pptx", "png", "jpg", "jpeg", "heic"] {
            let mut document = json!({"kind":kind, "analysis_state":"unanalysed"});
            assert!(analysis_enabled(&document, false), "{kind}");
            assert!(!analysis_enabled(&document, true), "busy {kind}");
            document["analysis_state"] = json!("analysed");
            assert!(!analysis_enabled(&document, false), "current {kind}");
            assert_eq!(analysis_label(&document), "분석됨");
            document["analysis_state"] = json!("stale");
            assert!(analysis_enabled(&document, false), "stale {kind}");
            assert_eq!(analysis_label(&document), "정리 필요");
            assert!(reread_enabled(&document, false) && !reread_enabled(&document, true), "reread {kind}");
            document["missing"] = json!(true);
            assert!(!analysis_enabled(&document, false), "missing {kind}");
        }
        assert!(!analysis_enabled(&json!({"kind":"draft"}), false));
        assert!(!analysis_enabled(&json!({"kind":"ppt"}), false));
    }
}
