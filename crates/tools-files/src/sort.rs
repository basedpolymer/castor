//! `files.sort`: route files into subfolders — by type, year, month, camera, extension, size,
//! or your own rules ("*.pdf => Documents", "IMG_* => Photos/{year}"). Dry-run counts in the
//! preview; every move is undoable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common::{self, Cell, MoveJournal};

pub struct Sort {
    meta: ToolMeta,
}

impl Sort {
    pub fn new() -> Self {
        Sort {
            meta: common::meta(
                "files.sort",
                InputSpec::kinds(common::ANY).folders(),
                "—",
                &["sort", "organize", "folders", "route", "tidy", "downloads", "photos by date", "camera", "rules"],
                "",
                true,
            ),
        }
    }
}

impl Tool for Sort {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("by", ["type", "year", "month", "day", "camera", "extension", "size", "rules"].map(Choice::key).to_vec(), "type"),
            OptionSpec::choice("date_source", ["capture", "modified", "created"].map(Choice::key).to_vec(), "capture")
                .when(Cond::Ne("by", "type")),
            OptionSpec::multiline("rules", "*.pdf, *.docx => Documents\n*.jpg, *.png, *.heic => Photos/{year}\n* => Other").when(Cond::Eq("by", "rules")),
            OptionSpec::folder("destination"),
            OptionSpec::toggle("recursive", false).advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], v: &Values) -> Result<Vec<Job>> {
        let mut files = Vec::new();
        for i in inputs {
            files.extend(common::expand_inputs(std::slice::from_ref(i), v.bool("recursive")));
        }
        anyhow::ensure!(!files.is_empty(), "{}", tr("files.err.no_files"));
        // Keep the dropped folders as the base: the first input that is a folder.
        let mut job = Job::many(files);
        if let Some(d) = inputs.iter().find(|f| f.is_dir) {
            job.inputs.insert(0, d.clone());
        }
        Ok(vec![job])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let files = if f.is_dir { common::expand_inputs(std::slice::from_ref(f), v.bool("recursive")) } else { vec![f.clone()] };
        let base = base_dir(Some(f), v);
        let moves = plan_moves(&files, &base, v)?;
        let mut counts: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for (from, to) in &moves {
            let rel = to.parent().and_then(|p| p.strip_prefix(&base).ok()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
            let e = counts.entry(rel).or_default();
            e.0 += 1;
            e.1 += std::fs::metadata(from).map(|m| m.len()).unwrap_or(0);
        }
        let rows: Vec<Vec<Cell>> = counts
            .iter()
            .map(|(k, (n, b))| vec![(format!("{k}/"), None), (n.to_string(), None), (castor_core::units::bytes(*b), None)])
            .collect();
        let png = common::table_png(
            &tr_args("files.sort.caption", &[("count", &moves.len().to_string()), ("folders", &counts.len().to_string())]),
            &[&tr("files.sort.col.folder"), &tr("files.sort.col.files"), &tr("files.sort.col.size")],
            &rows,
            max_side,
            0,
        )?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let base_input = job.inputs.first().filter(|f| f.is_dir);
        let base = base_dir(base_input.or(job.inputs.first()), v);
        let files: Vec<FileInfo> = job.inputs.iter().filter(|f| !f.is_dir).cloned().collect();
        let moves = plan_moves(&files, &base, v)?;
        anyhow::ensure!(!moves.is_empty(), "{}", tr("files.sort.err.nothing"));
        let journal = common::apply_moves(&moves, cx)?;
        let mut outs: Vec<Output> = journal.moves.iter().map(|(_, b)| Output::new(b)).collect();
        if let Some(first) = outs.first_mut() {
            first.undo = Some(serde_json::to_value(&journal)?);
            first.note = Some(tr_args("files.sort.note", &[("count", &journal.moves.len().to_string())]));
        }
        Ok(outs)
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let j: MoveJournal = serde_json::from_value(journal.clone())?;
        common::undo_moves(&j)
    }
}

fn base_dir(first: Option<&FileInfo>, v: &Values) -> PathBuf {
    if let Some(d) = v.path("destination") {
        return d.to_path_buf();
    }
    match first {
        Some(f) if f.is_dir => f.path.clone(),
        Some(f) => f.path.parent().map(Path::to_path_buf).unwrap_or_default(),
        None => PathBuf::from("."),
    }
}

fn kind_folder(f: &FileInfo) -> String {
    let k = match f.kind {
        FileKind::Image | FileKind::Raw | FileKind::Svg => "images",
        FileKind::Video => "videos",
        FileKind::Audio => "audio",
        FileKind::Pdf | FileKind::Office | FileKind::Ebook | FileKind::Markdown | FileKind::Text | FileKind::Table | FileKind::Html => "documents",
        FileKind::Archive => "archives",
        FileKind::Font => "fonts",
        FileKind::Subtitles => "subtitles",
        _ => "other",
    };
    tr(&format!("files.sort.folder.{k}"))
}

fn date_of(f: &FileInfo, v: &Values) -> Option<std::time::SystemTime> {
    let (c, m) = common::times(&f.path);
    match v.str("date_source") {
        "modified" => m,
        "created" => c,
        _ => common::capture_date(&f.path).or(m),
    }
}

/// Destination folder (relative) for one file, or None to leave it where it is.
fn folder_for(f: &FileInfo, v: &Values, rules: &[(String, String)]) -> Option<String> {
    let date = || date_of(f, v);
    Some(match v.str("by") {
        "year" => common::fmt_time(date()?, "%Y"),
        "month" => common::fmt_time(date()?, "%Y/%Y-%m"),
        "day" => common::fmt_time(date()?, "%Y/%Y-%m-%d"),
        "camera" => castor_core::job::sanitize_file_name(&common::exif_camera(&f.path).unwrap_or_else(|| tr("files.sort.folder.no_camera"))),
        "extension" => {
            let e = f.extension().to_uppercase();
            if e.is_empty() { tr("files.sort.folder.no_extension") } else { e }
        }
        "size" => {
            let s = f.size;
            tr(if s < 1_000_000 {
                "files.sort.folder.small"
            } else if s < 100_000_000 {
                "files.sort.folder.medium"
            } else {
                "files.sort.folder.large"
            })
        }
        "rules" => {
            let name = f.name();
            let (_, dest) = rules.iter().find(|(pat, _)| common::wildcard_any(pat, &name))?;
            let d = date();
            dest.replace("{year}", &d.map(|t| common::fmt_time(t, "%Y")).unwrap_or_default())
                .replace("{month}", &d.map(|t| common::fmt_time(t, "%m")).unwrap_or_default())
                .replace("{ext}", &f.extension())
                .replace("{type}", &kind_folder(f))
                .replace("{camera}", &common::exif_camera(&f.path).unwrap_or_default())
        }
        _ => kind_folder(f),
    })
}

/// "pattern, pattern => folder" lines.
pub fn parse_rules(s: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (i, line) in s.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (pat, dest) = line
            .split_once("=>")
            .or_else(|| line.split_once("->"))
            .ok_or_else(|| anyhow::anyhow!("{}", tr_args("files.sort.err.rule", &[("line", &(i + 1).to_string())])))?;
        let dest = dest.trim().trim_matches(['/', '\\']).to_string();
        anyhow::ensure!(!dest.contains(".."), "{}", tr_args("files.sort.err.rule", &[("line", &(i + 1).to_string())]));
        out.push((pat.trim().to_string(), dest));
    }
    Ok(out)
}

pub fn plan_moves(files: &[FileInfo], base: &Path, v: &Values) -> Result<Vec<(PathBuf, PathBuf)>> {
    let rules = if v.str("by") == "rules" { parse_rules(v.str("rules"))? } else { Vec::new() };
    let mut out = Vec::new();
    let mut taken = std::collections::HashSet::new();
    for f in files.iter().filter(|f| !f.is_dir) {
        let Some(rel) = folder_for(f, v, &rules) else { continue };
        let rel: PathBuf = rel.split(['/', '\\']).filter(|s| !s.is_empty()).map(castor_core::job::sanitize_file_name).collect();
        let dir = base.join(rel);
        if f.path.parent() == Some(dir.as_path()) {
            continue; // already there
        }
        // Unique name in the destination (existing files and other planned moves).
        let mut to = dir.join(f.name());
        let mut n = 2;
        while to.exists() || !taken.insert(to.to_string_lossy().to_lowercase()) {
            to = dir.join(format!("{} ({n}){}", f.stem(), if f.extension().is_empty() { String::new() } else { format!(".{}", f.path.extension().unwrap_or_default().to_string_lossy()) }));
            n += 1;
        }
        out.push((f.path.clone(), to));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rules() {
        let r = parse_rules("*.pdf, *.docx => Documents/\n# comment\nIMG_* -> Photos/{year}").unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].1, "Documents");
        assert!(parse_rules("nonsense").is_err());
    }
}
