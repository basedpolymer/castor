//! Runs history: one JSON object per line in `{data_dir}/runs.jsonl`.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::options::Values;
use crate::runner::RunRequest;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Running,
    Complete,
    /// Some jobs failed, some produced output.
    Partial,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    /// Tool id, or "chain" for chains.
    pub tool_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_name: Option<String>,
    /// Tool ids of each step (1 for a plain run).
    pub steps: Vec<String>,
    /// Unix seconds.
    pub started: i64,
    pub duration_secs: f32,
    pub status: RunStatus,
    pub files_in: usize,
    pub files_out: usize,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub inputs: Vec<PathBuf>,
    pub outputs: Vec<PathBuf>,
    /// Values of the first step (secrets removed), to re-run with the same settings.
    pub values: Values,
    pub errors: Vec<String>,
    pub cancelled: usize,
    /// (tool_id, journal) for in-place tools that support undo.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub undo: Vec<(String, serde_json::Value)>,
    #[serde(default)]
    pub undone: bool,
    /// Name of the workspace that was active when the run was made (app runs only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

impl RunRecord {
    pub(crate) fn new(id: &str, req: &RunRequest) -> RunRecord {
        let first = req.steps.first();
        RunRecord {
            id: id.to_string(),
            tool_id: if req.steps.len() == 1 {
                first.map(|s| s.tool.meta().id.to_string()).unwrap_or_default()
            } else {
                "chain".into()
            },
            chain_name: req.chain_name.clone(),
            steps: req.steps.iter().map(|s| s.tool.meta().id.to_string()).collect(),
            started: time::OffsetDateTime::now_utc().unix_timestamp(),
            duration_secs: 0.0,
            status: RunStatus::Running,
            files_in: 0,
            files_out: 0,
            bytes_in: 0,
            bytes_out: 0,
            inputs: req.inputs.iter().map(|f| f.path.clone()).collect(),
            outputs: Vec::new(),
            values: first.map(|s| s.values.without_secrets(&s.tool.options())).unwrap_or_default(),
            errors: Vec::new(),
            cancelled: 0,
            undo: Vec::new(),
            undone: false,
            workspace: None,
        }
    }
}

/// Sortable unique id: `20261106-104512-3f9a`.
pub fn new_run_id() -> String {
    let now = time::OffsetDateTime::now_utc();
    let fmt = time::macros::format_description!("[year][month][day]-[hour][minute][second]");
    let rnd = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos()
        ^ std::process::id())
        & 0xffff;
    format!("{}-{rnd:04x}", now.format(fmt).unwrap_or_default())
}

fn file() -> PathBuf {
    crate::paths::data_dir().join("runs.jsonl")
}

pub fn append(r: &RunRecord) -> crate::Result<()> {
    let p = file();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
    writeln!(f, "{}", serde_json::to_string(r)?)?;
    Ok(())
}

/// Most recent first. Later lines with the same id (e.g. after undo) replace earlier ones.
pub fn load(limit: usize) -> Vec<RunRecord> {
    let Ok(f) = std::fs::File::open(file()) else { return Vec::new() };
    let mut by_id: std::collections::HashMap<String, RunRecord> = Default::default();
    let mut order = Vec::new();
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        if let Ok(r) = serde_json::from_str::<RunRecord>(&line) {
            if !by_id.contains_key(&r.id) {
                order.push(r.id.clone());
            }
            by_id.insert(r.id.clone(), r);
        }
    }
    order.iter().rev().filter_map(|id| by_id.remove(id)).take(limit).collect()
}

/// Rewrite a record (e.g. mark as undone): appended, `load` keeps the last version.
pub fn update(r: &RunRecord) -> crate::Result<()> {
    append(r)
}

/// Forget every run.
pub fn clear() -> crate::Result<()> {
    let p = file();
    if p.exists() {
        std::fs::remove_file(p)?;
    }
    Ok(())
}
