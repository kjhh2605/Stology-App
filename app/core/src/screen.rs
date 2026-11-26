//! The screen the user last looked at, restored on the next launch.
//! App-wide layout lives in `ui/app.json`; each workspace's screen lives in
//! `ui/screens/<workspace id>.json` and is removed with the workspace.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn screen_path(root: &Path, workspace: &str) -> Result<PathBuf> {
    uuid::Uuid::parse_str(workspace).context("잘못된 워크스페이스 id")?;
    Ok(root.join("ui/screens").join(format!("{workspace}.json")))
}

// A missing or damaged file only means there is nothing to restore.
fn read(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

fn write(root: &Path, path: &Path, value: &Value) -> Result<()> {
    anyhow::ensure!(value.is_object(), "화면 상태는 객체여야 합니다");
    let parent = path.parent().context("잘못된 경로")?;
    crate::storage::ensure_no_symlink(root, parent)?;
    fs::create_dir_all(parent)?;
    crate::storage::atomic_write(path, &serde_json::to_vec_pretty(value)?, true)
}

pub fn handle(root: &Path, r: &Value) -> Result<Value> {
    let workspace = r["workspace_id"].as_str();
    match r["op"].as_str().unwrap_or("") {
        "screen.load" => {
            let screen = match workspace {
                Some(id) => read(&screen_path(root, id)?),
                None => Value::Null,
            };
            Ok(json!({"app": read(&root.join("ui/app.json")), "screen": screen}))
        }
        "screen.save" => {
            if !r["app"].is_null() {
                write(root, &root.join("ui/app.json"), &r["app"])?;
            }
            if let Some(id) = workspace {
                write(root, &screen_path(root, id)?, &r["screen"])?;
            }
            Ok(json!({"saved": true}))
        }
        op => anyhow::bail!("지원하지 않는 명령: {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WS: &str = "6f1c1f1e-4a4b-4c4d-8e8f-909192939495";

    #[test]
    fn saves_and_loads_app_and_workspace_screen() {
        let dir = tempfile::tempdir().unwrap();
        let empty = handle(dir.path(), &json!({"op":"screen.load","workspace_id":WS})).unwrap();
        assert_eq!(empty, json!({"app":null,"screen":null}));
        handle(dir.path(), &json!({"op":"screen.save","app":{"workspace":WS,"sidebar_open":[false,true]},
            "workspace_id":WS,"screen":{"document":{"id":"n1","kind":"md"},"map_open":true}})).unwrap();
        let loaded = handle(dir.path(), &json!({"op":"screen.load","workspace_id":WS})).unwrap();
        assert_eq!(loaded["app"]["sidebar_open"], json!([false, true]));
        assert_eq!(loaded["screen"]["document"]["id"], "n1");
        assert_eq!(loaded["screen"]["map_open"], true);
    }

    #[test]
    fn damaged_file_restores_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = screen_path(dir.path(), WS).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{broken").unwrap();
        fs::write(dir.path().join("ui/app.json"), b"[1]").unwrap();
        let loaded = handle(dir.path(), &json!({"op":"screen.load","workspace_id":WS})).unwrap();
        assert_eq!(loaded, json!({"app":null,"screen":null}));
    }

    #[test]
    fn rejects_paths_outside_screens() {
        let dir = tempfile::tempdir().unwrap();
        assert!(handle(dir.path(), &json!({"op":"screen.save","workspace_id":"../x","screen":{}})).is_err());
    }
}
