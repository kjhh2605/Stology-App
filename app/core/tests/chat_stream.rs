use serde_json::{json, Value};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
};
fn fixture() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("runtime/bin");
    fs::create_dir_all(&bin).unwrap();
    let p = bin.join("codex");
    fs::write(&p,r##"#!/usr/bin/env python3
import sys,json
if 'login' in sys.argv:
 print('Logged in using ChatGPT');sys.exit(0)
for line in sys.stdin:
 v=json.loads(line);m=v.get('method');i=v.get('id')
 if m=='initialize':r={}
 elif m=='model/list':r={'data':[{'model':'gpt-6.1-terra','displayName':'GPT-6.1 Terra','isDefault':True,'defaultReasoningEffort':'medium','supportedReasoningEfforts':[{'reasoningEffort':'medium'},{'reasoningEffort':'high'}]}]}
 elif m=='config/read':r={'config':{'mcp_servers':{}}}
 elif m in ['thread/start','thread/resume']:
  assert v['params']['model']=='gpt-6.1-terra'
  if v['params'].get('threadId')=='missing':
   print(json.dumps({'id':i,'error':{'message':'thread not found'}}),flush=True);continue
  r={'thread':{'id':'fixture-session'}}
 elif m=='turn/start':
  assert v['params']['model']=='gpt-6.1-terra'
  assert v['params']['effort']=='high'
  assert [x['type'] for x in v['params']['input']]==['image','text']
  assert v['params']['input'][0]['url']=='data:image/png;base64,iVBORw0KGgo='
  print(json.dumps({'id':i,'result':{'turn':{'id':'provider-turn'}}}),flush=True)
  print(json.dumps({'method':'item/agentMessage/delta','params':{'delta':'partial response'}}),flush=True)
  continue
 elif m=='turn/interrupt':
  print(json.dumps({'method':'turn/completed','params':{'turn':{'status':'interrupted'}}}),flush=True);continue
 else:continue
 print(json.dumps({'id':i,'result':r}),flush=True)
"##).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, uuid::Uuid::new_v4().to_string())
}
#[test]
fn stream_preserves_partial_answer_on_cancel_and_blocks_analysis() {
    let (dir, ws) = fixture();
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", dir.path().to_str().unwrap(), "--chat-stream"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = p.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"workspace_id":ws,"provider":"codex","model":"gpt-6.1-terra","effort":"high","text":"test","images":[{"mime":"image/png","data":"iVBORw0KGgo="}]})
    )
    .unwrap();
    input.flush().unwrap();
    let mut events = Vec::new();
    for line in BufReader::new(p.stdout.take().unwrap()).lines() {
        let e: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let kind = e["event"].as_str().unwrap().to_string();
        events.push(e);
        if kind == "text_delta" {
            let blocked = stology_core::providers::handle(
                dir.path(),
                &json!({"op":"provider.install","provider":"codex"}),
            );
            assert!(blocked.is_err());
            writeln!(input, "{{\"op\":\"stop\"}}").unwrap();
            input.flush().unwrap();
        }
        if ["completed", "interrupted", "failed"].contains(&kind.as_str()) {
            break;
        }
    }
    assert_eq!(events.first().unwrap()["event"], "accepted");
    assert_eq!(events.last().unwrap()["event"], "interrupted");
    assert_eq!(events.last().unwrap()["answer"], "partial response");
    drop(input);
    assert!(p.wait().unwrap().success());
    let state = stology_core::dispatch(
        dir.path(),
        &json!({"op":"chat.get","workspace_id":ws,"provider":"codex"}),
    )
    .unwrap();
    assert_eq!(state["turns"][0]["state"], "interrupted");
    assert_eq!(state["turns"][0]["model"], "gpt-6.1-terra");
    assert_eq!(state["turns"][0]["effort"], "high");
    assert_eq!(state["turns"][0]["images"], 1);
    assert_eq!(state["session_id"], "fixture-session");
    assert!(events
        .windows(2)
        .all(|w| w[0]["sequence"].as_u64() < w[1]["sequence"].as_u64()));
}
#[test]
fn missing_session_fails_without_silent_replacement() {
    let (dir, ws) = fixture();
    stology_core::dispatch(
        dir.path(),
        &json!({"op":"chat.get","workspace_id":ws,"provider":"codex"}),
    )
    .unwrap();
    let c = rusqlite::Connection::open(dir.path().join("chat.sqlite3")).unwrap();
    c.execute("UPDATE conversations SET session='missing'", [])
        .unwrap();
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root", dir.path().to_str().unwrap(), "--chat-stream"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = p.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"workspace_id":ws,"provider":"codex","text":"test"})
    )
    .unwrap();
    input.flush().unwrap();
    let mut last = Value::Null;
    for line in BufReader::new(p.stdout.take().unwrap()).lines() {
        last = serde_json::from_str(&line.unwrap()).unwrap();
        if last["event"] == "failed" {
            break;
        }
    }
    assert_eq!(last["event"], "failed");
    drop(input);
    p.wait().unwrap();
    let v = stology_core::dispatch(
        dir.path(),
        &json!({"op":"chat.get","workspace_id":ws,"provider":"codex"}),
    )
    .unwrap();
    assert_eq!(v["needs_new"], true);
    assert_eq!(v["session_id"], "missing");
}

#[test]
fn catalog_and_model_preferences_survive_reopening_and_provider_switch() {
    let (dir, _) = fixture();
    let call = |v| stology_core::dispatch(dir.path(), &v).unwrap();
    let catalog = call(json!({"op":"chat.models","provider":"codex"}));
    assert_eq!(catalog["models"][0]["id"], "gpt-6.1-terra");
    call(json!({"op":"chat.preferences","provider":"codex","model":"gpt-6.1-terra"}));
    call(json!({"op":"chat.preferences","provider":"claude","model":"sonnet"}));
    let prefs = call(json!({"op":"chat.preferences","provider":"codex"}));
    assert_eq!(prefs["models"]["codex"], "gpt-6.1-terra");
    assert_eq!(prefs["models"]["claude"], "sonnet");
    assert!(stology_core::dispatch(dir.path(), &json!({"op":"chat.preferences","provider":"claude","model":"gpt-6.1-terra"})).is_err());
    assert_eq!(call(json!({"op":"chat.preferences"}))["models"]["claude"], "sonnet");
}

fn claude_fixture() -> (tempfile::TempDir, String) {
    let (dir, ws) = fixture();
    let path = dir.path().join("runtime/bin/claude");
    fs::write(&path, r##"#!/usr/bin/env python3
import sys,json,pathlib
if 'auth' in sys.argv:
 print(json.dumps({'loggedIn':True,'authMethod':'claude.ai'}));sys.exit(0)
if '--version' in sys.argv:
 print('fixture');sys.exit(0)
models=[
 {'value':'default','resolvedModel':'claude-opus-5-5','displayName':'Default','supportedEffortLevels':['low','medium','high','xhigh','max']},
 {'value':'opus','resolvedModel':'claude-opus-5-5','displayName':'Opus 5.5','supportedEffortLevels':['low','medium','high','xhigh','max']},
 {'value':'claude-opus-4-6','resolvedModel':'claude-opus-4-6','displayName':'Opus 4.6','supportedEffortLevels':['low','medium','high','max']},
 {'value':'claude-haiku-4-5-20251001','displayName':'Haiku 4.5'}]
for line in sys.stdin:
 v=json.loads(line)
 if v['type']=='control_request':
  assert '--no-session-persistence' in sys.argv
  print(json.dumps({'type':'control_response','response':{'request_id':v['request_id'],'subtype':'success','response':{'models':models}}}),flush=True)
 elif v['type']=='user':
  with (pathlib.Path(__file__).parents[2]/'invocations.jsonl').open('a') as f:f.write(json.dumps(sys.argv[1:])+'\n')
  print(json.dumps({'type':'system','subtype':'init','tools':[],'session_id':'claude-fixture'}),flush=True)
  print(json.dumps({'type':'stream_event','event':{'delta':{'type':'text_delta','text':'fixture answer'}}}),flush=True)
  print(json.dumps({'type':'result','is_error':False}),flush=True)
"##).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, ws)
}
fn run_claude(root: &std::path::Path, ws: &str, model: &str, effort: &str) -> Vec<Value> {
    let mut p = Command::new(env!("CARGO_BIN_EXE_stology-core"))
        .args(["--root",root.to_str().unwrap(),"--chat-stream"])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    let mut input = p.stdin.take().unwrap();
    writeln!(input,"{}",json!({"workspace_id":ws,"provider":"claude","model":model,"effort":effort,"text":"fixture"})).unwrap();
    input.flush().unwrap();
    let mut events = Vec::new();
    for line in BufReader::new(p.stdout.take().unwrap()).lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        let done = ["failed","completed"].contains(&v["event"].as_str().unwrap_or(""));
        events.push(v);
        if done { break; }
    }
    drop(input);p.wait().unwrap();events
}
#[test]
fn claude_versions_effort_and_resume_reach_cli_and_turn_storage() {
    let (dir, ws) = claude_fixture();
    let catalog = stology_core::dispatch(dir.path(), &json!({"op":"chat.models","provider":"claude"})).unwrap();
    let items = catalog["models"].as_array().unwrap();
    assert!(items.iter().any(|m|m["id"]=="claude-opus-5-5" && m["label"]=="Opus 5.5"));
    assert!(items.iter().any(|m|m["id"]=="claude-opus-4-6"));
    assert!(items.iter().any(|m|m["id"]=="opus" && m["resolved_model"]=="claude-opus-5-5"));
    for (model, effort) in [("claude-opus-4-6","high"),("claude-opus-5-5","xhigh"),("claude-opus-5-5","")] {
        let events = run_claude(dir.path(), &ws, model, effort);
        assert_eq!(events.last().unwrap()["event"],"completed", "{events:?}");
        assert_eq!(events[0]["model"], model);
        assert_eq!(events[0]["effort"], effort);
    }
    let invocations: Vec<Vec<String>> = fs::read_to_string(dir.path().join("invocations.jsonl")).unwrap().lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    assert!(invocations[0].windows(2).any(|a|a==["--model","claude-opus-4-6"]));
    assert!(invocations[0].windows(2).any(|a|a==["--effort","high"]));
    assert!(!invocations[0].contains(&"--resume".into()));
    assert!(invocations[1].windows(2).any(|a|a==["--resume","claude-fixture"]));
    assert!(invocations[1].windows(2).any(|a|a==["--effort","xhigh"]));
    assert!(!invocations[2].contains(&"--effort".into()));
    let state = stology_core::dispatch(dir.path(), &json!({"op":"chat.get","workspace_id":ws,"provider":"claude"})).unwrap();
    assert_eq!(state["turns"][0]["model"],"claude-opus-4-6");
    assert_eq!(state["turns"][0]["effort"],"high");
    assert_eq!(state["turns"][1]["effort"],"xhigh");
    assert_eq!(state["turns"][2]["effort"],"");
}
#[test]
fn unsupported_effort_is_rejected_before_inference_and_preferences_are_per_model() {
    let (dir, ws) = claude_fixture();
    for (model, effort) in [("claude-opus-4-6","xhigh"),("claude-haiku-4-5-20251001","high")] {
        let events=run_claude(dir.path(),&ws,model,effort);
        assert_eq!(events.last().unwrap()["event"],"failed");
        assert!(events.last().unwrap()["error"].as_str().unwrap().contains("effort"));
    }
    assert!(!dir.path().join("invocations.jsonl").exists());
    for (model,effort) in [("claude-opus-4-6","high"),("claude-opus-5-5","max")] {
        stology_core::dispatch(dir.path(), &json!({"op":"chat.preferences","provider":"claude","model":model,"effort":effort})).unwrap();
    }
    let prefs = stology_core::dispatch(dir.path(), &json!({"op":"chat.preferences"})).unwrap();
    assert_eq!(prefs["efforts"]["claude.claude-opus-4-6"],"high");
    assert_eq!(prefs["efforts"]["claude.claude-opus-5-5"],"max");
}
