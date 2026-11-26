//! Keeps the screen the user last looked at across launches: the workspace,
//! each workspace's document/map/position, and the sidebar layout. The core
//! stores the JSON (`screen.save`/`screen.load`); documents are reopened by id.
use super::*;

// Only what reopens a document; bodies are kept by drafts and files.
fn document_ref(d: &Value) -> Value {
    let mut out = json!({});
    for key in ["id", "kind", "title", "absolute_path"] {
        if !d[key].is_null() {
            out[key] = d[key].clone();
        }
    }
    out
}

fn point(v: &Value) -> Option<NSPoint> {
    Some(NSPoint::new(v[0].as_f64()?, v[1].as_f64()?))
}

impl WorkspaceScreen {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "blank_title": self.blank_title,
            "file_query": self.file_query,
            // A distill suggestion may be accepted or dismissed elsewhere, so
            // it is not reopened after a restart.
            "document": self.document.as_ref().filter(|d| d["kind"] != "suggestion").map(document_ref),
            "evidence": self.evidence,
            "selection": [self.selection.location, self.selection.length],
            "scroll": [self.scroll.x, self.scroll.y],
            "map_open": self.map_open,
            "folders": self.folders,
            "pdf_page": self.pdf_page,
        })
    }
    pub(crate) fn from_json(v: &Value) -> Option<Self> {
        if !v.is_object() {
            return None;
        }
        let pair = |key: &str| (v[key][0].as_u64().unwrap_or(0) as usize, v[key][1].as_u64().unwrap_or(0) as usize);
        let (location, length) = pair("selection");
        Some(WorkspaceScreen {
            blank_title: v["blank_title"].as_str().unwrap_or("제목 없음").to_owned(),
            file_query: strv(v, "file_query"),
            document: v["document"]["id"].is_string().then(|| v["document"].clone()),
            evidence: v["evidence"]["material_id"].is_string().then(|| v["evidence"].clone()),
            selection: NSRange::new(location, length),
            scroll: point(&v["scroll"]).unwrap_or(NSPoint::new(0., 0.)),
            map_open: v["map_open"].as_bool().unwrap_or(false),
            folders: v["folders"].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_owned)).collect(),
            pdf_page: v["pdf_page"].as_u64().map(|p| p as usize),
        })
    }
}

impl State {
    fn app_state(&self) -> Value {
        json!({
            "workspace": self.workspace,
            "sidebar_open": self.sidebar_open,
            "left_width": self.left_width,
            "right_width": self.right_width,
            "shortcuts_open": self.shortcuts.as_ref().is_some_and(|p| !p.isHidden()),
        })
    }

    /// What decides the restored screen; positions are saved on switch and quit.
    fn screen_signature(&self) -> String {
        let doc = self.document.as_ref().map(|d| strv(d, "id")).unwrap_or_default();
        let evidence = self.current_evidence.as_ref().filter(|_| self.snapshot).map(|e| e.to_string()).unwrap_or_default();
        format!("{}|{doc}|{evidence}|{}|{}|{:?}", self.app_state(), self.map.is_some(), self.blank_title, self.open_folders)
    }

    /// Saves the current workspace's screen and the app layout.
    pub(crate) unsafe fn save_screen(&mut self) {
        let mut request = json!({"op":"screen.save","app":self.app_state()});
        if let Some((id, screen)) = self.current_screen() {
            let mut saved = screen.to_json();
            saved["map"] = if self.map.is_some() { self.map_preference() } else { self.map_preferences.get(&id).cloned().unwrap_or(Value::Null) };
            if let Some(p) = self.chat.position_for(&id) {
                saved["chat_scroll"] = json!([p.x, p.y]);
            }
            request["workspace_id"] = json!(id);
            request["screen"] = saved;
        }
        self.saved_signature = self.screen_signature();
        // Failing to remember the screen must not block editing or quitting.
        let _ = rpc(request);
    }

    pub(crate) unsafe fn save_screen_if_changed(&mut self) {
        let moving = self.sidebar_motion.is_some() || self.left_resizer.is_dragging() || self.right_resizer.is_dragging();
        if !moving && self.screen_signature() != self.saved_signature {
            self.save_screen();
        }
    }

    /// The workspace's screen saved by an earlier launch, if any.
    pub(crate) unsafe fn load_saved_screen(&mut self, id: &str) -> Option<WorkspaceScreen> {
        let saved = rpc(json!({"op":"screen.load","workspace_id":id})).ok()?["screen"].take();
        if saved["map"].is_object() && !self.map_preferences.contains_key(id) {
            self.map_preferences.insert(id.to_owned(), saved["map"].clone());
        }
        if let Some(p) = point(&saved["chat_scroll"]) {
            self.chat.seed_position(id, p);
        }
        WorkspaceScreen::from_json(&saved)
    }

    /// Reopens the last workspace, sidebar layout and screen at launch.
    pub(crate) unsafe fn restore_launch(&mut self) {
        let app = rpc(json!({"op":"screen.load"})).map(|v| v["app"].clone()).unwrap_or(Value::Null);
        if let Some(id) = app["workspace"].as_str() {
            let exists = rpc(json!({"op":"workspace.list"})).ok().is_some_and(|ws| {
                ws["workspaces"].as_array().is_some_and(|a| a.iter().any(|w| w["id"] == id))
            });
            if exists {
                self.workspace = Some(id.to_owned());
            }
        }
        if let Some(width) = app["left_width"].as_f64() {
            self.left_width = width.max(sidebar_layout::MIN_LEFT_WIDTH);
        }
        if let Some(width) = app["right_width"].as_f64() {
            self.right_width = width.max(sidebar_layout::MIN_RIGHT_WIDTH);
        }
        for side in 0..2 {
            if let Some(open) = app["sidebar_open"][side].as_bool() {
                self.sidebar_open[side] = open;
            }
            sidebar_reveal::set_open(&self.sidebar_handles[side], self.sidebar_open[side]);
        }
        self.sidebar_openness = self.sidebar_open.map(|open| if open { 1. } else { 0. });
        if let Some(panel) = &self.shortcuts {
            panel.setHidden(!app["shortcuts_open"].as_bool().unwrap_or(false));
        }
        self.layout_frames(self.root.bounds().size);
        self.finish_sidebar_motion();
        self.restore_screen();
        self.refresh_left();
        self.refresh_right();
        self.update_header();
        // Without this the window focuses the title field and selects it.
        if self.map.is_none() {
            self._window.makeFirstResponder(Some(&self.text));
        }
        self.saved_signature = self.screen_signature();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_round_trips_without_bodies_or_suggestions() {
        let screen = WorkspaceScreen {
            blank_title: "제목 없음".into(),
            file_query: "선형".into(),
            document: Some(json!({"id":"n1","kind":"md","title":"노트","body":"본문","version":3})),
            evidence: None,
            selection: NSRange::new(4, 2),
            scroll: NSPoint::new(0., 120.),
            map_open: true,
            folders: ["강의".to_owned()].into_iter().collect(),
            pdf_page: Some(7),
        };
        let v = screen.to_json();
        assert_eq!(v["document"], json!({"id":"n1","kind":"md","title":"노트"}));
        let back = WorkspaceScreen::from_json(&v).unwrap();
        assert_eq!((back.selection.location, back.selection.length), (4, 2));
        assert_eq!(back.scroll.y, 120.);
        assert!(back.map_open && back.folders.contains("강의"));
        assert_eq!(back.pdf_page, Some(7));
        assert_eq!(back.file_query, "선형");

        let suggestion = WorkspaceScreen { document: Some(json!({"id":"s1","kind":"suggestion"})), ..back };
        assert!(WorkspaceScreen::from_json(&suggestion.to_json()).unwrap().document.is_none());
        assert!(WorkspaceScreen::from_json(&Value::Null).is_none());
    }
}
