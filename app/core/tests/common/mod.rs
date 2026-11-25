//! Shared by the tests that run `stology-core` against a fake Codex CLI.
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Stdio},
};

/// A data root with `fake_codex` installed as the Codex CLI and one workspace made.
pub fn fixture(fake_codex: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("runtime/bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("codex"), fake_codex).unwrap();
    fs::set_permissions(bin.join("codex"), fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(dir.path().join("provider.json"), r#"{"provider":"codex"}"#).unwrap();
    let w = call(dir.path(), json!({"op":"workspace.create","name":"검증 과목"}))["id"].as_str().unwrap().to_owned();
    (dir, w)
}
pub fn request(root: &Path, request: Value) -> Value {
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", root.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    p.stdin.take().unwrap().write_all(request.to_string().as_bytes()).unwrap();
    serde_json::from_slice(&p.wait_with_output().unwrap().stdout).unwrap()
}
pub fn call(root: &Path, r: Value) -> Value {
    let v = request(root, r.clone());
    assert_eq!(v["ok"], true, "{r} → {v}");
    v["result"].clone()
}
/// What the fake CLI recorded for each call it received.
pub fn calls(root: &Path) -> Vec<Value> {
    fs::read_to_string(root.join("fake-calls.jsonl")).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}
