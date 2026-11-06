//! image.duplicates — group identical and near-identical images (perceptual hash), HTML report,
//! optionally move the extra copies into a "Duplicates" folder (undoable).

use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::Engine;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, RunCx, Tool, ToolMeta, ToolStatus};
use rayon::prelude::*;
use sha2::Digest;

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::{common, ops};

pub struct Duplicates {
    meta: ToolMeta,
}

impl Duplicates {
    pub fn new() -> Self {
        let mut meta = common::meta(
            "image.duplicates",
            "HTML",
            &["duplicates", "similar", "same", "dedupe", "clean up", "copies", "perceptual hash"],
            "duplicates",
            ToolStatus::Ready,
            &[],
            false,
        );
        meta.inputs = common::IMAGE_INPUTS.folders();
        meta.in_place = true;
        Duplicates { meta }
    }
}

struct Item {
    file: FileInfo,
    sha: String,
    hash: Option<image_hasher::ImageHash>,
    pixels: u64,
    dims: (u32, u32),
    thumb: Option<Vec<u8>>,
}

impl Tool for Duplicates {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("sensitivity", vec![Choice::key("exact"), Choice::key("near"), Choice::key("similar")], "near"),
            OptionSpec::choice("action", vec![Choice::key("report"), Choice::key("move")], "report"),
            OptionSpec::choice("keep", vec![Choice::key("largest"), Choice::key("oldest"), Choice::key("first")], "largest").when(Cond::Eq("action", "move")),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        let files: Vec<FileInfo> = castor_core::files::expand(&inputs.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), true)
            .into_iter()
            .filter(|f| f.kind.is_image_like())
            .collect();
        anyhow::ensure!(files.len() >= 2, "{}", castor_core::tr("image.duplicates.need_two"));
        Ok(vec![Job::many(files)])
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let n = job.inputs.len();
        let done = std::sync::atomic::AtomicUsize::new(0);
        let hasher = image_hasher::HasherConfig::new().hash_size(16, 16).hash_alg(image_hasher::HashAlg::Gradient).to_hasher();
        let items: Vec<Item> = job
            .inputs
            .par_iter()
            .map(|f| {
                if cx.is_cancelled() {
                    return None;
                }
                let bytes = std::fs::read(&f.path).ok()?;
                let sha = hex::encode(sha2::Sha256::digest(&bytes));
                drop(bytes);
                let d = codec::decode(&f.path, DecodeOpts { svg_side: 512, ..Default::default() }).ok();
                let (hash, pixels, dims, thumb) = match d {
                    Some(d) => {
                        let (w, h) = (d.image.width(), d.image.height());
                        let small = d.image.thumbnail(256, 256);
                        let hash = hasher.hash_image(&small);
                        let t = small.thumbnail(160, 160);
                        let jpg = codec::encode(&t, Fmt::Jpeg, &EncodeOpts { quality: 70, ..Default::default() }).ok();
                        (Some(hash), w as u64 * h as u64, (w, h), jpg)
                    }
                    None => (None, 0, (0, 0), None),
                };
                let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                cx.progress(k as f32 / n as f32 * 0.9);
                Some(Item { file: f.clone(), sha, hash, pixels, dims, thumb })
            })
            .collect::<Vec<_>>()
            .into_iter()
            .flatten()
            .collect();
        cx.check_cancel()?;
        let max_dist = match v.str("sensitivity") {
            "exact" => 0,
            "similar" => 40,
            _ => 16,
        };
        let groups = group(&items, max_dist);
        // Order each group: the copy to keep first.
        let keep_mode = v.str("keep");
        let mut groups: Vec<Vec<usize>> = groups
            .into_iter()
            .map(|mut g| {
                g.sort_by(|a, b| {
                    let (x, y) = (&items[*a], &items[*b]);
                    match keep_mode {
                        "oldest" => modified(&x.file).cmp(&modified(&y.file)),
                        "first" => a.cmp(b),
                        _ => y.pixels.cmp(&x.pixels).then(y.file.size.cmp(&x.file.size)),
                    }
                });
                g
            })
            .collect();
        groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
        let extra: usize = groups.iter().map(|g| g.len() - 1).sum();

        let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
        if v.str("action") == "move" {
            let folder_name = castor_core::tr("image.duplicates.folder");
            for g in &groups {
                for idx in &g[1..] {
                    let src = &items[*idx].file.path;
                    let dir = src.parent().map(|p| p.join(&folder_name)).context("no parent folder")?;
                    std::fs::create_dir_all(&dir).with_context(|| common::err("err.write", &dir))?;
                    let dst = castor_core::job::unique_path(&dir.join(src.file_name().unwrap_or_default()));
                    std::fs::rename(src, &dst).with_context(|| common::err("err.write", src))?;
                    moves.push((src.clone(), dst));
                }
            }
        }

        let report = html_report(&items, &groups, &moves);
        let out = cx.output_named(&format!("{}.html", castor_core::tr("image.duplicates.report")));
        common::write(&out, report.as_bytes())?;
        let note = if groups.is_empty() {
            castor_core::tr("image.duplicates.none")
        } else {
            castor_core::tr_args("image.duplicates.found", &[("groups", &groups.len().to_string()), ("extra", &extra.to_string())])
        };
        let journal = serde_json::json!({ "moves": moves.iter().map(|(a, b)| [a, b]).collect::<Vec<_>>() });
        Ok(vec![Output::new(out).with_note(note).with_undo(journal)])
    }

    fn undo(&self, journal: &serde_json::Value) -> Result<()> {
        let moves = journal["moves"].as_array().cloned().unwrap_or_default();
        for m in moves.iter().rev() {
            let (Some(from), Some(to)) = (m[0].as_str(), m[1].as_str()) else { continue };
            if std::path::Path::new(from).exists() {
                anyhow::bail!("{}", castor_core::tr_args("image.duplicates.undo_exists", &[("file", from)]));
            }
            std::fs::rename(to, from)?;
            if let Some(dir) = std::path::Path::new(to).parent() {
                let _ = std::fs::remove_dir(dir); // only if now empty
            }
        }
        Ok(())
    }
}

fn modified(f: &FileInfo) -> std::time::SystemTime {
    std::fs::metadata(&f.path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// Union-find over pairs that are byte-identical or within `max_dist` bits.
fn group(items: &[Item], max_dist: u32) -> Vec<Vec<usize>> {
    let n = items.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut [usize], i: usize) -> usize {
        let mut r = i;
        while p[r] != r {
            r = p[r];
        }
        let mut i = i;
        while p[i] != r {
            let next = p[i];
            p[i] = r;
            i = next;
        }
        r
    }
    for i in 0..n {
        for j in i + 1..n {
            let same = items[i].sha == items[j].sha
                || match (&items[i].hash, &items[j].hash) {
                    (Some(a), Some(b)) => {
                        // Very different aspect ratios are never "the same picture".
                        let ra = items[i].dims.0 as f32 / items[i].dims.1.max(1) as f32;
                        let rb = items[j].dims.0 as f32 / items[j].dims.1.max(1) as f32;
                        a.dist(b) <= max_dist && (ra / rb - 1.0).abs() < 0.12 && max_dist > 0
                    }
                    _ => false,
                };
            if same {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }
    let mut map: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for i in 0..n {
        let r = find(&mut parent, i);
        map.entry(r).or_default().push(i);
    }
    map.into_values().filter(|g| g.len() > 1).collect()
}

fn html_report(items: &[Item], groups: &[Vec<usize>], moves: &[(PathBuf, PathBuf)]) -> String {
    let esc = ops::xml_escape;
    let mut body = String::new();
    for (gi, g) in groups.iter().enumerate() {
        body += &format!("<section><h2>{} {}</h2><div class=\"row\">", esc(&castor_core::tr("image.duplicates.group")), gi + 1);
        for (k, idx) in g.iter().enumerate() {
            let it = &items[*idx];
            let img = it
                .thumb
                .as_ref()
                .map(|b| format!("<img src=\"data:image/jpeg;base64,{}\" alt=\"\">", base64::engine::general_purpose::STANDARD.encode(b)))
                .unwrap_or_default();
            let moved = moves.iter().find(|(a, _)| *a == it.file.path).map(|(_, b)| format!("<div class=\"mv\">→ {}</div>", esc(&b.display().to_string()))).unwrap_or_default();
            let tag = if k == 0 { format!("<b class=\"keep\">{}</b>", esc(&castor_core::tr("image.duplicates.kept"))) } else { String::new() };
            body += &format!(
                "<figure>{img}<figcaption>{tag}<div title=\"{p}\">{name}</div><small>{dims} · {size}</small>{moved}</figcaption></figure>",
                p = esc(&it.file.path.display().to_string()),
                name = esc(&it.file.name()),
                dims = common::dims(it.dims.0, it.dims.1),
                size = castor_core::units::bytes(it.file.size),
            );
        }
        body += "</div></section>";
    }
    if groups.is_empty() {
        body = format!("<p>{}</p>", esc(&castor_core::tr("image.duplicates.none")));
    }
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"generator\" content=\"Castor\"><title>{t}</title><style>body{{font:14px system-ui,sans-serif;margin:24px;background:#fafafa;color:#222}}h1{{font-size:20px}}h2{{font-size:15px;margin:18px 0 8px}}.row{{display:flex;flex-wrap:wrap;gap:12px}}figure{{margin:0;width:170px;background:#fff;border:1px solid #ddd;border-radius:8px;padding:6px}}img{{width:160px;height:160px;object-fit:contain;background:#eee;border-radius:4px}}figcaption div{{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}small{{color:#777}}.keep{{color:#1a7f37;font-size:12px}}.mv{{color:#b35900;font-size:11px;word-break:break-all}}</style></head><body><h1>{t}</h1><p>{n}</p>{body}</body></html>",
        t = esc(&castor_core::tr("image.duplicates.report")),
        n = esc(&castor_core::tr_args("image.duplicates.scanned", &[("count", &items.len().to_string())])),
    )
}
