//! `files.rename`: batch rename with tokens ({name}, {n:03}, {date:%Y-%m-%d}, {exif.date},
//! {exif.camera}, {ext}, {parent}, {created}, {size}), find & replace (plain or regex), case,
//! character removal, extension changes and numbering. Collision-safe, previewed, undoable.

use std::path::PathBuf;

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common::{self, Cell, MoveJournal};

pub struct Rename {
    meta: ToolMeta,
}

impl Rename {
    pub fn new() -> Self {
        Rename {
            meta: common::meta(
                "files.rename",
                InputSpec::kinds(common::ANY).folders(),
                "—",
                &["rename", "batch", "bulk", "names", "numbering", "regex", "replace", "exif", "date"],
                "",
                true,
            ),
        }
    }
}

impl Tool for Rename {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::text("pattern", "{name}"),
            OptionSpec::text("find", ""),
            OptionSpec::text("replace", "").when(Cond::Ne("find", "")),
            OptionSpec::toggle("regex", false).when(Cond::Ne("find", "")),
            OptionSpec::toggle("match_case", false).when(Cond::Ne("find", "")),
            OptionSpec::choice("case", ["keep", "lower", "upper", "title", "sentence", "kebab", "snake"].map(Choice::key).to_vec(), "keep"),
            OptionSpec::text("remove", ""),
            OptionSpec::toggle("ascii", false),
            OptionSpec::choice("extension", ["keep", "lower", "upper", "change", "remove"].map(Choice::key).to_vec(), "keep"),
            OptionSpec::text("new_extension", "").when(Cond::Eq("extension", "change")),
            OptionSpec::choice("order", ["natural", "modified", "created", "exif", "size"].map(Choice::key).to_vec(), "natural"),
            OptionSpec::int("start", 0, 1_000_000, 1),
            OptionSpec::int("step", 1, 1000, 1).advanced(),
            OptionSpec::toggle("recursive", false).advanced(),
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
        let plan = plan_names(&files, v)?;
        let changed = plan.iter().filter(|(a, b)| a != b).count();
        let rows: Vec<Vec<Cell>> = plan
            .iter()
            .map(|(a, b)| {
                let (an, bn) = (name_of(a), name_of(b));
                let col = if an == bn { None } else { Some(common::GOOD) };
                vec![(an, None), (bn, col)]
            })
            .collect();
        let caption = tr_args("files.rename.caption", &[("changed", &changed.to_string()), ("total", &plan.len().to_string())]);
        let png = common::table_png(&caption, &[&tr("files.rename.col.before"), &tr("files.rename.col.after")], &rows, max_side, 0)?;
        Ok(Some(Preview { png, caption: None }))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let plan = plan_names(&job.inputs, v)?;
        let moves: Vec<(PathBuf, PathBuf)> = plan.into_iter().filter(|(a, b)| a != b).collect();
        if moves.is_empty() {
            return Ok(job.inputs.iter().map(|f| Output::new(&f.path).with_note(tr("files.rename.unchanged"))).collect());
        }
        let journal = common::apply_moves(&moves, cx)?;
        let undo = serde_json::to_value(&journal)?;
        let mut outs: Vec<Output> = journal.moves.iter().map(|(a, b)| Output::new(b).with_note(format!("← {}", name_of(a)))).collect();
        if let Some(first) = outs.first_mut() {
            first.undo = Some(undo);
        }
        Ok(outs)
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let j: MoveJournal = serde_json::from_value(journal.clone())?;
        common::undo_moves(&j)
    }
}

fn name_of(p: &std::path::Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// (old path, new path) for every file, in numbering order. Errors on invalid patterns and on
/// two files ending up with the same name.
pub fn plan_names(files: &[FileInfo], v: &Values) -> Result<Vec<(PathBuf, PathBuf)>> {
    let mut files: Vec<FileInfo> = files.iter().filter(|f| !f.is_dir).cloned().collect();
    match v.str("order") {
        "modified" => files.sort_by_key(|f| common::times(&f.path).1),
        "created" => files.sort_by_key(|f| common::times(&f.path).0),
        "exif" => files.sort_by_key(|f| common::capture_date(&f.path).or(common::times(&f.path).1)),
        "size" => files.sort_by_key(|f| f.size),
        _ => files.sort_by(|a, b| castor_core::files::natural_cmp(&a.path.to_string_lossy(), &b.path.to_string_lossy())),
    }
    let find = v.str("find");
    let re = if !find.is_empty() {
        let pat = if v.bool("regex") { find.to_string() } else { regex::escape(find) };
        Some(
            regex::RegexBuilder::new(&pat)
                .case_insensitive(!v.bool("match_case"))
                .build()
                .map_err(|e| anyhow::anyhow!("{}: {e}", tr("files.rename.err.regex")))?,
        )
    } else {
        None
    };
    let pattern = if v.str("pattern").trim().is_empty() { "{name}" } else { v.str("pattern") };
    let (start, step) = (v.int("start"), v.int("step").max(1));
    let count_width = (start + step * files.len() as i64).to_string().len();
    let mut out = Vec::with_capacity(files.len());
    let mut taken = std::collections::HashSet::new();
    for (i, f) in files.iter().enumerate() {
        let n = start + step * i as i64;
        let ext = f.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let mut stem = expand(pattern, f, n, count_width, &ext)?;
        if let Some(re) = &re {
            stem = re.replace_all(&stem, v.str("replace")).into_owned();
        }
        for c in v.str("remove").chars() {
            stem = stem.replace(c, "");
        }
        if v.bool("ascii") {
            stem = ascii_fold(&stem);
        }
        stem = change_case(&stem, v.str("case"));
        let ext = match v.str("extension") {
            "lower" => ext.to_lowercase(),
            "upper" => ext.to_uppercase(),
            "change" => v.str("new_extension").trim().trim_start_matches('.').to_string(),
            "remove" => String::new(),
            _ => ext,
        };
        let stem = castor_core::job::sanitize_file_name(stem.trim());
        let name = if ext.is_empty() { stem } else { format!("{stem}.{ext}") };
        let new = f.path.with_file_name(&name);
        anyhow::ensure!(
            taken.insert(new.to_string_lossy().to_lowercase()),
            "{}",
            tr_args("files.err.duplicate_target", &[("name", &name)])
        );
        out.push((f.path.clone(), new));
    }
    Ok(out)
}

/// Expand tokens in the pattern. `{n}` / `{n:03}`, `{date}` / `{date:%Y%m%d}`, `{created:…}`,
/// `{exif.date:…}`, `{exif.camera}`, `{name}`, `{ext}`, `{parent}`, `{size}`.
fn expand(pattern: &str, f: &FileInfo, n: i64, width: usize, ext: &str) -> Result<String> {
    let mut out = String::new();
    let mut rest = pattern;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        let b = after.find('}').ok_or_else(|| anyhow::anyhow!("{}", tr("files.rename.err.brace")))?;
        let tok = &after[..b];
        let (name, arg) = tok.split_once(':').map(|(x, y)| (x, Some(y))).unwrap_or((tok, None));
        let s = match name.trim() {
            "name" => f.stem(),
            "ext" => ext.to_string(),
            "parent" => f.path.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            "size" => castor_core::units::bytes(f.size),
            "n" | "i" | "#" => {
                // {n:03} → 3 digits, {n:3} → 3 digits, {n:auto} → as wide as the last number.
                let w = match arg.map(str::trim) {
                    Some("auto") => width,
                    Some(a) if a.starts_with('0') && a.len() > 1 => a[1..].parse().unwrap_or(a.len()),
                    Some(a) => a.parse().unwrap_or(0),
                    None => 0,
                };
                format!("{n:0w$}")
            }
            "date" | "modified" => common::times(&f.path).1.map(|t| common::fmt_time(t, arg.unwrap_or("%Y-%m-%d"))).unwrap_or_default(),
            "created" => common::times(&f.path).0.map(|t| common::fmt_time(t, arg.unwrap_or("%Y-%m-%d"))).unwrap_or_default(),
            "exif.date" | "taken" => common::capture_date(&f.path)
                .or(common::times(&f.path).1)
                .map(|t| common::fmt_time(t, arg.unwrap_or("%Y-%m-%d")))
                .unwrap_or_default(),
            "exif.camera" | "camera" => common::exif_camera(&f.path).unwrap_or_default(),
            other => anyhow::bail!("{}", tr_args("files.rename.err.token", &[("token", other)])),
        };
        out.push_str(&s);
        rest = &after[b + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

pub fn change_case(s: &str, case: &str) -> String {
    match case {
        "lower" => s.to_lowercase(),
        "upper" => s.to_uppercase(),
        "title" => s
            .split(' ')
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        "sentence" => {
            let l = s.to_lowercase();
            let mut c = l.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
        "kebab" | "snake" => {
            let sep = if case == "kebab" { '-' } else { '_' };
            let mut o = String::new();
            let mut prev_lower = false;
            for c in s.chars() {
                if c.is_alphanumeric() {
                    if c.is_uppercase() && prev_lower && !o.ends_with(sep) {
                        o.push(sep);
                    }
                    o.extend(c.to_lowercase());
                    prev_lower = c.is_lowercase() || c.is_ascii_digit();
                } else if !o.is_empty() && !o.ends_with(sep) {
                    o.push(sep);
                    prev_lower = false;
                }
            }
            o.trim_end_matches(sep).to_string()
        }
        _ => s.to_string(),
    }
}

/// Strip accents and replace other non-ASCII letters (é → e, œ → oe, ß → ss).
pub fn ascii_fold(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            'œ' => o.push_str("oe"),
            'Œ' => o.push_str("OE"),
            'æ' => o.push_str("ae"),
            'Æ' => o.push_str("AE"),
            'ß' => o.push_str("ss"),
            c if c.is_ascii() => o.push(c),
            c => {
                let f = castor_core::registry::fold(&c.to_string());
                let up = c.is_uppercase();
                for d in f.chars().filter(|d| d.is_ascii()) {
                    o.extend(if up { d.to_uppercase().collect::<Vec<_>>() } else { vec![d] });
                }
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cases() {
        assert_eq!(change_case("My Holiday Photo", "kebab"), "my-holiday-photo");
        assert_eq!(change_case("myHolidayPhoto 2", "snake"), "my_holiday_photo_2");
        assert_eq!(change_case("hello WORLD", "title"), "Hello World");
        assert_eq!(ascii_fold("Été à Kraków œuvre"), "Ete a Krakow oeuvre");
    }
}
