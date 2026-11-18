use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::PathBuf;
fn main() {
    if let Err(error) = run() {
        println!("{}", json!({"ok":false,"error":format!("{error:#}")}));
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .iter()
        .position(|s| s == "--root")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(stology_core::data_root)?;
    std::fs::create_dir_all(&root)?;
    if args.iter().any(|s| s == "--chat-stream") { return stology_core::chat::stream(&root); }
    if let Some(i) = args.iter().position(|s| s == "--worker") {
        return stology_core::providers::run_worker(
            &root,
            args.get(i + 1).context("job id required")?,
        );
    }
    let mut input = String::new();
    std::io::stdin()
        .take(32 * 1024 * 1024 + 1)
        .read_to_string(&mut input)?;
    anyhow::ensure!(input.len() <= 32 * 1024 * 1024, "요청이 너무 큽니다");
    let request: Value = serde_json::from_str(&input).context("잘못된 JSON 요청")?;
    let result = stology_core::dispatch(&root, &request)?;
    writeln!(std::io::stdout(), "{}", json!({"ok":true,"result":result}))?;
    Ok(())
}
