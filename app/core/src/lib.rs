#[path = "../../shared/diagram_svg.rs"]
pub mod diagram_svg;
pub mod chat;
mod antigravity;
pub mod distill;
pub mod knowledge;
pub mod ontology;
pub mod organize;
pub mod pdf_recovery;
mod generated_pdf;
pub mod providers;
pub mod screen;
pub mod skills;
pub mod storage;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub fn document_title(input: &str) -> &str {
    match input.trim() {
        "" => "제목 없음",
        title => title,
    }
}

pub fn data_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("STOLOGY_DATA_DIR") {
        return Ok(root.into());
    }
    let home = std::env::var_os("HOME").context("HOME을 찾을 수 없습니다")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Stology Local"))
}

pub fn dispatch(root: &Path, request: &Value) -> Result<Value> {
    let op = request
        .get("op")
        .and_then(Value::as_str)
        .context("op가 필요합니다")?;
    if op.starts_with("chat.") { return chat::handle(root, request); }
    if op.starts_with("screen.") { return screen::handle(root, request); }
    if op == "pdf.recovery.discard" {
        pdf_recovery::discard_recovery(root, request["id"].as_str().context("id가 필요합니다")?)?;
        return Ok(json!({"discarded":true}));
    }
    if op.starts_with("pdf.generated.") {
        return generated_pdf::handle(root, request);
    }
    // One material read on its own: the same run as 과목 정리, without aligning or connecting.
    if op == "analysis.begin" {
        let id = request
            .get("material_id")
            .and_then(Value::as_str)
            .context("material_id가 필요합니다")?;
        let mut start = json!({"op":"organize.start","workspace_id":request["workspace_id"],"materials":[id],"single":true,"expected_hash":request["expected_hash"]});
        if request["provider"].is_string() {
            start["provider"] = request["provider"].clone();
        }
        return organize::start(root, &start);
    }
    if op.starts_with("organize.") {
        return organize::handle(root, request);
    }
    if op.starts_with("distill.") {
        return distill::handle(root, request);
    }
    if op.starts_with("ontology.") {
        return ontology::handle(root, request);
    }
    if op.starts_with("ai.") {
        return skills::handle(root, request);
    }
    if op == "note.delete" || op == "material.delete" {
        let result = storage::handle(root, request)?;
        // The graph also reconciles missing sources on read after interruption.
        let cleanup = knowledge::handle(
            root,
            &json!({"op":"graph.remove_source","material_id":request["id"]}),
        );
        if let Err(e) = cleanup {
            let mut result = result;
            result["graph_cleanup_error"] = json!(e.to_string());
            return Ok(result);
        }
        return Ok(result);
    }
    if op.starts_with("provider.") || op.starts_with("analysis.") {
        return providers::handle(root, request);
    }
    if op == "extract" || op.starts_with("graph.") {
        return knowledge::handle(root, request);
    }
    if op == "app.info" {
        return Ok(
            json!({"version":env!("CARGO_PKG_VERSION"),"data_root":root,"distribution":"personal-test-ad-hoc","notarized":false}),
        );
    }
    if op.starts_with("workspace.")
        || op.starts_with("draft.")
        || op.starts_with("note.")
        || op.starts_with("folder.")
        || op.starts_with("material.")
        || op == "tree"
        || op == "source.get"
        || op == "source.snapshot"
    {
        return storage::handle(root, request);
    }
    bail!("지원하지 않는 명령: {op}")
}
