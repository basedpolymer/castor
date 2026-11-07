//! End-to-end: start `castor-mcp`, speak MCP over stdio like Claude Desktop does.

use std::io::{BufRead, BufReader, Lines, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Server {
    child: Child,
    stdin: ChildStdin,
    lines: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

impl Server {
    fn start(home: &std::path::Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_castor-mcp"))
            .env("CASTOR_HOME", home)
            .env("CASTOR_LANG", "en")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut s = Server { child, stdin, lines, next_id: 1 };
        let r = s.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
        assert_eq!(r["result"]["serverInfo"]["name"], "castor", "{r}");
        s.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        s
    }

    fn send(&mut self, v: Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let l = self.lines.next().expect("server closed").unwrap();
            let v: Value = serde_json::from_str(&l).expect("stdout carries only JSON-RPC");
            if v.get("id").and_then(Value::as_i64) == Some(id) {
                return v;
            }
        }
    }

    /// Call an MCP tool: (is_error, first content block).
    fn call(&mut self, name: &str, args: Value) -> (bool, Value) {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        let is_error = r["result"]["isError"] == true;
        (is_error, r["result"]["content"][0].clone())
    }

    fn call_json(&mut self, name: &str, args: Value) -> (bool, Value) {
        let (e, c) = self.call(name, args);
        let text = c["text"].as_str().unwrap_or_default().to_string();
        (e, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn initialize_list_describe_and_run() {
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut s = Server::start(home.path());

    let r = s.request("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["list_tools", "describe_tool", "preview_tool", "run_tool", "run_chain", "undo_run", "history"]);
    let ann = |n: &str| tools.iter().find(|t| t["name"] == n).unwrap()["annotations"].clone();
    assert_eq!(ann("list_tools")["readOnlyHint"], true);
    assert_eq!(ann("run_tool")["readOnlyHint"], false);
    assert_eq!(ann("run_tool")["destructiveHint"], true);
    assert_eq!(ann("run_tool")["openWorldHint"], true);

    let (_, c) = s.call("list_tools", json!({"query": "qr code"}));
    assert!(c["text"].as_str().unwrap().contains("create.qr-code"), "{c}");

    let (_, d) = s.call_json("describe_tool", json!({"id": "create.qr-code"}));
    assert_eq!(d["options"]["properties"]["style"]["type"], "string");
    assert!(d["options"]["properties"]["style"]["enum"].as_array().unwrap().len() >= 12);
    let desc = d["options"]["properties"]["style"]["description"].as_str().unwrap();
    assert!(!desc.contains(".."), "no doubled full stop: {desc}");

    let (e, res) = s.call_json("run_tool", json!({
        "id": "create.qr-code", "values": {"url": "https://basedpolymer.eu/castor", "size": 400}, "output_dir": out.path()}));
    assert!(!e, "{res}");
    assert_eq!(res["status"], "complete", "{res}");
    let png = res["outputs"][0].as_str().unwrap();
    assert!(std::path::Path::new(png).is_file());

    // A bad option is a readable tool error, not a protocol error.
    let (e, _) = s.call("run_tool", json!({"id": "create.qr-code", "values": {"nope": 1}}));
    assert!(e);
    // Out of range is an error, not clamped in silence.
    let (e, c) = s.call("run_tool", json!({"id": "create.qr-code", "values": {"size": 1_000_000}, "output_dir": out.path()}));
    assert!(e, "{c}");
    // Relative paths are refused: the server's working folder is not the user's.
    let (e, c) = s.call("run_tool", json!({"id": "create.qr-code", "output_dir": "relative/out"}));
    assert!(e && c["text"].as_str().unwrap().contains("absolute"), "{c}");

    let (_, c) = s.call("history", json!({"limit": 5}));
    assert!(c["text"].as_str().unwrap().contains("create.qr-code"));
}

#[test]
fn chains_are_checked_strictly() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let img = dir.path().join("a.png");
    image::RgbImage::from_pixel(300, 200, image::Rgb([200, 100, 50])).save(&img).unwrap();
    std::fs::write(dir.path().join("notes.txt"), "hello").unwrap();
    let mut s = Server::start(home.path());

    // A wrong type, an unknown key and an empty chain stop before anything runs.
    let (e, c) = s.call("run_chain", json!({"toml": "name = \"x\"\n[[steps]]\ntool = \"image.resize\"\nvalues = { quality = \"lots\", nope = 1 }", "files": [img]}));
    assert!(e, "{c}");
    let t = c["text"].as_str().unwrap();
    assert!(t.contains("quality") && t.contains("nope"), "every problem is listed: {t}");
    let (e, _) = s.call("run_chain", json!({"toml": "name = \"empty\""}));
    assert!(e);

    // A size given as text works in a chain, like on the command line.
    let (e, res) = s.call_json("run_chain", json!({
        "toml": "name = \"x\"\n[[steps]]\ntool = \"image.resize\"\nvalues = { size = \"150x100\" }",
        "files": [img, dir.path().join("notes.txt")], "output_dir": out.path()}));
    assert!(!e, "{res}");
    let resized = image::image_dimensions(res["outputs"][0].as_str().unwrap()).unwrap();
    assert_eq!(resized, (150, 100));
    // The text file is reported as skipped, not dropped in silence.
    assert!(res["skipped"].to_string().contains("notes.txt"), "{res}");
}

#[test]
fn in_place_runs_can_be_previewed_and_undone() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    for n in ["b one.txt", "A two.txt"] {
        std::fs::write(dir.path().join(n), n).unwrap();
    }
    let mut s = Server::start(home.path());
    let values = json!({"pattern": "file-{n:02}"});

    let r = s.request("tools/call", json!({"name": "preview_tool", "arguments": {"id": "files.rename", "files": [dir.path()], "values": values}}));
    assert_ne!(r["result"]["isError"], true, "{r}");
    assert_eq!(r["result"]["content"][0]["type"], "image", "{r}");
    assert!(dir.path().join("b one.txt").exists(), "a preview changes nothing");

    let (e, res) = s.call_json("run_tool", json!({"id": "files.rename", "files": [dir.path()], "values": values}));
    assert!(!e, "{res}");
    assert_eq!(res["undoable"], true);
    assert!(dir.path().join("file-01.txt").exists());

    let (e, u) = s.call_json("undo_run", json!({"run_id": res["run_id"]}));
    assert!(!e, "{u}");
    assert!(dir.path().join("b one.txt").exists() && dir.path().join("A two.txt").exists());
    let (e, _) = s.call("undo_run", json!({"run_id": res["run_id"]}));
    assert!(e, "a run is undone once");
}
