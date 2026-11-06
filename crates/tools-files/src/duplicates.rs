//! `files.duplicates`: exact duplicate files — grouped by size, then a partial hash (first and
//! last 64 KB), then a full BLAKE3 hash. Writes a report (CSV / TXT / JSON) and can move the
//! extra copies into a "Duplicates" folder (undoable). Never deletes anything.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args, units};
use rayon::prelude::*;

use crate::common::{self, Cell, MoveJournal};

pub struct Duplicates {
    meta: ToolMeta,
}

impl Duplicates {
    pub fn new() -> Self {
        Duplicates {
            meta: common::meta(
                "files.duplicates",
                InputSpec::kinds(common::ANY).folders(),
                "CSV · TXT · JSON",
                &["duplicates", "dedupe", "same files", "copies", "clean up", "free space", "hash"],
                "duplicates",
                true,
            ),
        }
    }
}

impl Tool for Duplicates {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("action", ["report", "move"].map(Choice::key).to_vec(), "report"),
            OptionSpec::choice("keep", ["oldest", "newest", "shortest", "first"].map(Choice::key).to_vec(), "oldest"),
            OptionSpec::choice("report", vec![Choice::lit("csv", "CSV"), Choice::lit("txt", "TXT"), Choice::lit("json", "JSON")], "csv"),
            OptionSpec::int("min_size", 0, 1_000_000, 1).unit("KB").advanced(),
            OptionSpec::toggle("skip_empty", true).advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        let files = common::expand_inputs(inputs, true);
        anyhow::ensure!(!files.is_empty(), "{}", tr("files.err.no_files"));
        Ok(vec![Job::many(files)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input.filter(|f| f.is_dir) else { return Ok(None) };
        let files = common::expand_inputs(std::slice::from_ref(f), true);
        if files.len() > 5000 {
            return Ok(None); // too slow for a live preview; the run shows progress
        }
        let groups = find(&files, v, None)?;
        let rows = rows(&groups, v);
        let wasted: u64 = groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
        let png = common::table_png(
            &tr_args("files.duplicates.caption", &[("groups", &groups.len().to_string()), ("wasted", &units::bytes(wasted))]),
            &[&tr("files.duplicates.col.keep"), &tr("files.duplicates.col.copy"), &tr("files.sort.col.size")],
            &rows,
            max_side,
            0,
        )?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let groups = find(&job.inputs, v, Some(cx))?;
        let wasted: u64 = groups.iter().map(|g| g.size * (g.files.len() as u64 - 1)).sum();
        let extra: usize = groups.iter().map(|g| g.files.len() - 1).sum();
        let report = cx.output_named(&format!("duplicates.{}", v.str("report")));
        std::fs::write(&report, report_text(&groups, v))?;
        let note = if groups.is_empty() {
            tr("files.duplicates.none")
        } else {
            tr_args("files.duplicates.note", &[("groups", &groups.len().to_string()), ("copies", &extra.to_string()), ("wasted", &units::bytes(wasted))])
        };
        let mut outs = vec![Output::new(&report).with_note(note)];
        if v.str("action") == "move" && !groups.is_empty() {
            // The "Duplicates" folder sits next to the report (in the output folder).
            let dest = report.parent().unwrap_or(Path::new(".")).join(tr("files.duplicates.folder"));
            let mut moves = Vec::new();
            let mut taken = std::collections::HashSet::new();
            for g in &groups {
                for p in g.files.iter().skip(1) {
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    let mut to = dest.join(&name);
                    let mut n = 2;
                    while to.exists() || !taken.insert(to.to_string_lossy().to_lowercase()) {
                        let (stem, ext) = split_name(&name);
                        to = dest.join(format!("{stem} ({n}){ext}"));
                        n += 1;
                    }
                    moves.push((p.clone(), to));
                }
            }
            let journal = common::apply_moves(&moves, cx)?;
            outs[0].undo = Some(serde_json::to_value(&journal)?);
            outs.extend(journal.moves.iter().map(|(_, b)| Output::new(b)));
        }
        Ok(outs)
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let j: MoveJournal = serde_json::from_value(journal.clone())?;
        common::undo_moves(&j)
    }
}

fn split_name(n: &str) -> (String, String) {
    match n.rfind('.') {
        Some(i) if i > 0 => (n[..i].to_string(), n[i..].to_string()),
        _ => (n.to_string(), String::new()),
    }
}

pub struct Group {
    pub size: u64,
    pub hash: String,
    /// The file to keep first, then the copies.
    pub files: Vec<PathBuf>,
}

fn partial_hash(p: &Path, size: u64) -> Option<[u8; 32]> {
    const CHUNK: u64 = 64 * 1024;
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK.min(size) as usize];
    f.read_exact(&mut buf).ok()?;
    h.update(&buf);
    if size > CHUNK {
        f.seek(SeekFrom::Start(size.saturating_sub(CHUNK).max(CHUNK))).ok()?;
        let mut tail = Vec::new();
        f.take(CHUNK).read_to_end(&mut tail).ok()?;
        h.update(&tail);
    }
    Some(*h.finalize().as_bytes())
}

fn full_hash(p: &Path, cx: Option<&RunCx>) -> Option<[u8; 32]> {
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if cx.is_some_and(|c| c.is_cancelled()) {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(*h.finalize().as_bytes())
}

/// Find groups of identical files.
pub fn find(files: &[FileInfo], v: &Values, cx: Option<&RunCx>) -> Result<Vec<Group>> {
    let min = (v.int("min_size").max(0) as u64) * 1000;
    let skip_empty = v.bool("skip_empty") || min > 0;
    // 1. Same size.
    let mut by_size: HashMap<u64, Vec<&FileInfo>> = HashMap::new();
    for f in files.iter().filter(|f| !f.is_dir) {
        if (f.size == 0 && skip_empty) || f.size < min {
            continue;
        }
        by_size.entry(f.size).or_default().push(f);
    }
    let candidates: Vec<(u64, Vec<&FileInfo>)> = by_size.into_iter().filter(|(_, v)| v.len() > 1).collect();
    if let Some(cx) = cx {
        cx.message(tr_args("files.duplicates.msg.candidates", &[("count", &candidates.iter().map(|c| c.1.len()).sum::<usize>().to_string())]));
    }
    let total_bytes: u64 = candidates.iter().map(|(s, v)| s * v.len() as u64).sum::<u64>().max(1);
    let done = std::sync::atomic::AtomicU64::new(0);
    // 2. Partial hash, 3. full hash — in parallel, per size group.
    let mut groups: Vec<Group> = candidates
        .par_iter()
        .flat_map_iter(|(size, list)| {
            let mut by_part: HashMap<[u8; 32], Vec<&FileInfo>> = HashMap::new();
            for f in list {
                if let Some(h) = partial_hash(&f.path, *size) {
                    by_part.entry(h).or_default().push(f);
                }
            }
            let mut out = Vec::new();
            for (_, same) in by_part.into_iter().filter(|(_, v)| v.len() > 1) {
                let mut by_full: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
                for f in same {
                    let h = if *size <= 128 * 1024 { partial_hash(&f.path, *size) } else { full_hash(&f.path, cx) };
                    if let Some(h) = h {
                        by_full.entry(h).or_default().push(f.path.clone());
                    }
                    let d = done.fetch_add(*size, std::sync::atomic::Ordering::Relaxed) + size;
                    if let Some(cx) = cx {
                        cx.progress(d as f32 / total_bytes as f32);
                    }
                }
                for (h, files) in by_full.into_iter().filter(|(_, v)| v.len() > 1) {
                    out.push(Group { size: *size, hash: h.iter().map(|b| format!("{b:02x}")).collect(), files });
                }
            }
            out
        })
        .collect();
    if let Some(cx) = cx {
        cx.check_cancel()?;
    }
    for g in &mut groups {
        order(&mut g.files, v.str("keep"));
    }
    groups.sort_by(|a, b| (b.size * b.files.len() as u64).cmp(&(a.size * a.files.len() as u64)));
    Ok(groups)
}

/// Put the file to keep first.
fn order(files: &mut [PathBuf], keep: &str) {
    let mtime = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match keep {
        "newest" => files.sort_by_key(|p| std::cmp::Reverse(mtime(p))),
        "shortest" => files.sort_by_key(|p| (p.components().count(), p.to_string_lossy().len())),
        "first" => files.sort_by(|a, b| castor_core::files::natural_cmp(&a.to_string_lossy(), &b.to_string_lossy())),
        _ => files.sort_by_key(mtime),
    }
}

fn rows(groups: &[Group], _v: &Values) -> Vec<Vec<Cell>> {
    let mut rows = Vec::new();
    for g in groups {
        for p in g.files.iter().skip(1) {
            rows.push(vec![
                (g.files[0].to_string_lossy().into_owned(), Some(common::GOOD)),
                (p.to_string_lossy().into_owned(), None),
                (units::bytes(g.size), None),
            ]);
        }
    }
    rows
}

fn report_text(groups: &[Group], v: &Values) -> String {
    match v.str("report") {
        "json" => serde_json::to_string_pretty(
            &groups
                .iter()
                .map(|g| serde_json::json!({"size": g.size, "blake3": g.hash, "keep": g.files[0], "copies": &g.files[1..]}))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default(),
        "txt" => {
            let mut s = String::new();
            for (i, g) in groups.iter().enumerate() {
                s.push_str(&format!("{}. {} × {} ({})\n", i + 1, g.files.len(), units::bytes(g.size), &g.hash[..16]));
                for (j, p) in g.files.iter().enumerate() {
                    s.push_str(&format!("   {} {}\n", if j == 0 { "keep" } else { "copy" }, p.display()));
                }
                s.push('\n');
            }
            s
        }
        _ => {
            let mut s = String::from("group,role,size_bytes,blake3,path\n");
            for (i, g) in groups.iter().enumerate() {
                for (j, p) in g.files.iter().enumerate() {
                    let path = p.to_string_lossy().replace('"', "\"\"");
                    s.push_str(&format!("{},{},{},{},\"{path}\"\n", i + 1, if j == 0 { "keep" } else { "copy" }, g.size, g.hash));
                }
            }
            s
        }
    }
}
