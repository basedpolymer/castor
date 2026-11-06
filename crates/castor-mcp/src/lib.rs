//! Castor as an MCP server (Model Context Protocol, stdio transport, official `rmcp` SDK).
//!
//! Five MCP tools expose every Castor tool to AI agents:
//! - `list_tools` — ids, names, descriptions, families, inputs and outputs (optional filter);
//! - `describe_tool` — a JSON Schema of a tool's options, derived from its `OptionSpec`s;
//! - `run_tool` — run a tool on files with option values, returns outputs and notes;
//! - `run_chain` — run a chain (TOML, same format as the app and `castor-cli chain`);
//! - `history` — recent runs.
//!
//! Runs go through `castor_core::runner`, exactly like the app and the CLI (history included).

use std::path::PathBuf;
use std::sync::Arc;

use castor_core::options::{OptionKind, OptionSpec, Value, Values};
use castor_core::runner::{self, OutputPolicy, RunRequest, Step};
use castor_core::{CancelToken, FileInfo, Registry, Tool};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value as Json, json};

/// The server state: the registry of every Castor tool.
#[derive(Clone)]
pub struct CastorMcp {
    reg: Arc<Registry>,
}

impl CastorMcp {
    pub fn new(reg: Registry) -> Self {
        CastorMcp { reg: Arc::new(reg) }
    }
}

/// Serve on stdin/stdout until the client disconnects. Used by `castor-mcp` and
/// (on request to the coordinator) by `castor-cli mcp`.
pub async fn serve_stdio() -> anyhow::Result<()> {
    let lang = std::env::var("CASTOR_LANG").unwrap_or_else(|_| "en".into());
    castor_core::i18n::set_language(&lang);
    let server = CastorMcp::new(castor_tools::registry());
    let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| anyhow::anyhow!("MCP initialisation failed: {e}"))?;
    running.waiting().await?;
    Ok(())
}

/// Blocking entry point (creates its own Tokio runtime).
pub fn run_stdio() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread().enable_all().build()?.block_on(serve_stdio())
}

fn obj(v: Json) -> Arc<JsonObject> {
    match v {
        Json::Object(m) => Arc::new(m),
        _ => Arc::new(JsonObject::new()),
    }
}

/// The five MCP tools.
pub fn mcp_tools() -> Vec<rmcp::model::Tool> {
    vec![
        rmcp::model::Tool::new(
            "list_tools",
            "List Castor's offline file tools (PDF, image, video, audio, create, files). Returns id, name, description, family, accepted inputs and output formats. Optional: family (pdf|image|video|audio|create|files) and query (keywords, any language).",
            obj(json!({"type": "object", "properties": {
                "family": {"type": "string", "enum": ["pdf", "image", "video", "audio", "create", "files"]},
                "query": {"type": "string", "description": "Search words, e.g. 'compress pdf' or 'qr code'"}
            }})),
        ),
        rmcp::model::Tool::new(
            "describe_tool",
            "Describe one Castor tool: what it does, which files it accepts, and the JSON Schema of its options (with defaults, ranges and choices). Call this before run_tool.",
            obj(json!({"type": "object", "required": ["id"], "properties": {"id": {"type": "string", "description": "Tool id, e.g. pdf.compress"}}})),
        ),
        rmcp::model::Tool::new(
            "run_tool",
            "Run a Castor tool locally on files (absolute paths; folders are expanded). 'values' sets options (see describe_tool; anything left out keeps its default). Results go next to the inputs, or into output_dir. Returns the output files, notes and errors. Tools marked in_place (rename, dates, sort…) change the input files and can be undone from the Castor app's Runs page.",
            obj(json!({"type": "object", "required": ["id"], "properties": {
                "id": {"type": "string"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "Absolute paths of input files or folders"},
                "values": {"type": "object", "description": "Option values by key"},
                "output_dir": {"type": "string", "description": "Folder for the results (created if needed)"}
            }})),
        ),
        rmcp::model::Tool::new(
            "run_chain",
            "Run a chain of Castor tools (outputs of step k feed step k+1). 'toml' uses the chain format: name = \"…\" then [[steps]] tool = \"image.resize\" values = { mode = \"fit\", size = [2048, 2048] }.",
            obj(json!({"type": "object", "required": ["toml"], "properties": {
                "toml": {"type": "string"},
                "files": {"type": "array", "items": {"type": "string"}},
                "output_dir": {"type": "string"}
            }})),
        ),
        rmcp::model::Tool::new(
            "history",
            "Recent Castor runs (most recent first): tool, status, inputs, outputs, sizes, errors.",
            obj(json!({"type": "object", "properties": {"limit": {"type": "integer", "minimum": 1, "maximum": 200, "default": 20}}})),
        ),
    ]
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
    let label = castor_core::tr(&o.label_key(tool_id));
    let help_key = format!("{}.help", o.label_key(tool_id));
    let mut desc = label;
    if castor_core::i18n::has(&help_key) {
        desc.push_str(". ");
        desc.push_str(&castor_core::tr(&help_key));
    }
    if let Some(c) = &o.visible_if {
        let cond = match c {
            castor_core::Cond::Eq(k, v) => format!("{k} = {v}"),
            castor_core::Cond::Ne(k, v) => format!("{k} ≠ {v}"),
            castor_core::Cond::True(k) => format!("{k} is true"),
            castor_core::Cond::False(k) => format!("{k} is false"),
        };
        desc.push_str(&format!(" (used when {cond})"));
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
            desc.push_str(&format!(". Choices: {}", labels.join("; ")));
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
            desc.push_str(if *folder { " (absolute folder path)" } else { " (absolute file path)" });
            if !filters.is_empty() {
                desc.push_str(&format!(" [{}]", filters.join(", ")));
            }
            json!({"type": "string"})
        }
        OptionKind::PageRange => json!({"type": "string", "description": "e.g. 1-3, 5, 8- (empty = all)"}),
        OptionKind::Anchor => json!({"type": "string", "enum": castor_core::options::Anchor::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>()}),
        OptionKind::Size => json!({"type": "string", "pattern": "^[0-9]+x[0-9]+$"}),
        OptionKind::Language { multi, list } => {
            let codes: Vec<&str> = list.iter().map(|(c, _)| *c).collect();
            if *multi {
                desc.push_str(" (several codes joined with +)");
                json!({"type": "string"})
            } else {
                json!({"type": "string", "enum": codes})
            }
        }
    };
    s["description"] = json!(desc);
    if !default.is_null() {
        s["default"] = default;
    }
    s
}

/// Convert JSON option values to Castor values, checked against the schema.
pub fn parse_values(tool: &dyn Tool, given: Option<&JsonObject>) -> Result<Values, String> {
    let specs = tool.options();
    let mut values = Values::defaults(&specs);
    let Some(given) = given else { return Ok(values) };
    for (k, v) in given {
        let spec = specs
            .iter()
            .find(|s| s.key == k)
            .ok_or_else(|| format!("'{}' has no option '{k}' (options: {})", tool.meta().id, specs.iter().map(|s| s.key).collect::<Vec<_>>().join(", ")))?;
        let text = match v {
            Json::String(s) => s.clone(),
            Json::Bool(b) => b.to_string(),
            Json::Number(n) => n.to_string(),
            Json::Array(a) if a.len() == 2 && matches!(spec.kind, OptionKind::Size) => format!("{}x{}", a[0], a[1]),
            Json::Null => continue,
            other => other.to_string(),
        };
        let val = match (&spec.kind, v) {
            (OptionKind::Color | OptionKind::Text { .. } | OptionKind::Password | OptionKind::Font | OptionKind::PageRange, _) => Value::Text(text),
            _ => spec.parse(&text).map_err(|e| format!("option '{k}': {e:#}"))?,
        };
        values.set(k, val);
    }
    Ok(values)
}

fn text(s: impl Into<String>) -> ContentBlock {
    ContentBlock::text(s)
}

fn ok_json(v: Json) -> CallToolResponse {
    CallToolResult::success(vec![text(serde_json::to_string_pretty(&v).unwrap_or_default())]).into()
}

fn err(msg: impl Into<String>) -> CallToolResponse {
    CallToolResult::error(vec![text(msg)]).into()
}

fn str_arg<'a>(args: Option<&'a JsonObject>, k: &str) -> Option<&'a str> {
    args.and_then(|a| a.get(k)).and_then(Json::as_str)
}

fn files_arg(args: Option<&JsonObject>) -> Result<Vec<FileInfo>, String> {
    let paths: Vec<PathBuf> = args
        .and_then(|a| a.get("files"))
        .and_then(Json::as_array)
        .map(|a| a.iter().filter_map(Json::as_str).map(PathBuf::from).collect())
        .unwrap_or_default();
    for p in &paths {
        if !p.exists() {
            return Err(format!("{} does not exist", p.display()));
        }
    }
    Ok(castor_core::files::expand(&paths, true))
}

fn output_policy(args: Option<&JsonObject>) -> Result<OutputPolicy, String> {
    Ok(match str_arg(args, "output_dir").filter(|s| !s.trim().is_empty()) {
        Some(d) => {
            let p = PathBuf::from(d);
            std::fs::create_dir_all(&p).map_err(|e| format!("output_dir {}: {e}", p.display()))?;
            OutputPolicy::Folder(p)
        }
        None => OutputPolicy::NextToInput { fallback: castor_core::paths::default_output_dir() },
    })
}

fn record_json(r: &castor_core::history::RunRecord) -> Json {
    json!({
        "run_id": r.id, "status": r.status, "tool": r.tool_id, "steps": r.steps,
        "files_in": r.files_in, "files_out": r.files_out,
        "bytes_in": r.bytes_in, "bytes_out": r.bytes_out,
        "duration_secs": r.duration_secs,
        "outputs": r.outputs, "errors": r.errors, "cancelled": r.cancelled,
        "undoable": !r.undo.is_empty(),
    })
}

impl CastorMcp {
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

    fn describe(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let Some(id) = str_arg(args, "id") else { return err("missing 'id'") };
        let Some(t) = self.reg.get(id) else { return err(format!("unknown tool '{id}' — call list_tools")) };
        let m = t.meta();
        let kinds: Vec<String> = m.inputs.kinds.iter().map(|k| format!("{k:?}").to_lowercase()).collect();
        ok_json(json!({
            "id": m.id, "name": m.name(), "description": m.desc(), "family": m.family.key(),
            "status": format!("{:?}", m.status).to_lowercase(),
            "inputs": {"kinds": kinds, "extensions": m.inputs.extensions, "min": m.inputs.min, "max": m.inputs.max, "folders": m.inputs.folders},
            "output": m.output, "in_place": m.in_place,
            "requires": m.requires.iter().map(|r| format!("{r:?}")).collect::<Vec<_>>(),
            "unavailable": t.unavailable(),
            "downloads_needed_with_defaults": t.downloads(&Values::defaults(&t.options())).iter().filter(|a| !a.is_present()).map(|a| json!({"id": a.id, "size": a.size, "license": a.license})).collect::<Vec<_>>(),
            "options": options_schema(t.as_ref()),
        }))
    }

    fn run(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let Some(id) = str_arg(args, "id") else { return err("missing 'id'") };
        let Some(t) = self.reg.get(id) else { return err(format!("unknown tool '{id}' — call list_tools")) };
        if t.meta().status == castor_core::ToolStatus::Soon {
            return err(format!("'{id}' is not available yet"));
        }
        if let Some(why) = t.unavailable() {
            return err(why);
        }
        let values = match parse_values(t.as_ref(), args.and_then(|a| a.get("values")).and_then(Json::as_object)) {
            Ok(v) => v,
            Err(e) => return err(e),
        };
        let inputs = match files_arg(args) {
            Ok(f) => f,
            Err(e) => return err(e),
        };
        let usable: Vec<FileInfo> = inputs.iter().filter(|f| t.accepts(f)).cloned().collect();
        if usable.len() < t.meta().inputs.min {
            return err(format!("{id} needs at least {} matching file(s); {} of {} given are usable", t.meta().inputs.min, usable.len(), inputs.len()));
        }
        let output = match output_policy(args) {
            Ok(o) => o,
            Err(e) => return err(e),
        };
        let req = RunRequest { steps: vec![Step { tool: t, values }], inputs: usable, output, chain_name: None, max_parallel: 0, record: true };
        self.execute(req)
    }

    fn chain(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let Some(src) = str_arg(args, "toml") else { return err("missing 'toml'") };
        let chain = match castor_core::chain::Chain::from_toml(src) {
            Ok(c) => c,
            Err(e) => return err(format!("invalid chain: {e:#}")),
        };
        let steps = match chain.resolve(&self.reg) {
            Ok(s) => s,
            Err(e) => return err(format!("{e:#}")),
        };
        let inputs = match files_arg(args) {
            Ok(f) => f,
            Err(e) => return err(e),
        };
        let output = match output_policy(args) {
            Ok(o) => o,
            Err(e) => return err(e),
        };
        let name = if chain.name.is_empty() { None } else { Some(chain.name.clone()) };
        self.execute(RunRequest { steps, inputs, output, chain_name: name, max_parallel: 0, record: true })
    }

    fn execute(&self, req: RunRequest) -> CallToolResponse {
        let mut notes: Vec<Json> = Vec::new();
        let rec = runner::run_blocking(req, CancelToken::new(), |ev| {
            if let runner::RunEvent::JobDone { outputs, .. } = ev {
                for o in outputs {
                    if let Some(n) = &o.note {
                        notes.push(json!({"file": o.path, "note": n}));
                    }
                }
            }
        });
        let mut v = record_json(&rec);
        v["notes"] = json!(notes);
        if rec.outputs.is_empty() {
            return CallToolResult::error(vec![text(serde_json::to_string_pretty(&v).unwrap_or_default())]).into();
        }
        ok_json(v)
    }

    fn history(&self, args: Option<&JsonObject>) -> CallToolResponse {
        let limit = args.and_then(|a| a.get("limit")).and_then(Json::as_u64).unwrap_or(20).clamp(1, 200) as usize;
        ok_json(json!(castor_core::history::load(limit).iter().map(record_json).collect::<Vec<_>>()))
    }

    /// Dispatch one MCP tool call (blocking; runs on a worker thread).
    pub fn call(&self, name: &str, args: Option<&JsonObject>) -> CallToolResponse {
        match name {
            "list_tools" => self.list(args),
            "describe_tool" => self.describe(args),
            "run_tool" => self.run(args),
            "run_chain" => self.chain(args),
            "history" => self.history(args),
            other => err(format!("unknown MCP tool '{other}'")),
        }
    }
}

impl ServerHandler for CastorMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("castor", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Castor runs ~100 offline file tools on this computer (PDF, image, video, audio, QR codes and other creations, archives, renaming…). \
                 Nothing is uploaded. Typical flow: list_tools (optionally with a query) → describe_tool → run_tool with absolute paths. \
                 Results are written next to the input files unless output_dir is given; existing files are never overwritten.",
            )
    }

    async fn list_tools(&self, _request: Option<PaginatedRequestParams>, _context: RequestContext<RoleServer>) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(mcp_tools()))
    }

    async fn call_tool(&self, request: CallToolRequestParams, _context: RequestContext<RoleServer>) -> Result<CallToolResponse, McpError> {
        let me = self.clone();
        let name = request.name.to_string();
        let args = request.arguments.clone();
        tokio::task::spawn_blocking(move || me.call(&name, args.as_ref()))
            .await
            .map_err(|e| McpError::internal_error(format!("tool call crashed: {e}"), None))
    }
}
