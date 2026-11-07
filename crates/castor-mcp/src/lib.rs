//! Castor as an MCP server (Model Context Protocol, stdio transport, official `rmcp` SDK).
//!
//! Seven MCP tools expose every Castor tool to AI agents:
//! - `list_tools` — ids, names, descriptions, families, inputs and outputs (optional filter);
//! - `describe_tool` — a JSON Schema of a tool's options, derived from its `OptionSpec`s;
//! - `preview_tool` — the tool's live preview as an image (what a rename will do, a QR code…);
//! - `run_tool` — run a tool on files with option values, returns outputs and notes;
//! - `run_chain` — run a chain (TOML, same format as the app and `castor-cli chain`);
//! - `undo_run` — undo an in-place run (rename, dates, sort…) from the history;
//! - `history` — recent runs.
//!
//! Runs go through `castor_core::runner`, exactly like the app and the CLI (history included).
//! Values are checked strictly ([`OptionSpec::check`]): nothing is clamped or ignored in silence.
//! Paths must be absolute (the server's working folder is not the user's). One-time downloads
//! need `allow_downloads: true`, so the agent asks the user first. A cancelled request stops the
//! run; long runs report progress when the client sends a progress token.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use castor_core::download::Asset;
use castor_core::history::{self, RunStatus};
use castor_core::options::{OptionKind, OptionSpec, Value, Values};
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{CancelToken, Cond, FileInfo, Registry, Tool, tr, tr_args};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities, ServerConfig, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value as Json, json};

/// Side of the image returned by `preview_tool`.
const PREVIEW_SIDE: u32 = 1024;

/// The server state: the registry of every Castor tool and the runs in progress.
#[derive(Clone)]
pub struct CastorMcp {
    reg: Arc<Registry>,
    runs: Arc<Mutex<HashMap<u64, CancelToken>>>,
    next_run: Arc<AtomicU64>,
}

/// Progress of a run, as sent in `notifications/progress`: `progress` out of `total` (= steps).
#[derive(Clone, Debug)]
pub struct Progress {
    pub progress: f64,
    pub total: f64,
    pub message: String,
}

impl CastorMcp {
    pub fn new(reg: Registry) -> Self {
        CastorMcp { reg: Arc::new(reg), runs: Default::default(), next_run: Default::default() }
    }

    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<u64, CancelToken>> {
        self.runs.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Cancel every call still running (the client went away).
    pub fn cancel_all(&self) {
        for c in self.runs().values() {
            c.cancel();
        }
    }
}

/// Serve on stdin/stdout until the client disconnects. Used by `castor-mcp` and `castor mcp`.
pub async fn serve_stdio() -> anyhow::Result<()> {
    let lang = std::env::var("CASTOR_LANG").unwrap_or_else(|_| "en".into());
    castor_core::i18n::set_language(&lang);
    let server = CastorMcp::new(castor_tools::registry());
    let keep = server.clone();
    let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| anyhow::anyhow!("MCP initialisation failed: {e}"))?;
    let res = running.waiting().await;
    // Nobody is waiting for the results any more: stop the runs instead of finishing them.
    keep.cancel_all();
    res?;
    Ok(())
}

/// Blocking entry point (creates its own Tokio runtime).
pub fn run_stdio() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let res = rt.block_on(serve_stdio());
    // Cancelled runs stop at their next check; never hang on exit because of a stuck helper.
    rt.shutdown_timeout(std::time::Duration::from_secs(10));
    res
}

fn obj(v: Json) -> Arc<JsonObject> {
    match v {
        Json::Object(m) => Arc::new(m),
        _ => Arc::new(JsonObject::new()),
    }
}

fn annotations(title: &str, read_only: bool, open_world: bool) -> ToolAnnotations {
    let a = ToolAnnotations::with_title(title).read_only(read_only).open_world(open_world);
    if read_only { a } else { a.destructive(true).idempotent(false) }
}

/// The seven MCP tools.
pub fn mcp_tools() -> Vec<rmcp::model::Tool> {
    let files = json!({"type": "array", "items": {"type": "string"}, "description": "Absolute paths of input files or folders (folders are expanded; '~' is your home folder)"});
    let output_dir = json!({"type": "string", "description": "Absolute path of the folder for the results (created if needed). Default: next to the inputs"});
    let allow_downloads = json!({"type": "boolean", "default": false, "description": "Allow one-time downloads (models, OCR languages, helper programs). Leave false: if something must be downloaded, the call fails with the list and sizes; ask the user, then call again with true"});
    vec![
        rmcp::model::Tool::new(
            "list_tools",
            "List Castor's offline file tools (PDF, image, video, audio, create, files). Returns id, name, description, family, output formats, whether input files are needed and whether the tool changes files in place. Optional: family (pdf|image|video|audio|create|files) and query (keywords, any language).",
            obj(json!({"type": "object", "properties": {
                "family": {"type": "string", "enum": ["pdf", "image", "video", "audio", "create", "files"]},
                "query": {"type": "string", "description": "Search words, e.g. 'compress pdf' or 'qr code'"}
            }})),
        )
        .with_annotations(annotations("List Castor tools", true, false)),
        rmcp::model::Tool::new(
            "describe_tool",
            "Describe one Castor tool: what it does, which files it accepts, what it still needs to download, and the JSON Schema of its options (types, ranges, choices, defaults). Call this before run_tool.",
            obj(json!({"type": "object", "required": ["id"], "properties": {"id": {"type": "string", "description": "Tool id, e.g. pdf.compress"}}})),
        )
        .with_annotations(annotations("Describe a Castor tool", true, false)),
        rmcp::model::Tool::new(
            "preview_tool",
            "Show what a Castor tool would do, as an image, without changing anything: the before/after names of a batch rename, a QR code or another creation, an image effect… Uses the first accepted file or folder. Use it before running a tool that changes files in place (in_place: true).",
            obj(json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "files": files,
                "values": {"type": "object", "description": "Option values by key (see describe_tool)"}
            }})),
        )
        .with_annotations(annotations("Preview a Castor tool", true, false)),
        rmcp::model::Tool::new(
            "run_tool",
            "Run a Castor tool locally on files. 'values' sets options (see describe_tool; anything left out keeps its default; wrong keys, types or out-of-range values are errors). Results go next to the inputs, or into output_dir; existing files are never overwritten. Returns the output files, notes, errors and the files the tool skipped. Tools marked in_place (rename, dates, sort…) change the input files: preview them first; undo_run reverts them.",
            obj(json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "files": files,
                "values": {"type": "object", "description": "Option values by key: strings for choices, colours (\"#1b1616\"), paths (absolute) and sizes (\"1200x630\" or [1200, 630]); numbers; booleans"},
                "output_dir": output_dir,
                "allow_downloads": allow_downloads
            }})),
        )
        .with_annotations(annotations("Run a Castor tool", false, true)),
        rmcp::model::Tool::new(
            "run_chain",
            "Run a chain of Castor tools (outputs of step k feed step k+1). 'toml' uses the chain format: name = \"…\" then [[steps]] tool = \"image.resize\" values = { mode = \"fit\", size = [2048, 2048] }. Values are checked like run_tool's: a wrong key, type or value stops the chain before anything runs.",
            obj(json!({"type": "object", "required": ["toml"], "properties": {
                "toml": {"type": "string"},
                "files": files,
                "output_dir": output_dir,
                "allow_downloads": allow_downloads
            }})),
        )
        .with_annotations(annotations("Run a Castor chain", false, true)),
        rmcp::model::Tool::new(
            "undo_run",
            "Undo a run that changed files in place (rename, dates, sort, duplicates moved aside…): files go back to their old names and places. run_id comes from run_tool, run_chain or history ('undoable': true).",
            obj(json!({"type": "object", "required": ["run_id"], "properties": {"run_id": {"type": "string"}}})),
        )
        .with_annotations(annotations("Undo a Castor run", false, false)),
        rmcp::model::Tool::new(
            "history",
            "Recent Castor runs, from the app, the command line and agents (most recent first): run id, tool, status, inputs, outputs, sizes, errors, whether it can be undone.",
            obj(json!({"type": "object", "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20}}})),
        )
        .with_annotations(annotations("Castor run history", true, false)),
    ]
}

/// Append a sentence to a description, without doubling the full stop.
fn sentence(desc: &mut String, more: &str) {
    let more = more.trim();
    if more.is_empty() {
        return;
    }
    let d = desc.trim_end();
    *desc = if d.is_empty() {
        more.to_string()
    } else if d.ends_with(['.', '!', '?', ':', '。', '！', '？']) {
        format!("{d} {more}")
    } else {
        format!("{d}. {more}")
    };
}

/// JSON Schema (draft 2020-12 subset) for a tool's options.
pub fn options_schema(tool: &dyn Tool) -> Json {
    let id = tool.meta().id;
    let mut props = serde_json::Map::new();
    for o in tool.options() {
        props.insert(o.key.to_string(), option_schema(id, &o));
    }
    json!({"type": "object", "properties": props, "additionalProperties": false})
}

fn option_schema(tool_id: &str, o: &OptionSpec) -> Json {
    let mut desc = tr(&o.label_key(tool_id));
    let help_key = format!("{}.help", o.label_key(tool_id));
    if castor_core::i18n::has(&help_key) {
        sentence(&mut desc, &tr(&help_key));
    }
    let default = match &o.default {
        Value::Bool(b) => json!(b),
        Value::Int(n) => json!(n),
        Value::Float(f) => json!(f),
        Value::Size(w, h) => json!(format!("{w}x{h}")),
        Value::Text(s) => json!(s),
        Value::Path(p) => p.as_ref().map(|p| json!(p.to_string_lossy())).unwrap_or(Json::Null),
    };
    let mut s = match &o.kind {
        OptionKind::Choice(cs) => {
            let labels: Vec<String> = cs.iter().map(|c| format!("{} = {}", c.value, o.choice_label(tool_id, c))).collect();
            sentence(&mut desc, &tr_args("mcp.schema.choices", &[("list", &labels.join("; "))]));
            json!({"type": "string", "enum": cs.iter().map(|c| c.value).collect::<Vec<_>>()})
        }
        OptionKind::Toggle => json!({"type": "boolean"}),
        OptionKind::Int { min, max, unit, .. } => {
            if let Some(u) = unit {
                desc.push_str(&format!(" [{u}]"));
            }
            json!({"type": "integer", "minimum": min, "maximum": max})
        }
        OptionKind::Float { min, max, unit, .. } => {
            if let Some(u) = unit {
                desc.push_str(&format!(" [{u}]"));
            }
            json!({"type": "number", "minimum": min, "maximum": max})
        }
        OptionKind::Text { .. } | OptionKind::Font => json!({"type": "string"}),
        OptionKind::Password => json!({"type": "string", "writeOnly": true}),
        OptionKind::Color => json!({"type": "string", "pattern": "^#([0-9a-fA-F]{3}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$|^$"}),
        OptionKind::Path { folder, filters } => {
            sentence(&mut desc, &tr(if *folder { "mcp.schema.folder" } else { "mcp.schema.file" }));
            if !filters.is_empty() {
                desc.push_str(&format!(" [{}]", filters.join(", ")));
            }
            json!({"type": "string"})
        }
        OptionKind::PageRange => {
            sentence(&mut desc, &tr("mcp.schema.pages"));
            json!({"type": "string"})
        }
        OptionKind::Anchor => json!({"type": "string", "enum": castor_core::options::Anchor::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>()}),
        OptionKind::Size => json!({"type": "string", "pattern": "^[0-9]+x[0-9]+$"}),
        OptionKind::Language { multi, list } => {
            let codes: Vec<&str> = list.iter().map(|(c, _)| *c).collect();
            if *multi {
                sentence(&mut desc, &tr_args("mcp.schema.languages", &[("list", &codes.join(", "))]));
                json!({"type": "string"})
            } else {
                json!({"type": "string", "enum": codes})
            }
        }
    };
    if let Some(c) = &o.visible_if {
        let cond = match c {
            Cond::Eq(k, v) if v.is_empty() => tr_args("mcp.schema.is_empty", &[("key", k)]),
            Cond::Ne(k, v) if v.is_empty() => tr_args("mcp.schema.not_empty", &[("key", k)]),
            Cond::Eq(k, v) => format!("{k} = {v}"),
            Cond::Ne(k, v) => format!("{k} ≠ {v}"),
            Cond::True(k) => tr_args("mcp.schema.is_on", &[("key", k)]),
            Cond::False(k) => tr_args("mcp.schema.is_off", &[("key", k)]),
        };
        sentence(&mut desc, &tr_args("mcp.schema.used_when", &[("cond", &cond)]));
    }
    s["description"] = json!(desc);
    if !default.is_null() {
        s["default"] = default;
    }
    s
}

/// A JSON value as a Castor value (`None` for null = keep the default).
fn json_to_value(v: &Json) -> Result<Option<Value>, String> {
    Ok(Some(match v {
        Json::Null => return Ok(None),
        Json::Bool(b) => Value::Bool(*b),
        Json::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        Json::String(s) => Value::Text(s.clone()),
        Json::Array(a) if a.len() == 2 => {
            let dim = |x: &Json| -> Option<u32> {
                match x {
                    Json::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
                    Json::String(s) => s.trim().parse().ok(),
                    _ => None,
                }
            };
            match (dim(&a[0]), dim(&a[1])) {
                (Some(w), Some(h)) => Value::Size(w, h),
                _ => return Err(format!("expected [width, height] in whole pixels, got {v}")),
            }
        }
        other => return Err(format!("unsupported value {other} (use a string, a number, true/false or [width, height])")),
    }))
}

/// Convert JSON option values to Castor values, checked strictly against the schema.
/// Every problem is reported, one per line.
pub fn parse_values(tool: &dyn Tool, given: Option<&JsonObject>) -> Result<Values, String> {
    let specs = tool.options();
    let mut values = Values::defaults(&specs);
    let Some(given) = given else { return Ok(values) };
    let mut problems = Vec::new();
    for (k, v) in given {
        let Some(spec) = specs.iter().find(|s| s.key == k) else {
            problems.push(format!(
                "'{}' has no option '{k}' (options: {})",
                tool.meta().id,
                specs.iter().map(|s| s.key).collect::<Vec<_>>().join(", ")
            ));
            continue;
        };
        match json_to_value(v).and_then(|v| v.map(|v| spec.check(&v).map_err(|e| format!("{e:#}"))).transpose()) {
            Ok(Some(v)) => values.set(k, v),
            Ok(None) => {}
            Err(e) => problems.push(format!("option '{k}': {e}")),
        }
    }
    if problems.is_empty() { Ok(values) } else { Err(problems.join("\n")) }
}

/// An absolute path from an agent: `~` is expanded, relative paths are refused (the server runs
/// in a folder of its own, often `/`, not in the user's).
fn abs_path(s: &str) -> Result<PathBuf, String> {
    let t = s.trim();
    let p = if t == "~" || t.starts_with("~/") || t.starts_with("~\\") {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .ok_or_else(|| format!("cannot expand '~' in '{t}': no home folder"))?;
        let mut h = PathBuf::from(home);
        if t.len() > 2 {
            h.push(&t[2..]);
        }
        h
    } else {
        PathBuf::from(t)
    };
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(format!(
            "'{s}' is not an absolute path. The Castor server does not run in your folder: give full paths, e.g. /Users/me/report.pdf or C:\\Users\\me\\report.pdf"
        ))
    }
}

/// Path options (logo, font file, output folder…) must be absolute too.
fn absolutize_paths(tool: &dyn Tool, values: &mut Values) -> Result<(), String> {
    let mut problems = Vec::new();
    for spec in tool.options().iter().filter(|s| matches!(s.kind, OptionKind::Path { .. })) {
        if let Some(p) = values.path(spec.key).map(|p| p.to_string_lossy().into_owned()) {
            match abs_path(&p) {
                Ok(p) => values.set(spec.key, Value::Path(Some(p))),
                Err(e) => problems.push(format!("{}: option '{}': {e}", tool.meta().id, spec.key)),
            }
        }
    }
    if problems.is_empty() { Ok(()) } else { Err(problems.join("\n")) }
}

fn text(s: impl Into<String>) -> ContentBlock {
    ContentBlock::text(s)
}

fn ok_json(v: Json) -> CallToolResponse {
    CallToolResult::success(vec![text(serde_json::to_string_pretty(&v).unwrap_or_default())]).into()
}

fn err_json(v: Json) -> CallToolResponse {
    CallToolResult::error(vec![text(serde_json::to_string_pretty(&v).unwrap_or_default())]).into()
}

fn err(msg: impl Into<String>) -> CallToolResponse {
    CallToolResult::error(vec![text(msg)]).into()
}

/// `?` for tool errors: `Err` is the error response to send back.
type Reply<T> = Result<T, CallToolResponse>;

fn str_arg<'a>(args: Option<&'a JsonObject>, k: &str) -> Option<&'a str> {
    args.and_then(|a| a.get(k)).and_then(Json::as_str)
}

fn bool_arg(args: Option<&JsonObject>, k: &str) -> bool {
    args.and_then(|a| a.get(k)).and_then(Json::as_bool).unwrap_or(false)
}

/// The `files` argument: absolute, existing paths; folders are expanded.
fn files_arg(args: Option<&JsonObject>) -> Reply<Vec<FileInfo>> {
    let arr = match args.and_then(|a| a.get("files")) {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Array(a)) => a,
        Some(other) => return Err(err(format!("'files' must be an array of absolute paths, e.g. [\"/Users/me/a.pdf\"]; got {other}"))),
    };
    let mut paths = Vec::new();
    let mut problems = Vec::new();
    for x in arr {
        match x.as_str().ok_or_else(|| format!("not a path: {x}")).and_then(abs_path) {
            Ok(p) if p.exists() => paths.push(p),
            Ok(p) => problems.push(format!("{} does not exist", p.display())),
            Err(e) => problems.push(e),
        }
    }
    if !problems.is_empty() {
        return Err(err(problems.join("\n")));
    }
    Ok(castor_core::files::expand(&paths, true))
}

/// Files the first tool takes, and the ones it skips (reported to the agent).
fn split_inputs(tool: &dyn Tool, files: Vec<FileInfo>) -> (Vec<FileInfo>, Vec<PathBuf>) {
    let (usable, skipped): (Vec<FileInfo>, Vec<FileInfo>) = files.into_iter().partition(|f| tool.accepts(f));
    (usable, skipped.into_iter().map(|f| f.path).collect())
}

fn check_input_count(tool: &dyn Tool, usable: usize, skipped: &[PathBuf]) -> Reply<()> {
    let m = tool.meta();
    let skipped_note = if skipped.is_empty() {
        String::new()
    } else {
        format!(" Skipped (not accepted by {}): {}", m.id, skipped.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "))
    };
    if usable < m.inputs.min {
        return Err(err(format!("{} needs at least {} matching file(s); {usable} given are usable.{skipped_note}", m.id, m.inputs.min)));
    }
    if let Some(max) = m.inputs.max.filter(|max| usable > *max) {
        return Err(err(format!("{} takes at most {max} file(s); {usable} given.", m.id)));
    }
    Ok(())
}

fn output_policy(args: Option<&JsonObject>) -> Reply<OutputPolicy> {
    Ok(match str_arg(args, "output_dir").filter(|s| !s.trim().is_empty()) {
        Some(d) => {
            let p = abs_path(d).map_err(|e| err(format!("output_dir: {e}")))?;
            std::fs::create_dir_all(&p).map_err(|e| err(format!("output_dir {}: {e}", p.display())))?;
            OutputPolicy::Folder(p)
        }
        None => OutputPolicy::NextToInput { fallback: castor_core::paths::default_output_dir() },
    })
}

/// One-time downloads the steps would need and that are not on this computer yet.
fn missing_downloads(steps: &[Step]) -> Vec<Asset> {
    let mut seen = HashSet::new();
    steps.iter().flat_map(|s| s.tool.downloads(&s.values)).filter(|a| !a.is_present() && seen.insert(a.id)).collect()
}

/// Refuse to download without the user's consent (CLAUDE.md rule 7: name + size first).
fn check_downloads(steps: &[Step], args: Option<&JsonObject>) -> Reply<()> {
    let missing = missing_downloads(steps);
    if missing.is_empty() || bool_arg(args, "allow_downloads") {
        return Ok(());
    }
    let total: u64 = missing.iter().map(|a| a.size).sum();
    Err(err_json(json!({
        "error": "downloads_needed",
        "message": "This run needs one-time downloads (below). Ask the user first; if they agree, call again with allow_downloads: true. Nothing was run.",
        "downloads": missing.iter().map(|a| json!({
            "id": a.id, "size_bytes": a.size, "size": castor_core::units::bytes(a.size), "license": a.license, "url": a.url,
        })).collect::<Vec<_>>(),
        "total_bytes": total,
        "total": castor_core::units::bytes(total),
    })))
}

fn record_json(r: &history::RunRecord) -> Json {
    json!({
        "run_id": r.id, "status": r.status, "tool": r.tool_id, "chain_name": r.chain_name, "steps": r.steps,
        "files_in": r.files_in, "files_out": r.files_out,
        "bytes_in": r.bytes_in, "bytes_out": r.bytes_out,
        "duration_secs": r.duration_secs,
        "outputs": r.outputs, "errors": r.errors, "cancelled": r.cancelled,
        "undoable": !r.undo.is_empty() && !r.undone, "undone": r.undone,
    })
}

/// Standard base64 (for the preview PNG), to avoid a dependency for 20 lines.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

impl CastorMcp {
    fn tool(&self, args: Option<&JsonObject>) -> Reply<Arc<dyn Tool>> {
        let id = str_arg(args, "id").ok_or_else(|| err("missing 'id'"))?;
        self.reg.get(id).ok_or_else(|| err(format!("unknown tool '{id}' — call list_tools")))
    }

    fn list(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let fam = str_arg(args, "family");
        let q = str_arg(args, "query").unwrap_or("").trim();
        let tools: Vec<Arc<dyn Tool>> = if q.is_empty() { self.reg.all().to_vec() } else { self.reg.search(q).into_iter().map(|(t, _)| t).collect() };
        let v: Vec<Json> = tools
            .iter()
            .filter(|t| fam.is_none_or(|f| t.meta().family.key() == f))
            .map(|t| {
                let m = t.meta();
                json!({
                    "id": m.id, "name": m.name(), "description": m.desc(), "family": m.family.key(),
                    "output": m.output, "status": format!("{:?}", m.status).to_lowercase(),
                    "needs_input_files": m.inputs.min > 0, "in_place": m.in_place,
                    "unavailable": t.unavailable(),
                })
            })
            .collect();
        ok_json(json!(v))
    }

    fn describe(&self, args: Option<&JsonObject>) -> Reply<CallToolResponse> {
        let t = self.tool(args)?;
        let m = t.meta();
        let kinds: Vec<String> = m.inputs.kinds.iter().map(|k| format!("{k:?}").to_lowercase()).collect();
        Ok(ok_json(json!({
            "id": m.id, "name": m.name(), "description": m.desc(), "family": m.family.key(),
            "status": format!("{:?}", m.status).to_lowercase(),
            "inputs": {"kinds": kinds, "extensions": m.inputs.extensions, "min": m.inputs.min, "max": m.inputs.max, "folders": m.inputs.folders},
            "output": m.output, "in_place": m.in_place,
            "requires": m.requires.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>(),
            "unavailable": t.unavailable(),
            "downloads_needed_with_defaults": t.downloads(&Values::defaults(&t.options())).iter().filter(|a| !a.is_present()).map(|a| json!({"id": a.id, "size": a.size, "license": a.license})).collect::<Vec<_>>(),
            "options": options_schema(t.as_ref()),
        })))
    }

    fn values(&self, t: &dyn Tool, args: Option<&JsonObject>) -> Reply<Values> {
        let mut values = parse_values(t, args.and_then(|a| a.get("values")).and_then(Json::as_object)).map_err(err)?;
        absolutize_paths(t, &mut values).map_err(err)?;
        Ok(values)
    }

    fn preview(&self, args: Option<&JsonObject>) -> Reply<CallToolResponse> {
        let t = self.tool(args)?;
        let id = t.meta().id;
        let values = self.values(t.as_ref(), args)?;
        let input = files_arg(args)?.into_iter().find(|f| t.accepts(f));
        if input.is_none() && t.meta().inputs.min > 0 {
            return Err(err(format!("preview of {id} needs a file or folder it accepts in 'files'")));
        }
        match t.preview(input.as_ref(), &values, PREVIEW_SIDE) {
            Ok(Some(p)) => {
                let mut content = vec![ContentBlock::image(base64(&p.png), "image/png")];
                if let Some(c) = p.caption {
                    content.push(text(c));
                }
                Ok(CallToolResult::success(content).into())
            }
            Ok(None) => Err(err(format!("{id} has no preview; run it on a copy of the files, or into output_dir, instead"))),
            Err(e) => Err(err(format!("{e:#}"))),
        }
    }

    fn run(&self, args: Option<&JsonObject>, cancel: &CancelToken, progress: &mut dyn FnMut(Progress)) -> Reply<CallToolResponse> {
        let t = self.tool(args)?;
        let id = t.meta().id;
        if t.meta().status == castor_core::ToolStatus::Soon {
            return Err(err(format!("'{id}' is not available yet")));
        }
        if let Some(why) = t.unavailable() {
            return Err(err(why));
        }
        let values = self.values(t.as_ref(), args)?;
        let (usable, skipped) = split_inputs(t.as_ref(), files_arg(args)?);
        check_input_count(t.as_ref(), usable.len(), &skipped)?;
        let output = output_policy(args)?;
        let steps = vec![Step { tool: t, values }];
        check_downloads(&steps, args)?;
        let req = RunRequest { steps, inputs: usable, output, chain_name: None, max_parallel: 0, record: true };
        Ok(self.execute(req, skipped, cancel, progress))
    }

    fn chain(&self, args: Option<&JsonObject>, cancel: &CancelToken, progress: &mut dyn FnMut(Progress)) -> Reply<CallToolResponse> {
        let src = str_arg(args, "toml").ok_or_else(|| err("missing 'toml'"))?;
        let chain = castor_core::chain::Chain::from_toml(src).map_err(|e| err(format!("invalid chain: {e:#}")))?;
        let mut steps = chain.resolve_strict(&self.reg).map_err(|e| err(format!("{e:#}")))?;
        for s in &mut steps {
            absolutize_paths(s.tool.as_ref(), &mut s.values).map_err(err)?;
        }
        let (usable, skipped) = split_inputs(steps[0].tool.as_ref(), files_arg(args)?);
        check_input_count(steps[0].tool.as_ref(), usable.len(), &skipped)?;
        let output = output_policy(args)?;
        check_downloads(&steps, args)?;
        let name = if chain.name.is_empty() { None } else { Some(chain.name.clone()) };
        let req = RunRequest { steps, inputs: usable, output, chain_name: name, max_parallel: 0, record: true };
        Ok(self.execute(req, skipped, cancel, progress))
    }

    fn execute(&self, req: RunRequest, skipped: Vec<PathBuf>, cancel: &CancelToken, progress: &mut dyn FnMut(Progress)) -> CallToolResponse {
        let steps = req.steps.len().max(1) as f64;
        let mut notes: Vec<Json> = Vec::new();
        let (mut step, mut tool_id, mut fracs, mut sent) = (0usize, String::new(), Vec::<f32>::new(), -1.0f64);
        let rec = runner::run_blocking(req, cancel.clone(), |ev| {
            let mut milestone = false;
            match ev {
                RunEvent::StepStarted { step: s, tool_id: t, jobs, .. } => {
                    (step, tool_id, fracs, milestone) = (*s, t.clone(), vec![0.0; jobs.len()], true);
                }
                RunEvent::JobProgress { job, frac, .. } => {
                    if let Some(f) = fracs.get_mut(*job) {
                        *f = f.max(frac.clamp(0.0, 1.0));
                    }
                }
                RunEvent::JobDone { job, outputs, .. } => {
                    for o in outputs {
                        if let Some(n) = &o.note {
                            notes.push(json!({"file": o.path, "note": n}));
                        }
                    }
                    if let Some(f) = fracs.get_mut(*job) {
                        *f = 1.0;
                    }
                    milestone = true;
                }
                RunEvent::JobFailed { job, .. } | RunEvent::JobCancelled { job, .. } => {
                    if let Some(f) = fracs.get_mut(*job) {
                        *f = 1.0;
                    }
                    milestone = true;
                }
                _ => {}
            }
            let within = if fracs.is_empty() { 0.0 } else { fracs.iter().map(|f| f64::from(*f)).sum::<f64>() / fracs.len() as f64 };
            let p = step as f64 + within;
            // Progress only goes up, and at most every 1 % unless a file is done.
            if p > sent && (milestone || p - sent >= 0.01) {
                sent = p;
                let done = fracs.iter().filter(|f| **f >= 1.0).count();
                progress(Progress { progress: p, total: steps, message: format!("{tool_id}: {done}/{} done", fracs.len()) });
            }
        });
        let mut v = record_json(&rec);
        v["notes"] = json!(notes);
        v["skipped"] = json!(skipped);
        if rec.outputs.is_empty() || matches!(rec.status, RunStatus::Failed | RunStatus::Cancelled) {
            return err_json(v);
        }
        ok_json(v)
    }

    fn undo(&self, args: Option<&JsonObject>) -> Reply<CallToolResponse> {
        let run_id = str_arg(args, "run_id").ok_or_else(|| err("missing 'run_id' (see history)"))?;
        let mut r = history::load(usize::MAX)
            .into_iter()
            .find(|r| r.id == run_id)
            .ok_or_else(|| err(format!("no run '{run_id}' in the history")))?;
        if r.undone {
            return Err(err(format!("run {run_id} was already undone")));
        }
        if r.undo.is_empty() {
            return Err(err(format!(
                "run {run_id} changed no files in place: there is nothing to undo (its outputs are new files, which can simply be deleted)"
            )));
        }
        let mut errors = Vec::new();
        // Undo in reverse order, like the app's Runs page.
        for (tool_id, journal) in r.undo.iter().rev() {
            match self.reg.get(tool_id) {
                Some(t) => {
                    if let Err(e) = t.undo(journal) {
                        errors.push(format!("{tool_id}: {e:#}"));
                    }
                }
                None => errors.push(format!("{tool_id}: this run can only be undone from the Castor app (Runs page)")),
            }
        }
        if !errors.is_empty() {
            return Err(err_json(json!({"run_id": r.id, "undone": false, "errors": errors})));
        }
        r.undone = true;
        if let Err(e) = history::update(&r) {
            tracing_warn(&format!("could not update the history: {e:#}"));
        }
        Ok(ok_json(json!({"run_id": r.id, "undone": true})))
    }

    fn history(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let limit = args.and_then(|a| a.get("limit")).and_then(Json::as_u64).unwrap_or(20).clamp(1, 200) as usize;
        ok_json(json!(history::load(limit).iter().map(record_json).collect::<Vec<_>>()))
    }

    /// Dispatch one MCP tool call (blocking), without cancellation or progress.
    pub fn call(&self, name: &str, args: Option<&JsonObject>) -> CallToolResponse {
        self.call_with(name, args, CancelToken::new(), &mut |_| {})
    }

    /// Dispatch one MCP tool call (blocking; runs on a worker thread). `cancel` stops a run;
    /// `progress` receives its progress.
    pub fn call_with(&self, name: &str, args: Option<&JsonObject>, cancel: CancelToken, progress: &mut dyn FnMut(Progress)) -> CallToolResponse {
        let key = self.next_run.fetch_add(1, Ordering::SeqCst);
        self.runs().insert(key, cancel.clone());
        let res = match name {
            "list_tools" => Ok(self.list(args)),
            "describe_tool" => self.describe(args),
            "preview_tool" => self.preview(args),
            "run_tool" => self.run(args, &cancel, progress),
            "run_chain" => self.chain(args, &cancel, progress),
            "undo_run" => self.undo(args),
            "history" => Ok(self.history(args)),
            other => Err(err(format!("unknown MCP tool '{other}'"))),
        };
        self.runs().remove(&key);
        res.unwrap_or_else(|e| e)
    }
}

/// Log to stderr (stdout carries the protocol).
fn tracing_warn(msg: &str) {
    eprintln!("castor-mcp: {msg}");
}

impl ServerHandler for CastorMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("castor", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Castor runs ~100 offline file tools on this computer (PDF, image, video, audio, QR codes and other creations, archives, renaming…). \
                 Nothing is uploaded. Typical flow: list_tools (optionally with a query) → describe_tool → run_tool, with absolute paths. \
                 Results are written next to the input files unless output_dir is given; existing files are never overwritten. \
                 Tools with in_place: true rename, move or re-date the user's files: show preview_tool first, and undo_run reverts a run. \
                 When a run needs a one-time download, the call fails with names and sizes: ask the user, then retry with allow_downloads: true.",
            )
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(mcp_tools()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let me = self.clone();
        let name = request.name.to_string();
        let args = request.arguments.clone();
        let cancel = CancelToken::new();
        // notifications/cancelled from the client stops the run itself, not just the wait.
        let watcher = {
            let (ct, c) = (context.ct.clone(), cancel.clone());
            tokio::spawn(async move {
                ct.cancelled().await;
                c.cancel();
            })
        };
        // Progress notifications, when the client asked for them (progressToken in _meta).
        let (ptx, mut prx) = tokio::sync::mpsc::unbounded_channel::<Progress>();
        let forwarder = context.meta.get_progress_token().map(|token| {
            let peer = context.peer.clone();
            tokio::spawn(async move {
                while let Some(p) = prx.recv().await {
                    let param = ProgressNotificationParam::new(token.clone(), p.progress).with_total(p.total).with_message(p.message);
                    let _ = peer.notify_progress(param).await;
                }
            })
        });
        let res = tokio::task::spawn_blocking(move || {
            me.call_with(&name, args.as_ref(), cancel, &mut |p| {
                let _ = ptx.send(p);
            })
        })
        .await;
        watcher.abort();
        // `ptx` is gone with the worker: the forwarder sends what is left, then ends.
        if let Some(f) = forwarder {
            let _ = f.await;
        }
        res.map_err(|e| McpError::internal_error(format!("tool call crashed: {e}"), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn sentences_do_not_double_full_stops() {
        let mut d = "Mode. Keeps everything.".to_string();
        sentence(&mut d, "Choices: fit");
        assert_eq!(d, "Mode. Keeps everything. Choices: fit");
        let mut d = "Size".to_string();
        sentence(&mut d, "used when preset = custom");
        assert_eq!(d, "Size. used when preset = custom");
    }

    #[test]
    fn json_values() {
        assert_eq!(json_to_value(&json!([1200, 630])).unwrap(), Some(Value::Size(1200, 630)));
        assert_eq!(json_to_value(&json!(["1200", "630"])).unwrap(), Some(Value::Size(1200, 630)));
        assert!(json_to_value(&json!([1.5, 2])).is_err());
        assert!(json_to_value(&json!({"a": 1})).is_err());
        assert_eq!(json_to_value(&json!(null)).unwrap(), None);
        assert_eq!(json_to_value(&json!(2.5)).unwrap(), Some(Value::Float(2.5)));
    }

    #[test]
    fn paths_must_be_absolute() {
        assert!(abs_path("relative/out").is_err());
        assert!(abs_path("~/Documents").unwrap().is_absolute());
        #[cfg(unix)]
        assert!(abs_path("/tmp").is_ok());
        #[cfg(windows)]
        assert!(abs_path("C:\\Users").is_ok());
    }
}
