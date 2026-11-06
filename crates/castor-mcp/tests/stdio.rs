//! End-to-end: start `castor-mcp`, speak MCP over stdio like Claude Desktop does.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

#[test]
fn initialize_list_describe_and_run() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_castor-mcp"))
        .env("CASTOR_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut send = |v: Value| {
        writeln!(stdin, "{v}").unwrap();
        stdin.flush().unwrap();
    };
    let mut recv = |id: i64| -> Value {
        loop {
            let l = lines.next().expect("server closed").unwrap();
            let v: Value = serde_json::from_str(&l).unwrap();
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return v;
            }
        }
    };
    send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}}));
    let r = recv(1);
    assert_eq!(r["result"]["serverInfo"]["name"], "castor", "{r}");
    send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let r = recv(2);
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_tools", "describe_tool", "run_tool", "run_chain", "history"]);

    send(json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "list_tools", "arguments": {"query": "qr code"}}}));
    let r = recv(3);
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("create.qr-code"), "{text}");

    send(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "describe_tool", "arguments": {"id": "create.qr-code"}}}));
    let r = recv(4);
    let d: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(d["options"]["properties"]["style"]["type"], "string");
    assert!(d["options"]["properties"]["style"]["enum"].as_array().unwrap().len() >= 12);

    send(json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "run_tool", "arguments": {
        "id": "create.qr-code", "values": {"url": "https://basedpolymer.eu/castor", "size": 400}, "output_dir": out.path()}}}));
    let r = recv(5);
    assert_ne!(r["result"]["isError"], true, "{r}");
    let res: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(res["status"], "complete", "{res}");
    let png = res["outputs"][0].as_str().unwrap();
    assert!(std::path::Path::new(png).is_file());

    // A bad option is a readable tool error, not a protocol error.
    send(json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call", "params": {"name": "run_tool", "arguments": {
        "id": "create.qr-code", "values": {"nope": 1}}}}));
    let r = recv(6);
    assert_eq!(r["result"]["isError"], true, "{r}");

    send(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": "history", "arguments": {"limit": 5}}}));
    let r = recv(7);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("create.qr-code"));

    drop(send);
    let _ = child.kill();
    let _ = child.wait();
}
