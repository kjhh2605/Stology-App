//! Run on macOS: cargo run --locked --example document-tree-smoke
#![allow(deprecated, dead_code)]
#[path = "../src/scrollbar.rs"]
#[allow(dead_code)]
mod scrollbar;
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::{json, Value};
use std::collections::HashSet;
#[path = "../src/font.rs"]
mod font;
#[path = "../src/document_tree.rs"]
mod document_tree;
#[path = "../src/theme.rs"]
mod theme;
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("main thread")
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
}
fn strv(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or("").to_owned()
}
struct DragState {
    source: Retained<NSOutlineView>,
    pasteboard: Retained<NSPasteboard>,
    location: std::cell::Cell<NSPoint>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[ivars=DragState]
    struct TestDraggingInfo;
    unsafe impl NSObjectProtocol for TestDraggingInfo {}
    impl TestDraggingInfo {
        #[unsafe(method(draggingSourceOperationMask))]
        fn mask(&self)->NSDragOperation { NSDragOperation::Copy | NSDragOperation::Move }
        #[unsafe(method(draggingLocation))]
        fn location(&self)->NSPoint { self.ivars().location.get() }
        #[unsafe(method_id(draggingSource))]
        fn source(&self)->Retained<AnyObject> { unsafe { Retained::cast_unchecked(self.ivars().source.clone()) } }
        #[unsafe(method_id(draggingPasteboard))]
        fn pasteboard(&self)->Retained<NSPasteboard> { self.ivars().pasteboard.clone() }
    }
);
unsafe fn node(outline: &NSOutlineView, wanted: &str) -> Retained<AnyObject> {
    (0..outline.numberOfRows())
        .filter_map(|r| outline.itemAtRow(r))
        .find(|item| {
            let s: Retained<NSString> = msg_send![&**item, description];
            s.to_string() == wanted
        })
        .unwrap_or_else(|| panic!("missing visible node {wanted}"))
}
unsafe fn document_cell(outline: &NSOutlineView, id: &str) -> Retained<NSTableCellView> {
    let item = node(outline, &format!("doc:{id}"));
    Retained::cast_unchecked(
        outline
            .viewAtColumn_row_makeIfNecessary(0, outline.rowForItem(Some(&item)), true)
            .unwrap(),
    )
}
fn main() {
    unsafe {
        let _app = NSApplication::sharedApplication(mtm());
        let tree = json!({"folders":[{"path":"folder"},{"path":"folder/nested"},{"path":"other"}],"entries":[
            {"id":"a","title":"Root note","path":"a.md","kind":"md"},
            {"id":"b","title":"Nested note","path":"folder/nested/b.md","kind":"md"},
            {"id":"c","title":"Missing note","path":"c.md","kind":"md","missing":true},
            {"id":"pdf","title":"A very long nested PDF document title that must truncate before its badge","path":"folder/nested/report.PDF","kind":"pdf","missing":true},
            {"id":"slides","workspace_id":"smoke","title":"Presentation","path":"slides.pptx","kind":"pptx"},
            {"id":"photo","title":"Photograph","path":"image.JPG","kind":"jpeg"},
            {"id":"heic","title":"Camera image","path":"camera.heic","kind":"heic"}
        ]});
        let (scroll, controller) =
            document_tree::build(&tree, Some("b"), &HashSet::new(), &Default::default(), rect(0., 0., 300., 500.));
        let outline: Retained<NSOutlineView> =
            Retained::cast_unchecked(scroll.documentView().unwrap());
        assert!(
            document_tree::drain().is_empty(),
            "restore must not enqueue actions"
        );
        let window = NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm()),
            rect(0., 0., 300., 500.),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        );
        window.setContentView(Some(&scroll));
        scroll.layoutSubtreeIfNeeded();
        for width in [300., 244., 360.] {
            window.setContentSize(NSSize::new(width, 500.));
            scroll.layoutSubtreeIfNeeded();
            let viewport_width = scroll.contentView().bounds().size.width;
            assert!(
                outline.frame().size.width <= viewport_width + 0.5,
                "tree widened beyond its viewport"
            );
            assert!(!scroll.hasHorizontalScroller());
            for i in 0..outline.numberOfRows() {
                if let Some(row) = outline.rowViewAtRow_makeIfNecessary(i, true) {
                    row.layoutSubtreeIfNeeded();
                    assert!(
                        row.frame().size.width <= viewport_width + 0.5,
                        "selected row right corner clipped"
                    );
                }
            }
        }
        assert_eq!(
            document_tree::file_extension(&json!({"path":"image.JPG","kind":"jpeg"})),
            "JPG"
        );
        assert_eq!(
            document_tree::file_extension(&json!({"path":"no-extension","kind":"pdf"})),
            "PDF"
        );
        assert_eq!(
            document_tree::file_extension(&json!({"path":"opaque.abc"})),
            "ABC"
        );
        assert_eq!(document_tree::file_extension(&json!({})), "FILE");
        assert_ne!(
            theme::file_type_symbol("PDF"),
            theme::file_type_symbol("PPTX")
        );
        assert_ne!(
            theme::file_type_symbol("PDF"),
            theme::file_type_symbol("PNG")
        );
        for ext in ["PDF", "PPTX", "PNG", "JPG", "JPEG", "HEIC", "ABC"] {
            assert!(
                theme::icon(theme::file_type_symbol(ext), 13.).is_some(),
                "{ext} SF Symbol missing"
            );
        }
        for width in [244., 300., 360.] {
            window.setContentSize(NSSize::new(width, 500.));
            scroll.layoutSubtreeIfNeeded();
            for (id, ext) in [
                ("pdf", "PDF"),
                ("slides", "PPTX"),
                ("photo", "JPG"),
                ("heic", "HEIC"),
            ] {
                let item = node(&outline, &format!("doc:{id}"));
                let row = outline.rowForItem(Some(&item));
                let cell: Retained<NSTableCellView> = Retained::cast_unchecked(
                    outline
                        .viewAtColumn_row_makeIfNecessary(0, row, true)
                        .unwrap(),
                );
                cell.layoutSubtreeIfNeeded();
                let label = cell.textField().unwrap();
                let badge = cell
                    .subviews()
                    .iter()
                    .find(|view| {
                        let is_field: bool =
                            msg_send![&***view, isKindOfClass:NSTextField::class()];
                        if !is_field {
                            return false;
                        }
                        let value: Retained<NSString> = msg_send![&***view, stringValue];
                        value.to_string() == ext
                    })
                    .expect("extension badge missing");
                assert!(
                    badge.frame().origin.x + badge.frame().size.width
                        <= cell.bounds().size.width + 0.5,
                    "{ext} badge clipped at width {width}"
                );
                assert_eq!(badge.frame().size.width, theme::FILE_TYPE_BADGE_WIDTH, "{ext} badge width differs");
                assert_eq!(badge.frame().size.height, theme::FILE_TYPE_BADGE_HEIGHT, "{ext} badge height differs");
                assert!(
                    label.frame().origin.x + label.frame().size.width
                        <= badge.frame().origin.x - 4. + 0.5,
                    "{ext} title overlaps badge"
                );
                assert!(cell.toolTip().is_none(), "document title has no native tooltip");
            }
        }
        let menu = NSMenu::new(mtm());
        for (key, expected) in [
            (Some("folder:folder/nested"), "folder/nested"),
            (Some("folder:"), ""),
            (None, ""),
        ] {
            let item = key.map(|key| node(&outline, key));
            controller.populate_context_menu(&menu, item.as_deref());
            assert_eq!(menu.numberOfItems(), 2);
            let note = menu.itemAtIndex(0).unwrap();
            assert_eq!(note.title().to_string(), "새 노트");
            assert!(note.isEnabled());
            let _: () = msg_send![&controller, newNote:std::ptr::null::<AnyObject>()];
            assert!(
                matches!(document_tree::drain().as_slice(),[document_tree::Event::NewNote(path)] if path==expected)
            );
            let create = menu.itemAtIndex(1).unwrap();
            assert_eq!(create.title().to_string(), "새 폴더…");
            assert!(create.isEnabled());
            let _: () = msg_send![&controller, newFolder:std::ptr::null::<AnyObject>()];
            assert!(
                matches!(document_tree::drain().as_slice(),[document_tree::Event::NewFolder(path)] if path==expected)
            );
            let has_rename = menu
                .itemArray()
                .iter()
                .any(|i| i.title().to_string() == "제목 수정…");
            assert!(!has_rename);
        }
        for id in ["a", "b", "c", "pdf", "slides", "photo", "heic"] {
            let item = node(&outline, &format!("doc:{id}"));
            controller.populate_context_menu(&menu, Some(&item));
            let rename = menu
                .itemArray()
                .iter()
                .find(|item| item.title().to_string() == "제목 수정…")
                .expect("rename menu item");
            let enabled = !matches!(id, "c" | "pdf");
            assert_eq!(rename.isEnabled(), enabled);
            if enabled {
                let _: () = msg_send![&controller, renameDocument:std::ptr::null::<AnyObject>()];
                assert!(
                    matches!(document_tree::drain().as_slice(), [document_tree::Event::Rename(doc)] if doc["id"] == id),
                    "rename uses its context target, not the open document"
                );
            }
        }
        controller.reveal_folder("folder/nested");
        assert!(
            document_tree::drain().is_empty(),
            "revealing new folder must not open a different document"
        );
        assert_eq!(
            outline.selectedRow(),
            outline.rowForItem(Some(&node(&outline, "folder:folder/nested")))
        );
        controller.select_document(Some("b"));
        let b = node(&outline, "doc:b");
        assert_eq!(outline.selectedRow(), outline.rowForItem(Some(&b)));
        assert!(
            outline.isItemExpanded(Some(&node(&outline, "folder:folder/nested"))),
            "selected ancestors revealed"
        );
        let a = node(&outline, "doc:a");
        outline.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(outline.rowForItem(Some(&a)) as usize),
            false,
        );
        assert!(
            matches!(document_tree::drain().as_slice(),[document_tree::Event::Open(v)] if v["id"]=="a")
        );
        controller.select_document(Some("a"));
        assert!(document_tree::drain().is_empty());
        let folder = node(&outline, "folder:folder");
        outline.collapseItem(Some(&folder));
        assert!(document_tree::drain()
            .iter()
            .any(|e| matches!(e,document_tree::Event::ToggleFolder(p,false) if p=="folder")));
        outline.expandItem(Some(&folder));
        assert!(document_tree::drain()
            .iter()
            .any(|e| matches!(e,document_tree::Event::ToggleFolder(p,true) if p=="folder")));
        let source = outline.dataSource().unwrap();
        // Keyboard focus may visit a folder; the current editor document still has a marker.
        outline.selectRowIndexes_byExtendingSelection(
            &NSIndexSet::indexSetWithIndex(outline.rowForItem(Some(&folder)) as usize),
            false,
        );
        assert!(document_tree::drain().is_empty());
        let active_label = source
            .outlineView_objectValueForTableColumn_byItem(&outline, None, Some(&a))
            .unwrap();
        let active_label: Retained<NSString> = Retained::cast_unchecked(active_label);
        assert_eq!(
            active_label.to_string(),
            "Root note · 미분석",
            "labels use SF Symbols, not text glyphs"
        );
        let delegate = outline.delegate().unwrap();
        let active_view = delegate
            .outlineView_viewForTableColumn_item(
                &outline,
                Some(&outline.tableColumns().objectAtIndex(0)),
                &a,
            )
            .unwrap();
        let active_cell: Retained<NSTableCellView> = Retained::cast_unchecked(active_view);
        assert!(active_cell.imageView().is_some(), "SF Symbol image missing");
        assert_eq!(
            active_cell.textField().unwrap().stringValue().to_string(),
            "Root note"
        );
        let hit: Option<Retained<NSView>> = msg_send![&active_cell, hitTest:NSPoint::new(35., 15.)];
        assert!(
            hit.is_none(),
            "decorative cell must not capture tree drag/clicks"
        );
        let active_row = delegate.outlineView_rowViewForItem(&outline, &a).unwrap();
        let is_active: bool = msg_send![&active_row, isDocumentActive];
        assert!(
            is_active,
            "current document highlight survives folder keyboard focus"
        );
        assert_eq!(
            active_row.interiorBackgroundStyle(),
            NSBackgroundStyle::Normal
        );
        active_cell.setBackgroundStyle(NSBackgroundStyle::Emphasized);
        assert_eq!(
            active_cell.backgroundStyle(),
            NSBackgroundStyle::Normal,
            "selected text must remain legible on pale glass"
        );
        assert_eq!(outline.style(), NSTableViewStyle::Plain);
        controller.select_document(Some("a"));
        let pb = NSPasteboard::pasteboardWithUniqueName();
        pb.declareTypes_owner(
            &NSArray::from_retained_slice(&[NSString::from_str("app.stology.local.document")]),
            None,
        );
        pb.setString_forType(
            &NSString::from_str("a"),
            &NSString::from_str("app.stology.local.document"),
        );
        let drag: Retained<TestDraggingInfo> = msg_send![
            super(TestDraggingInfo::alloc(mtm()).set_ivars(DragState {
                source: outline.clone(),
                pasteboard: pb.clone(),
                location: std::cell::Cell::new(NSPoint::new(0., 0.))
            })),
            init
        ];
        let info = &*(Retained::as_ptr(&drag).cast::<ProtocolObject<dyn NSDraggingInfo>>());
        assert_eq!(
            source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                &outline,
                info,
                Some(&folder),
                -1
            ),
            NSDragOperation::Move
        );
        assert!(source.outlineView_acceptDrop_item_childIndex(&outline, info, Some(&folder), -1));
        assert!(
            matches!(document_tree::drain().as_slice(),[document_tree::Event::Move(v,p)] if v["id"]=="a" && p=="folder")
        );
        let root = node(&outline, "folder:");
        assert_eq!(
            source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                &outline,
                info,
                Some(&root),
                -1
            ),
            NSDragOperation::None,
            "same parent rejected"
        );
        assert_eq!(
            source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                &outline,
                info,
                Some(&a),
                -1
            ),
            NSDragOperation::None,
            "file target rejected"
        );
        pb.setString_forType(
            &NSString::from_str("c"),
            &NSString::from_str("app.stology.local.document"),
        );
        assert_eq!(
            source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                &outline,
                info,
                Some(&folder),
                -1
            ),
            NSDragOperation::None,
            "missing source rejected"
        );
        let (_, foreign) =
            document_tree::build(&tree, None, &HashSet::new(), &Default::default(), rect(0., 0., 300., 500.));
        // A controller from another outline may know the same ID but must reject the foreign source.
        let foreign_source: &ProtocolObject<dyn NSOutlineViewDataSource> =
            ProtocolObject::from_ref(&*foreign);
        assert_eq!(
            foreign_source
                .outlineView_validateDrop_proposedItem_proposedChildIndex(&outline, info, None, -1),
            NSDragOperation::None
        );
        // Finder-style file URLs: Unicode/space decoding, multiple files,
        // pointer destination independent of AppKit's proposed insertion item.
        let files = tempfile::tempdir().unwrap();
        let paths: Vec<_> = ["한글 자료.md", "second.pdf"]
            .iter()
            .map(|name| {
                let path = files.path().join(name);
                std::fs::write(&path, "original").unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        pb.clearContents();
        let writers: Vec<_> = paths
            .iter()
            .map(|path| {
                ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(path)))
            })
            .collect();
        assert!(pb.writeObjects(&NSArray::from_retained_slice(&writers)));
        for (target, expected) in [
            (Some("folder:folder/nested"), "folder/nested"),
            (Some("doc:b"), "folder/nested"),
            (Some("folder:"), ""),
            (None, ""),
        ] {
            let point = if let Some(target) = target {
                let frame = outline.rectOfRow(outline.rowForItem(Some(&node(&outline, target))));
                NSPoint::new(60., frame.origin.y + frame.size.height / 2.)
            } else {
                NSPoint::new(60., outline.bounds().size.height + 20.)
            };
            drag.ivars()
                .location
                .set(outline.convertPoint_toView(point, None));
            assert_eq!(
                source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                    &outline,
                    info,
                    Some(&root),
                    0
                ),
                NSDragOperation::Copy
            );
            assert!(source.outlineView_acceptDrop_item_childIndex(&outline, info, Some(&root), -1));
            let events = document_tree::drain();
            assert!(
                matches!(events.as_slice(),
                [document_tree::Event::CopyFiles(actual, folder)] if actual.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect::<Vec<_>>() == paths.iter().map(|p| std::fs::canonicalize(p).unwrap()).collect::<Vec<_>>() && folder == expected),
                "target {target:?}: {events:?}, expected paths {paths:?}, folder {expected}"
            );
        }
        // Directories and non-file URLs are not accepted as external files.
        for url in [
            NSURL::fileURLWithPath(&NSString::from_str(&files.path().to_string_lossy())),
            NSURL::URLWithString(&NSString::from_str("https://example.com/file.md")).unwrap(),
        ] {
            pb.clearContents();
            let item = NSPasteboardItem::new();
            item.setString_forType(
                &url.absoluteString().unwrap(),
                &NSString::from_str("public.file-url"),
            );
            pb.writeObjects(&NSArray::from_retained_slice(&[
                ProtocolObject::from_retained(item),
            ]));
            assert_eq!(
                source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                    &outline,
                    info,
                    Some(&folder),
                    -1
                ),
                NSDragOperation::None
            );
        }
        pb.declareTypes_owner(
            &NSArray::from_retained_slice(&[NSString::from_str("app.stology.local.document")]),
            None,
        );
        // Rename an unselected file in place, preserving the current document.
        let slides = &tree["entries"][4];
        controller.begin_rename(slides, None, None);
        assert_eq!(controller.renaming_workspace().as_deref(), Some("smoke"));
        let rename_cell = document_cell(&outline, "slides");
        let field = rename_cell.textField().unwrap();
        assert!(field.isEditable() && field.isSelectable());
        assert_eq!(field.stringValue().to_string(), "Presentation");
        assert_eq!(outline.selectedRow(), outline.rowForItem(Some(&a)));
        assert!(
            document_tree::drain().is_empty(),
            "begin rename must not open its target"
        );
        let editor: Retained<NSTextView> =
            Retained::cast_unchecked(field.currentEditor().expect("native filename editor"));
        assert_eq!(
            NSTextInputClient::selectedRange(&*editor),
            NSRange::new(0, "Presentation".len())
        );
        let cell_frame = rename_cell.frame();
        let field_frame = field.frame();
        let hit: Option<Retained<NSView>> = msg_send![&rename_cell, hitTest:NSPoint::new(cell_frame.origin.x + field_frame.origin.x + 5., cell_frame.origin.y + field_frame.origin.y + 5.)];
        assert!(
            hit.is_some(),
            "editing filename must accept native pointer input"
        );
        assert!(
            source
                .outlineView_pasteboardWriterForItem(&outline, &a)
                .is_none(),
            "rename prevents a new drag"
        );
        pb.setString_forType(
            &NSString::from_str("a"),
            &NSString::from_str("app.stology.local.document"),
        );
        assert_eq!(
            source.outlineView_validateDrop_proposedItem_proposedChildIndex(
                &outline,
                info,
                Some(&folder),
                -1
            ),
            NSDragOperation::None,
            "rename prevents a drop"
        );

        // The native field editor owns Korean composition: Return/Escape must
        // reach the input method and cannot commit/cancel the filename yet.
        let editing_delegate = field.delegate().unwrap();
        editor.setMarkedText_selectedRange_replacementRange(
            &NSString::from_str("한"),
            NSRange::new(1, 0),
            NSRange::new(0, "Presentation".len()),
        );
        assert!(NSTextInputClient::hasMarkedText(&*editor));
        for command in [sel!(insertNewline:), sel!(cancelOperation:)] {
            assert!(
                !editing_delegate.control_textView_doCommandBySelector(&field, &editor, command)
            );
            assert!(
                document_tree::drain().is_empty(),
                "composition cannot submit rename"
            );
            assert!(field.isEditable());
        }
        NSTextInputClient::unmarkText(&*editor);
        editor.setString(&NSString::from_str("발표 자료"));
        assert!(editing_delegate.control_textView_doCommandBySelector(
            &field,
            &editor,
            sel!(insertNewline:)
        ));
        assert!(
            matches!(document_tree::drain().as_slice(), [document_tree::Event::CommitRename(doc, title)] if doc["id"] == "slides" && doc["path"] == "slides.pptx" && title == "발표 자료")
        );
        assert!(!field.isEditable());
        assert_eq!(controller.renaming_workspace(), None);
        window.makeFirstResponder(Some(&outline));
        assert!(
            document_tree::drain().is_empty(),
            "Return followed by blur commits only once"
        );

        // A failed request keeps attempted text editable, with an inline error;
        // Escape restores the original filename and emits no persistence intent.
        controller.begin_rename(
            slides,
            Some("existing name"),
            Some("같은 이름의 파일이 있습니다"),
        );
        assert!(field.toolTip().is_none());
        let editor: Retained<NSTextView> = Retained::cast_unchecked(field.currentEditor().unwrap());
        assert_eq!(editor.string().to_string(), "existing name");
        editor.setString(&NSString::from_str("discard this"));
        assert!(field
            .delegate()
            .unwrap()
            .control_textView_doCommandBySelector(&field, &editor, sel!(cancelOperation:)));
        assert_eq!(field.stringValue().to_string(), "Presentation");
        assert!(!field.isEditable());
        assert!(document_tree::drain().is_empty());

        controller.begin_rename(slides, None, None);
        let editor = field.currentEditor().unwrap();
        editor.setString(&NSString::from_str("Blur saved title"));
        assert!(window.makeFirstResponder(Some(&outline)));
        assert!(
            matches!(document_tree::drain().as_slice(), [document_tree::Event::CommitRename(doc, title)] if doc["id"] == "slides" && title == "Blur saved title")
        );
        assert!(!field.isEditable());
        assert!(document_tree::drain().is_empty());

        controller.begin_rename(slides, None, None);
        let editor: Retained<NSTextView> = Retained::cast_unchecked(field.currentEditor().unwrap());
        assert!(field
            .delegate()
            .unwrap()
            .control_textView_doCommandBySelector(&field, &editor, sel!(insertNewline:)));
        assert!(
            document_tree::drain().is_empty(),
            "unchanged filename is a no-op"
        );
        controller.begin_rename(&tree["entries"][2], None, None);
        assert!(
            !document_cell(&outline, "c")
                .textField()
                .unwrap()
                .isEditable(),
            "missing file cannot be renamed"
        );
        controller.select_document(None);
        assert_eq!(outline.selectedRow(), -1);
        assert!(document_tree::drain().is_empty());
        controller.select_document(Some("b"));
        assert_eq!(controller.filter("  REPORT.pdf  ", &HashSet::new()), 1);
        assert_eq!(
            outline.numberOfRows(),
            4,
            "matching file and ancestors only"
        );
        node(&outline, "doc:pdf");
        assert_eq!(
            outline.selectedRow(),
            -1,
            "hidden current file is not a different selection"
        );
        assert_eq!(controller.filter("no-such-filename", &HashSet::new()), 0);
        assert_eq!(outline.numberOfRows(), 1, "empty search keeps root only");
        assert_eq!(controller.filter("NESTED NOTE", &HashSet::new()), 1);
        let nested = node(&outline, "doc:b");
        assert_eq!(outline.selectedRow(), outline.rowForItem(Some(&nested)));
        assert_eq!(controller.filter("", &HashSet::new()), 7);
        node(&outline, "doc:slides");
        node(&outline, "doc:b");
        assert!(
            document_tree::drain().is_empty(),
            "filter must not open files or persist temporary expansion"
        );
        // The analysis mark sits in the gutter left of its cell, which clips
        // the mark's own tooltip away; the row that spans the gutter must
        // receive the pointer there, since it carries the tooltip.
        let marked = json!({"folders":[{"path":"folder"}],"entries":[
            {"id":"stale","title":"Stale note","path":"folder/stale.md","kind":"md","analysis_state":"stale"}
        ]});
        let mut open = HashSet::new();
        open.insert("folder".to_owned());
        let (marked_scroll, _marked_controller) =
            document_tree::build(&marked, None, &open, &Default::default(), rect(0., 0., 300., 500.));
        window.setContentView(Some(&marked_scroll));
        marked_scroll.layoutSubtreeIfNeeded();
        let marked_outline: Retained<NSOutlineView> =
            Retained::cast_unchecked(marked_scroll.documentView().unwrap());
        let cell = document_cell(&marked_outline, "stale");
        let dot = cell
            .subviews()
            .iter()
            .find(|view| view.toolTip().is_some())
            .expect("analysis mark missing")
            .clone();
        assert_eq!(dot.toolTip().unwrap().to_string(), "문서가 바뀌어 다시 분석이 필요해요");
        let centre = dot.frame();
        let centre = NSPoint::new(centre.origin.x + 4., centre.origin.y + 4.);
        let frame_view = window.contentView().unwrap().superview().unwrap();
        let hit = frame_view.hitTest(cell.convertPoint_toView(centre, None));
        let row = cell.superview().unwrap();
        assert!(
            hit.is_some_and(|hit| std::ptr::eq(&*hit, &*row)),
            "pointer over analysis mark does not reach the row carrying its tooltip"
        );
        window.setContentView(None);
        let _: () = msg_send![&pb, releaseGlobally];
        println!("document-tree-smoke: hierarchy, badges, context menus, selection, external file copy destinations/URLs, move validation, inline rename target/commit/cancel/blur/retry/IME, analysis mark tooltip hit passed");
    }
}
