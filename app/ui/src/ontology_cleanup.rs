use super::*;

/// A scrollable list of explicitly opt-in choices. Closing the alert discards them.
unsafe fn choose(title: &str, message: &str, rows: &[Value], accept: &str) -> Option<Vec<String>> {
    let alert = NSAlert::new(mtm());
    alert.setMessageText(&NSString::from_str(title));
    alert.setInformativeText(&NSString::from_str(message));
    let mut choices = Vec::new();
    if !rows.is_empty() {
        let mut top = 0.;
        let list = view(rect(0., 0., 470., 1.));
        for row in rows {
            top -= 30.;
            let button = NSButton::initWithFrame(NSButton::alloc(mtm()), rect(2., top, 450., 28.));
            button.setButtonType(NSButtonType::Switch);
            button.setTitle(&NSString::from_str(&strv(row, "name")));
            button.setState(0);
            list.addSubview(&button);
            choices.push((strv(row, "id"), button));
            if let Some(reason) = row["reason"].as_str() {
                wrapped(&list, reason, 23., &mut top, 425., 12., false, 5);
            }
        }
        let height = -top;
        for v in list.subviews().iter() {
            let p = v.frame().origin;
            v.setFrameOrigin(NSPoint::new(p.x, p.y + height));
        }
        list.setFrameSize(NSSize::new(470., height));
        let scroll = NSScrollView::initWithFrame(
            NSScrollView::alloc(mtm()),
            rect(0., 0., 480., height.min(350.)),
        );
        scroll.setHasVerticalScroller(true);
        scroll.setDocumentView(Some(&list));
        list.scrollPoint(NSPoint::new(0., height));
        alert.setAccessoryView(Some(&scroll));
    }
    alert.addButtonWithTitle(&NSString::from_str("취소"));
    alert.addButtonWithTitle(&NSString::from_str(accept));
    if alert.runModal() != 1001 {
        return None;
    }
    Some(
        choices
            .into_iter()
            .filter(|(_, b)| b.state() == 1)
            .map(|(id, _)| id)
            .collect(),
    )
}

impl State {
    pub(super) unsafe fn start_cleanup(&mut self) {
        if self.cleanup_review.is_some() {
            return;
        }
        let Some(ws) = self.workspace.clone() else {
            return;
        };
        if !confirm_action("AI 삭제 후보를 검토할까요?", "현재 과목의 개념과 자료 근거를 살펴 삭제 후보를 제안합니다. 후보를 확인한 뒤 지울 개념을 선택할 수 있어요.", "검토 시작") {
            return;
        }
        let provider = self.provider.clone();
        let (send, recv) = mpsc::channel();
        self.cleanup_review = Some((ws.clone(), recv));
        self.update_ai_spinner();
        std::thread::spawn(move || {
            let _ = send.send(rpc(
                json!({"op":"ontology.cleanup.review","workspace_id":ws,"provider":provider}),
            ));
        });
        self.refresh_right();
        self.set_status("AI가 과목 전체 지식지도의 삭제 후보를 검토하고 있어요");
    }
    pub(super) unsafe fn poll_cleanup(&mut self) {
        let Some((workspace, rx)) = &self.cleanup_review else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("검토가 중단되었습니다.".into()),
        };
        let current = self.workspace.as_ref() == Some(workspace) && self.map.is_some();
        self.cleanup_review = None;
        self.update_ai_spinner();
        if !current {
            return;
        }
        self.refresh_right();
        self.set_status("삭제 후보 검토 종료");
        match result {
            Err(e) => self.fail(&e),
            Ok(r) => {
                let rows = r["candidates"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if rows.is_empty() {
                    alert("AI 삭제 후보 검토", "제안된 삭제 후보가 없습니다.");
                    return;
                }
                if let Some(ids) = choose(
                    "AI 삭제 후보",
                    "지울 후보를 선택하세요.",
                    rows,
                    "삭제 영향 확인",
                ) {
                    if !ids.is_empty() {
                        self.confirm_nodes(ids, r["version"].clone());
                    }
                }
            }
        }
    }
    pub(super) unsafe fn confirm_nodes(&mut self, ids: Vec<String>, version: Value) {
        let mut req = json!({"op":"ontology.cleanup.preview","workspace_id":self.workspace,"ids":ids,"version":version});
        let preview = match rpc(req.clone()) {
            Ok(v) => v,
            Err(e) => {
                self.fail(&e);
                return;
            }
        };
        let names = preview["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| strv(v, "name"))
            .collect::<Vec<_>>()
            .join(", ");
        let affected = preview["affected"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default();
        let hint = if affected.is_empty() {
            "연결이 모두 사라지는 이웃은 없습니다."
        } else {
            "아래 이웃은 연결이 모두 사라집니다. 함께 지울 이웃만 체크하세요."
        };
        let Some(extra) = choose(
            "삭제 영향 확인",
            &format!(
                "삭제 대상 · {names}\n\n{hint}\n승인하면 선택한 개념과 연결을 영구 삭제합니다."
            ),
            affected,
            "삭제 승인",
        ) else {
            return;
        };
        req["op"] = json!("ontology.cleanup.delete");
        req["version"] = preview["version"].clone();
        req["extra"] = json!(extra);
        match rpc(req) {
            Ok(r) => {
                self.reload_graph();
                self.refresh_right();
                self.set_status(&format!("개념 {}개를 삭제했어요", r["deleted"]));
            }
            Err(e) => self.fail(&e),
        }
    }
}
