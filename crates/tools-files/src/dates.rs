//! `files.dates`: set the created / modified dates of files — to a given date, to the photo or
//! video capture date (EXIF, QuickTime), or shifted by ± N. Previewed, undoable.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};
use serde::{Deserialize, Serialize};

use crate::common::{self, Cell};

pub struct Dates {
    meta: ToolMeta,
}

impl Dates {
    pub fn new() -> Self {
        Dates {
            meta: common::meta(
                "files.dates",
                InputSpec::kinds(common::ANY).folders(),
                "—",
                &["date", "timestamp", "created", "modified", "exif", "touch", "time", "shift", "timezone"],
                "",
                true,
            ),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Journal {
    /// (path, created nanos, modified nanos)
    files: Vec<(PathBuf, Option<i128>, Option<i128>)>,
}

impl Tool for Dates {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", ["capture", "set", "shift", "modified_to_created", "created_to_modified"].map(Choice::key).to_vec(), "capture"),
            OptionSpec::text("date", "").when(Cond::Eq("mode", "set")),
            OptionSpec::int("shift_days", -36_500, 36_500, 0).unit("d").when(Cond::Eq("mode", "shift")),
            OptionSpec::int("shift_hours", -1000, 1000, 0).unit("h").when(Cond::Eq("mode", "shift")),
            OptionSpec::int("shift_minutes", -1000, 1000, 0).unit("min").when(Cond::Eq("mode", "shift")),
            OptionSpec::choice("which", ["both", "modified", "created"].map(Choice::key).to_vec(), "both"),
            OptionSpec::toggle("recursive", true).advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], v: &Values) -> Result<Vec<Job>> {
        let files = common::expand_inputs(inputs, v.bool("recursive"));
        anyhow::ensure!(!files.is_empty(), "{}", tr("files.err.no_files"));
        Ok(vec![Job::many(files)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let files = if f.is_dir { common::expand_inputs(std::slice::from_ref(f), v.bool("recursive")) } else { vec![f.clone()] };
        let target = target_fn(v)?;
        let fmt = "%Y-%m-%d %H:%M";
        let show = |t: Option<SystemTime>| t.map(|t| common::fmt_time(t, fmt)).unwrap_or_else(|| "—".into());
        let mut rows: Vec<Vec<Cell>> = Vec::new();
        for f in files.iter().take(200) {
            let (c, m) = common::times(&f.path);
            let (nc, nm) = new_times(&*target, &f.path, c, m, which(v));
            let changed = |a: Option<SystemTime>, b: Option<SystemTime>| if b.is_some() && a != b { Some(common::GOOD) } else { None };
            let skipped = nc.is_none() && nm.is_none();
            rows.push(vec![
                (f.name(), None),
                (show(m), None),
                (if skipped { tr("files.dates.no_date") } else { show(nm.or(m)) }, if skipped { Some(common::BAD) } else { changed(m, nm) }),
                (show(nc.or(c)), changed(c, nc)),
            ]);
        }
        let png = common::table_png(
            &tr_args("files.dates.caption", &[("count", &files.len().to_string())]),
            &[&tr("files.col.file"), &tr("files.dates.col.modified"), &tr("files.dates.col.new_modified"), &tr("files.dates.col.new_created")],
            &rows,
            max_side,
            files.len().saturating_sub(200),
        )?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let target = target_fn(v)?;
        let mut journal = Journal { files: Vec::new() };
        let mut outs = Vec::new();
        let mut skipped = 0;
        let mut errors = Vec::new();
        for (i, f) in job.inputs.iter().enumerate() {
            cx.check_cancel()?;
            cx.progress(i as f32 / job.inputs.len() as f32);
            let (c, m) = common::times(&f.path);
            let (nc, nm) = new_times(&*target, &f.path, c, m, which(v));
            if nc.is_none() && nm.is_none() {
                skipped += 1;
                continue;
            }
            match common::set_times(&f.path, nc, nm) {
                Ok(()) => {
                    journal.files.push((f.path.clone(), c.map(common::to_nanos), m.map(common::to_nanos)));
                    let shown = nm.or(nc).map(|t| common::fmt_time(t, "%Y-%m-%d %H:%M")).unwrap_or_default();
                    outs.push(Output::new(&f.path).with_note(shown));
                }
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        if outs.is_empty() {
            anyhow::bail!("{}", if errors.is_empty() { tr("files.dates.err.nothing") } else { errors.join("\n") });
        }
        if let Some(first) = outs.first_mut() {
            first.undo = Some(serde_json::to_value(&journal)?);
            let mut note = first.note.clone().unwrap_or_default();
            if skipped > 0 {
                note.push_str(&format!(" · {}", tr_args("files.dates.skipped", &[("count", &skipped.to_string())])));
            }
            if !errors.is_empty() {
                note.push_str(&format!(" · {}", tr_args("files.dates.failed", &[("count", &errors.len().to_string())])));
            }
            first.note = Some(note);
        }
        Ok(outs)
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let j: Journal = serde_json::from_value(journal.clone())?;
        let mut failed = Vec::new();
        for (p, c, m) in &j.files {
            if let Err(e) = common::set_times(p, c.map(common::from_nanos), m.map(common::from_nanos)) {
                failed.push(format!("{e:#}"));
            }
        }
        anyhow::ensure!(failed.is_empty(), "{}\n{}", tr("files.err.undo_partial"), failed.join("\n"));
        Ok(())
    }
}

/// The `which` argument of `new_times`, folding the copy modes in.
fn which(v: &Values) -> &str {
    match v.str("mode") {
        "modified_to_created" => "copy_to_modified",
        "created_to_modified" => "copy_to_created",
        _ => v.str("which"),
    }
}

type Target = dyn Fn(&std::path::Path, Option<SystemTime>, Option<SystemTime>) -> Option<SystemTime> + Sync;

/// The new date of a file from its path and (created, modified), or None to leave it.
fn target_fn(v: &Values) -> Result<Box<Target>> {
    Ok(match v.str("mode") {
        "set" => {
            let t = common::parse_local(v.str("date")).ok_or_else(|| anyhow::anyhow!("{}", tr_args("files.dates.err.date", &[("value", v.str("date"))])))?;
            Box::new(move |_, _, _| Some(t))
        }
        "shift" => {
            let secs = v.int("shift_days") * 86_400 + v.int("shift_hours") * 3600 + v.int("shift_minutes") * 60;
            anyhow::ensure!(secs != 0, "{}", tr("files.dates.err.zero"));
            Box::new(move |_, _, m| m.map(|t| if secs > 0 { t + Duration::from_secs(secs as u64) } else { t - Duration::from_secs((-secs) as u64) }))
        }
        "modified_to_created" => Box::new(|_, c, _| c),
        "created_to_modified" => Box::new(|_, _, m| m),
        _ => Box::new(|p, _, _| common::capture_date(p)),
    })
}

fn new_times(target: &Target, p: &std::path::Path, c: Option<SystemTime>, m: Option<SystemTime>, which: &str) -> (Option<SystemTime>, Option<SystemTime>) {
    // Copy modes touch only the other date.
    match which {
        "copy_to_modified" => return (None, c),
        "copy_to_created" => return (m, None),
        _ => {}
    }
    // "Shift" moves each date by the same amount, separately.
    let t_m = target(p, c, m);
    let t_c = target(p, c, c);
    match which {
        "modified" => (None, t_m),
        "created" => (t_c, None),
        _ => (t_c, t_m),
    }
}
