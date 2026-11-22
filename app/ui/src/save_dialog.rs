//! The first-save sheet keeps the title and expandable destination tree visible.
use crate::{alert, input, mtm, rect, rpc, strv, text_label, theme, view};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    sel, DefinedClass, MainThreadOnly,
};
use objc2_app_kit::*;
use objc2_foundation::*;
use serde_json::{json, Value};
use std::cell::RefCell;
struct FolderState {
    workspace: String,
    nodes: RefCell<Vec<Retained<NSString>>>,
    outline: Retained<NSOutlineView>,
}
unsafe fn path(item: &AnyObject) -> String {
    let s: Retained<NSString> = msg_send![item, description];
    s.to_string()
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}
define_class!(
    #[unsafe(super(NSObject))]
    #[name="StologySaveFolderTree"]
    #[thread_kind=MainThreadOnly]
    #[ivars=FolderState]
    struct FolderTree;
    unsafe impl NSObjectProtocol for FolderTree{}
    unsafe impl NSControlTextEditingDelegate for FolderTree {}
    unsafe impl NSOutlineViewDataSource for FolderTree{
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        unsafe fn count(&self,_outline:&NSOutlineView,item:Option<&AnyObject>)->isize{self.children(item).len()as isize}
        #[unsafe(method_id(outlineView:child:ofItem:))]
        unsafe fn child(&self,_outline:&NSOutlineView,index:isize,item:Option<&AnyObject>)->Retained<AnyObject>{Retained::cast_unchecked(self.children(item)[index as usize].clone())}
        #[unsafe(method(outlineView:isItemExpandable:))]
        unsafe fn expandable(&self,_outline:&NSOutlineView,item:&AnyObject)->bool{!self.children(Some(item)).is_empty()}
        #[unsafe(method_id(outlineView:objectValueForTableColumn:byItem:))]
        unsafe fn display(&self,_outline:&NSOutlineView,_column:Option<&NSTableColumn>,item:Option<&AnyObject>)->Option<Retained<AnyObject>>{let p=item.map(|i|path(i)).unwrap_or_default();let label=if p.is_empty(){"과목 루트"}else{p.rsplit('/').next().unwrap_or(&p)};Some(Retained::cast_unchecked(NSString::from_str(label)))}
    }
    unsafe impl NSOutlineViewDelegate for FolderTree {
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        unsafe fn cell_view(&self, outline: &NSOutlineView, column: Option<&NSTableColumn>, item: &AnyObject) -> Option<Retained<NSView>> {
            let p = path(item);
            let title = if p.is_empty() { "과목 루트" } else { p.rsplit('/').next().unwrap_or(&p) };
            let width = column.map(|column| column.width()).unwrap_or(420.);
            let cell = NSTableCellView::initWithFrame(NSTableCellView::alloc(mtm()), rect(0., 0., width, 28.));
            let label = NSTextField::labelWithString(&NSString::from_str(title), mtm());
            label.setFont(Some(&crate::font::regular(13.)));
            label.setFrame(rect(26., 4., (width - 30.).max(0.), 20.));
            label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            cell.addSubview(&label);
            cell.setTextField(Some(&label));
            let symbol = if outline.isItemExpanded(Some(item)) { "folder-open" } else { "folder" };
            if let Some(icon) = theme::icon_view(symbol, rect(2., 5., 18., 18.)) {
                cell.addSubview(&icon);
                cell.setImageView(Some(&icon));
            }
            let _: () = msg_send![&cell, setAccessibilityLabel: &*NSString::from_str(title)];
            Some(cell.into_super())
        }
        #[unsafe(method(outlineViewItemDidExpand:))]
        fn expanded(&self, notification: &NSNotification) { unsafe { self.refresh_icon(notification); } }
        #[unsafe(method(outlineViewItemDidCollapse:))]
        fn collapsed(&self, notification: &NSNotification) { unsafe { self.refresh_icon(notification); } }
    }
    impl FolderTree{
        #[unsafe(method(addFolder:))]
        fn add_folder(&self,_sender:Option<&AnyObject>){unsafe{
            let destination=self.selected();let Some(name)=input("새 폴더",&format!("{} 안에 폴더를 만듭니다.",if destination.is_empty(){"과목 루트"}else{&destination}),"", "만들기")else{return;};
            match rpc(json!({"op":"folder.create","workspace_id":self.ivars().workspace,"parent":destination,"name":name})){Ok(f)=>{let p=strv(&f,"path");let node=NSString::from_str(&p);{let mut nodes=self.ivars().nodes.borrow_mut();nodes.push(node.clone());nodes.sort_by_key(|n|n.to_string());}self.ivars().outline.reloadData();let root=self.ivars().nodes.borrow()[0].clone();self.ivars().outline.expandItem_expandChildren(Some(&root),true);let row=self.ivars().outline.rowForItem(Some(&node));if row>=0{self.ivars().outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(row as usize),false);}},Err(e)=>alert("폴더를 만들 수 없습니다",&e)}
        }}
    }
);
impl FolderTree {
    unsafe fn refresh_icon(&self, notification: &NSNotification) {
        if let Some(info) = notification.userInfo() {
            let item: Option<Retained<AnyObject>> =
                msg_send![&info, objectForKey: &*NSString::from_str("NSObject")];
            if let Some(item) = item {
                self.ivars().outline.reloadItem(Some(&item));
            }
        }
    }
    unsafe fn children(&self, item: Option<&AnyObject>) -> Vec<Retained<NSString>> {
        let nodes = self.ivars().nodes.borrow();
        match item {
            None => nodes.iter().filter(|n| n.is_empty()).cloned().collect(),
            Some(i) => {
                let p = path(i);
                nodes
                    .iter()
                    .filter(|n| !n.is_empty() && parent(&n.to_string()) == p)
                    .cloned()
                    .collect()
            }
        }
    }
    unsafe fn selected(&self) -> String {
        self.ivars()
            .outline
            .itemAtRow(self.ivars().outline.selectedRow())
            .map(|i| path(&i))
            .unwrap_or_default()
    }
}
pub unsafe fn choose(
    workspace: &str,
    suggested: &str,
    folders: &Value,
) -> Option<(String, String)> {
    let a = NSAlert::new(mtm());
    a.setMessageText(&NSString::from_str("노트 저장"));
    a.setInformativeText(&NSString::from_str(
        "제목과 보관할 폴더를 선택하세요. 같은 이름은 덮어쓰지 않습니다.",
    ));
    let container = view(rect(0., 0., 460., 350.));
    container.addSubview(&text_label("제목", rect(0., 325., 450., 20.), 12., true));
    let title = NSTextField::initWithFrame(NSTextField::alloc(mtm()), rect(0., 292., 460., 28.));
    title.setFont(Some(&crate::font::regular(13.)));
    title.setStringValue(&NSString::from_str(suggested));
    container.addSubview(&title);
    container.addSubview(&text_label(
        "보관 폴더",
        rect(0., 259., 450., 20.),
        12.,
        true,
    ));
    let outline =
        NSOutlineView::initWithFrame(NSOutlineView::alloc(mtm()), rect(0., 0., 440., 208.));
    let column = NSTableColumn::initWithIdentifier(
        NSTableColumn::alloc(mtm()),
        &NSString::from_str("folder"),
    );
    column.setWidth(420.);
    outline.addTableColumn(&column);
    outline.setOutlineTableColumn(Some(&column));
    outline.setHeaderView(None);
    outline.setRowHeight(28.);
    outline.setAllowsMultipleSelection(false);
    outline.setAllowsEmptySelection(false);
    outline.setIndentationPerLevel(18.);
    outline.setAutoresizesOutlineColumn(false);
    let mut nodes = vec![NSString::from_str("")];
    if let Some(fs) = folders["folders"].as_array() {
        for f in fs {
            nodes.push(NSString::from_str(&strv(f, "path")));
        }
    }
    nodes.sort_by_key(|n| n.to_string());
    let target: Retained<FolderTree> = msg_send![
        super(FolderTree::alloc(mtm()).set_ivars(FolderState {
            workspace: workspace.into(),
            nodes: RefCell::new(nodes),
            outline: outline.clone()
        })),
        init
    ];
    outline.setDataSource(Some(ProtocolObject::from_ref(&*target)));
    outline.setDelegate(Some(ProtocolObject::from_ref(&*target)));
    outline.reloadData();
    let root = target.ivars().nodes.borrow()[0].clone();
    outline.expandItem(Some(&root));
    outline.selectRowIndexes_byExtendingSelection(&NSIndexSet::indexSetWithIndex(0), false);
    let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm()), rect(0., 47., 460., 208.));
    scroll.setHasVerticalScroller(true);
    crate::scrollbar::style(&scroll);
    scroll.setBorderType(NSBorderType::BezelBorder);
    scroll.setDocumentView(Some(&outline));
    container.addSubview(&scroll);
    let new = NSButton::buttonWithTitle_target_action(
        &NSString::from_str("＋ 새 폴더"),
        Some(&target),
        Some(sel!(addFolder:)),
        mtm(),
    );
    new.setFrame(rect(0., 5., 120., 30.));
    new.setFont(Some(&crate::font::regular(12.)));
    container.addSubview(&new);
    a.setAccessoryView(Some(&container));
    a.addButtonWithTitle(&NSString::from_str("저장"));
    a.addButtonWithTitle(&NSString::from_str("취소"));
    let result = if a.runModal() == 1000 {
        Some((title.stringValue().to_string(), target.selected()))
    } else {
        None
    };
    outline.setDataSource(None);
    outline.setDelegate(None);
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn folder_parent_keeps_hierarchy() {
        assert_eq!(parent("week1"), "");
        assert_eq!(parent("week1/images"), "week1");
        assert_eq!(parent("week1/images/png"), "week1/images");
    }
}
