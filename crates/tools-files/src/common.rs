//! Shared pieces of the files tools: metadata builder, move journals with undo, preview tables
//! drawn in the house style, dates (EXIF, QuickTime, file times) and small text helpers.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use anyhow::{Result, anyhow};
use castor_core::{Family, FileKind, InputSpec, ToolMeta, ToolStatus, tr, tr_args};
use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};

pub fn meta(id: &'static str, inputs: InputSpec, output: &'static str, keywords: &'static [&'static str], suffix: &'static str, in_place: bool) -> ToolMeta {
    ToolMeta {
        id,
        family: Family::Files,
        inputs,
        output,
        keywords,
        status: ToolStatus::Ready,
        requires: &[],
        suffix,
        in_place,
        serial: false,
    }
}

pub const ANY: &[FileKind] = &[FileKind::Any];

/// Every input expanded (folders → their files, recursively), deduplicated, natural order.
pub fn expand_inputs(inputs: &[castor_core::FileInfo], recursive: bool) -> Vec<castor_core::FileInfo> {
    let mut v = castor_core::files::expand(&inputs.iter().map(|f| f.path.clone()).collect::<Vec<_>>(), recursive);
    let mut seen = std::collections::HashSet::new();
    v.retain(|f| seen.insert(f.path.clone()));
    v
}

// ---------------------------------------------------------------- moves + undo

/// A journal of file moves (renames), stored in `Output::undo`.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MoveJournal {
    pub moves: Vec<(PathBuf, PathBuf)>,
    /// Folders created by the run (removed on undo when empty).
    #[serde(default)]
    pub created_dirs: Vec<PathBuf>,
}

/// Apply planned moves safely: never overwrite, two phases when targets collide with sources
/// (a → b, b → c), case-only renames through a temporary name. Returns the journal of what
/// actually moved; on the first error, everything done so far is rolled back.
pub fn apply_moves(plan: &[(PathBuf, PathBuf)], cx: &castor_core::RunCx) -> Result<MoveJournal> {
    let sources: std::collections::HashSet<String> = plan.iter().map(|(a, _)| key(a)).collect();
    // Destinations must be unique and must not exist unless they are themselves being moved away.
    let mut dests = std::collections::HashSet::new();
    for (from, to) in plan {
        anyhow::ensure!(dests.insert(key(to)), "{}", tr_args("files.err.duplicate_target", &[("name", &to.display().to_string())]));
        if to.exists() && !sources.contains(&key(to)) && key(from) != key(to) {
            anyhow::bail!("{}", tr_args("files.err.exists", &[("name", &to.display().to_string())]));
        }
    }
    let mut journal = MoveJournal::default();
    let mut staged: Vec<(PathBuf, PathBuf, PathBuf)> = Vec::new(); // (orig, tmp, final)
    let res = (|| -> Result<()> {
        for (i, (from, to)) in plan.iter().enumerate() {
            cx.check_cancel()?;
            if from == to {
                continue;
            }
            if let Some(d) = to.parent() {
                if !d.exists() {
                    std::fs::create_dir_all(d)?;
                    journal.created_dirs.push(d.to_path_buf());
                }
            }
            // Phase 1: everything goes to a unique temporary name in its destination folder.
            let tmp = to.with_file_name(format!(".castor-{}-{i}.tmp", std::process::id()));
            move_file(from, &tmp)?;
            staged.push((from.clone(), tmp, to.clone()));
            cx.progress((i + 1) as f32 / plan.len() as f32 * 0.5);
        }
        for (i, (orig, tmp, fin)) in staged.iter().enumerate() {
            anyhow::ensure!(!fin.exists(), "{}", tr_args("files.err.exists", &[("name", &fin.display().to_string())]));
            move_file(tmp, fin)?;
            journal.moves.push((orig.clone(), fin.clone()));
            cx.progress(0.5 + (i + 1) as f32 / staged.len().max(1) as f32 * 0.5);
        }
        Ok(())
    })();
    if let Err(e) = res {
        // Roll back: finished moves, then staged temporaries.
        let done: std::collections::HashSet<PathBuf> = journal.moves.iter().map(|(a, _)| a.clone()).collect();
        let _ = undo_moves(&journal);
        for (orig, tmp, _) in &staged {
            if !done.contains(orig) && tmp.exists() && !orig.exists() {
                let _ = move_file(tmp, orig);
            }
        }
        return Err(e);
    }
    Ok(journal)
}

fn key(p: &Path) -> String {
    // Windows and macOS file systems are case-insensitive by default.
    p.to_string_lossy().to_lowercase()
}

/// Rename, falling back to copy + delete across volumes.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(17) || e.raw_os_error() == Some(18) || e.kind() == std::io::ErrorKind::CrossesDevices => {
            std::fs::copy(from, to)?;
            if let Ok(m) = std::fs::metadata(from) {
                let _ = filetime_copy(&m, to);
            }
            std::fs::remove_file(from)?;
            Ok(())
        }
        Err(e) => Err(anyhow!("{} → {}: {e}", from.display(), to.display())),
    }
}

fn filetime_copy(m: &std::fs::Metadata, to: &Path) -> Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(to)?;
    let mut t = std::fs::FileTimes::new();
    if let Ok(x) = m.modified() {
        t = t.set_modified(x);
    }
    if let Ok(x) = m.accessed() {
        t = t.set_accessed(x);
    }
    f.set_times(t)?;
    Ok(())
}

/// Undo a move journal (last move first). Errors list the files that could not go back.
pub fn undo_moves(j: &MoveJournal) -> Result<()> {
    let mut failed = Vec::new();
    for (from, to) in j.moves.iter().rev() {
        if !to.exists() {
            failed.push(format!("{} ({})", to.display(), tr("files.err.gone")));
            continue;
        }
        if from.exists() && key(from) != key(to) {
            failed.push(format!("{} ({})", from.display(), tr("files.err.occupied")));
            continue;
        }
        let tmp = from.with_file_name(format!(".castor-undo-{}.tmp", std::process::id()));
        if let Err(e) = move_file(to, &tmp).and_then(|_| move_file(&tmp, from)) {
            failed.push(format!("{}: {e}", to.display()));
        }
    }
    for d in j.created_dirs.iter().rev() {
        let _ = std::fs::remove_dir(d); // only if empty
    }
    anyhow::ensure!(failed.is_empty(), "{}\n{}", tr("files.err.undo_partial"), failed.join("\n"));
    Ok(())
}

// ---------------------------------------------------------------- preview tables

const SANS: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-regular.otf");
const SANS_BOLD: &[u8] = include_bytes!("../../../assets/fonts/lmsans10-bold.otf");
const MONO: &[u8] = include_bytes!("../../../assets/fonts/lmmono10-regular.otf");

const PAPER: [u8; 3] = [0xfc, 0xfb, 0xf9];
const INK: [u8; 3] = [0x1b, 0x16, 0x16];
const MUTED: [u8; 3] = [0x5c, 0x5a, 0x55];
const RULE: [u8; 3] = [0xd9, 0xd6, 0xcf];
pub const GOOD: [u8; 3] = [0x2e, 0x7d, 0x4f];
pub const BAD: [u8; 3] = [0xc0, 0x39, 0x2b];

/// A cell: text and colour (None = ink).
pub type Cell = (String, Option<[u8; 3]>);

/// Draw a booktabs table ("Table 1 — caption", strong top/bottom rules, thin header rule) as PNG.
pub fn table_png(caption: &str, header: &[&str], rows: &[Vec<Cell>], max_side: u32, more: usize) -> Result<Vec<u8>> {
    let sans = FontRef::try_from_slice(SANS).map_err(|e| anyhow!("{e}"))?;
    let bold = FontRef::try_from_slice(SANS_BOLD).map_err(|e| anyhow!("{e}"))?;
    let mono = FontRef::try_from_slice(MONO).map_err(|e| anyhow!("{e}"))?;
    let width = max_side.clamp(400, 2400);
    let k = width as f32 / 900.0;
    let fs = 15.0 * k;
    let row_h = (30.0 * k) as u32;
    let pad = (24.0 * k) as u32;
    let shown = rows.len().min(((max_side as f32 * 1.1 / row_h as f32) as usize).saturating_sub(4).max(3));
    let height = pad * 2 + row_h * (shown as u32 + 2) + if more + rows.len() > shown { row_h } else { 0 } + (8.0 * k) as u32;
    let mut img = image::RgbImage::from_pixel(width, height, image::Rgb(PAPER));
    let cols = header.len().max(1);
    // Column widths: proportional to the longest text, at least a little.
    let measure = |f: &FontRef, s: &str| text_width(f, fs, s);
    let mut w: Vec<f32> = (0..cols).map(|c| measure(&bold, header.get(c).copied().unwrap_or("")) + 20.0 * k).collect();
    for r in rows.iter().take(shown) {
        for (c, cell) in r.iter().enumerate().take(cols) {
            w[c] = w[c].max(measure(&mono, &cell.0) + 20.0 * k);
        }
    }
    let avail = (width - 2 * pad) as f32;
    let total: f32 = w.iter().sum();
    if total > avail {
        let s = avail / total;
        w.iter_mut().for_each(|x| *x *= s);
    }
    let x0 = pad as f32;
    let mut y = pad as f32;
    draw_text(&mut img, &sans, fs * 0.92, x0, y + fs, caption, MUTED);
    y += row_h as f32 * 0.9;
    hline(&mut img, x0 as u32, width - pad, y as u32, (1.6 * k).max(1.0) as u32, INK);
    let mut x = x0;
    for (c, h) in header.iter().enumerate() {
        draw_text(&mut img, &bold, fs, x, y + row_h as f32 * 0.68, &clip(&bold, fs, h, w[c] - 10.0 * k), INK);
        x += w[c];
    }
    y += row_h as f32;
    hline(&mut img, x0 as u32, width - pad, y as u32, 1, INK);
    for r in rows.iter().take(shown) {
        let mut x = x0;
        for (c, cell) in r.iter().enumerate().take(cols) {
            let col = cell.1.unwrap_or(INK);
            draw_text(&mut img, &mono, fs * 0.92, x, y + row_h as f32 * 0.66, &clip(&mono, fs * 0.92, &cell.0, w[c] - 10.0 * k), col);
            x += w[c];
        }
        y += row_h as f32;
        hline(&mut img, x0 as u32, width - pad, y as u32, 1, RULE);
    }
    hline(&mut img, x0 as u32, width - pad, y as u32, (1.6 * k).max(1.0) as u32, INK);
    let hidden = rows.len() - shown + more;
    if hidden > 0 {
        draw_text(&mut img, &sans, fs * 0.9, x0, y + row_h as f32 * 0.75, &tr_args("files.preview.more", &[("count", &hidden.to_string())]), MUTED);
    }
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)?;
    Ok(out)
}

use image::ImageEncoder;

fn hline(img: &mut image::RgbImage, x0: u32, x1: u32, y: u32, t: u32, c: [u8; 3]) {
    for yy in y..(y + t).min(img.height()) {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, yy, image::Rgb(c));
        }
    }
}

fn text_width(f: &FontRef, size: f32, s: &str) -> f32 {
    let sf = f.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut prev = None;
    for c in s.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            w += sf.kern(p, id);
        }
        w += sf.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Shorten with "…" in the middle so both ends stay readable.
fn clip(f: &FontRef, size: f32, s: &str, max: f32) -> String {
    if text_width(f, size, s) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut keep = chars.len();
    while keep > 2 {
        keep -= 1;
        let a = keep / 2;
        let b = keep - a;
        let t: String = chars[..a].iter().chain(['…'].iter()).chain(chars[chars.len() - b..].iter()).collect();
        if text_width(f, size, &t) <= max {
            return t;
        }
    }
    "…".into()
}

fn draw_text(img: &mut image::RgbImage, f: &FontRef, size: f32, x: f32, baseline: f32, s: &str, col: [u8; 3]) {
    let sf = f.as_scaled(PxScale::from(size));
    let mut pen = x;
    let mut prev = None;
    for c in s.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            pen += sf.kern(p, id);
        }
        let g = id.with_scale_and_position(PxScale::from(size), ab_glyph::point(pen, baseline));
        pen += sf.h_advance(id);
        prev = Some(id);
        if let Some(o) = f.outline_glyph(g) {
            let b = o.px_bounds();
            o.draw(|gx, gy, cov| {
                let (px, py) = (b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32);
                if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                    let p = img.get_pixel_mut(px as u32, py as u32);
                    for i in 0..3 {
                        p[i] = (p[i] as f32 * (1.0 - cov) + col[i] as f32 * cov).round() as u8;
                    }
                }
            });
        }
    }
}

// ---------------------------------------------------------------- dates

/// Local date-time from a SystemTime.
pub fn local(t: SystemTime) -> DateTime<Local> {
    DateTime::<Local>::from(t)
}

/// strftime-like formatting (chrono): %Y %m %d %H %M %S %y %b %B %j %A …
pub fn fmt_time(t: SystemTime, pattern: &str) -> String {
    let p = if pattern.is_empty() { "%Y-%m-%d" } else { pattern };
    // chrono panics on invalid specifiers when displayed: validate first.
    let items: Vec<chrono::format::Item> = chrono::format::StrftimeItems::new(p).collect();
    if items.iter().any(|i| matches!(i, chrono::format::Item::Error)) {
        return p.to_string();
    }
    local(t).format_with_items(items.into_iter()).to_string()
}

/// Parse "2026-11-06", "2026-11-06 19:30", "2026-11-06T19:30:15", "06/11/2026 19:30" (local time).
pub fn parse_local(s: &str) -> Option<SystemTime> {
    let s = s.trim();
    let fmts = ["%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y:%m:%d %H:%M:%S", "%d/%m/%Y %H:%M", "%d/%m/%Y %H:%M:%S"];
    for f in fmts {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, f) {
            return Local.from_local_datetime(&dt).earliest().map(SystemTime::from);
        }
    }
    for f in ["%Y-%m-%d", "%d/%m/%Y", "%Y:%m:%d"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, f) {
            return Local.from_local_datetime(&d.and_hms_opt(12, 0, 0)?).earliest().map(SystemTime::from);
        }
    }
    None
}

/// Capture date from EXIF (photos) or QuickTime/MP4 `mvhd` (videos), if any.
pub fn capture_date(p: &Path) -> Option<SystemTime> {
    exif_date(p).or_else(|| mp4_date(p))
}

/// EXIF DateTimeOriginal (or DateTimeDigitized / DateTime), as local time.
pub fn exif_date(p: &Path) -> Option<SystemTime> {
    let f = std::fs::File::open(p).ok()?;
    let ex = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f)).ok()?;
    for tag in [exif::Tag::DateTimeOriginal, exif::Tag::DateTimeDigitized, exif::Tag::DateTime] {
        if let Some(field) = ex.get_field(tag, exif::In::PRIMARY) {
            if let exif::Value::Ascii(ref v) = field.value {
                if let Some(s) = v.first() {
                    if let Some(t) = parse_local(&String::from_utf8_lossy(s)) {
                        return Some(t);
                    }
                }
            }
        }
    }
    None
}

/// Camera "Make Model" from EXIF.
pub fn exif_camera(p: &Path) -> Option<String> {
    let f = std::fs::File::open(p).ok()?;
    let ex = exif::Reader::new().read_from_container(&mut std::io::BufReader::new(f)).ok()?;
    let get = |t| {
        ex.get_field(t, exif::In::PRIMARY).and_then(|f| match &f.value {
            exif::Value::Ascii(v) => v.first().map(|s| String::from_utf8_lossy(s).trim().trim_matches('\0').to_string()),
            _ => None,
        })
    };
    let make = get(exif::Tag::Make).unwrap_or_default();
    let model = get(exif::Tag::Model).unwrap_or_default();
    let s = if !make.is_empty() && model.to_lowercase().starts_with(&make.to_lowercase().split_whitespace().next().unwrap_or("").to_string()) {
        model
    } else {
        format!("{make} {model}").trim().to_string()
    };
    (!s.is_empty()).then_some(s)
}

/// Creation time from an MP4/MOV `moov/mvhd` box (seconds since 1904, UTC).
pub fn mp4_date(p: &Path) -> Option<SystemTime> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    let mut pos = 0u64;
    let rd = |f: &mut std::fs::File, n: usize| -> Option<Vec<u8>> {
        let mut b = vec![0; n];
        f.read_exact(&mut b).ok()?;
        Some(b)
    };
    // Walk top-level boxes to moov, then its children to mvhd.
    let mut end = len;
    let mut inside_moov = false;
    for _ in 0..10_000 {
        if pos + 8 > end {
            return None;
        }
        f.seek(SeekFrom::Start(pos)).ok()?;
        let h = rd(&mut f, 8)?;
        let mut size = u32::from_be_bytes(h[0..4].try_into().ok()?) as u64;
        let typ = &h[4..8];
        let mut hdr = 8;
        if size == 1 {
            size = u64::from_be_bytes(rd(&mut f, 8)?.try_into().ok()?);
            hdr = 16;
        } else if size == 0 {
            size = end - pos;
        }
        if size < hdr {
            return None;
        }
        if typ == b"moov" && !inside_moov {
            inside_moov = true;
            end = pos + size;
            pos += hdr;
            continue;
        }
        if typ == b"mvhd" && inside_moov {
            let v = rd(&mut f, 4)?;
            let secs = if v[0] == 1 { u64::from_be_bytes(rd(&mut f, 8)?.try_into().ok()?) } else { u32::from_be_bytes(rd(&mut f, 4)?.try_into().ok()?) as u64 };
            if secs < 2_082_844_800 + 86_400 {
                return None; // zero / unset
            }
            return Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs - 2_082_844_800));
        }
        pos += size;
    }
    None
}

/// File times (created may be missing on some file systems).
pub fn times(p: &Path) -> (Option<SystemTime>, Option<SystemTime>) {
    let m = std::fs::metadata(p).ok();
    (m.as_ref().and_then(|m| m.created().ok()), m.as_ref().and_then(|m| m.modified().ok()))
}

pub fn to_nanos(t: SystemTime) -> i128 {
    match t.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as i128,
        Err(e) => -(e.duration().as_nanos() as i128),
    }
}

pub fn from_nanos(n: i128) -> SystemTime {
    if n >= 0 {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(n as u64)
    } else {
        SystemTime::UNIX_EPOCH - std::time::Duration::from_nanos((-n) as u64)
    }
}

/// Set created and/or modified time of a file.
pub fn set_times(p: &Path, created: Option<SystemTime>, modified: Option<SystemTime>) -> Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
    let mut t = std::fs::FileTimes::new();
    if let Some(m) = modified {
        t = t.set_modified(m);
    }
    #[cfg(windows)]
    if let Some(c) = created {
        use std::os::windows::fs::FileTimesExt;
        t = t.set_created(c);
    }
    #[cfg(target_os = "macos")]
    if let Some(c) = created {
        use std::os::macos::fs::FileTimesExt;
        t = t.set_created(c);
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = created;
    f.set_times(t).map_err(|e| anyhow!("{}: {e}", p.display()))?;
    Ok(())
}

// ---------------------------------------------------------------- text

/// Glob-like match: `*` any run, `?` one character, case-insensitive. Several patterns
/// separated by commas or semicolons.
pub fn wildcard_any(patterns: &str, name: &str) -> bool {
    patterns.split([',', ';', '\n']).map(str::trim).filter(|p| !p.is_empty()).any(|p| wildcard(&p.to_lowercase(), &name.to_lowercase()))
}

pub fn wildcard(p: &str, s: &str) -> bool {
    let (p, s): (Vec<char>, Vec<char>) = (p.chars().collect(), s.chars().collect());
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wildcards() {
        assert!(wildcard_any("*.tmp, ~*", "x.TMP"));
        assert!(wildcard_any("*.tmp, ~*", "~lock"));
        assert!(!wildcard_any("*.tmp", "x.tmpl"));
        assert!(wildcard("a?c*", "abcdef"));
    }
    #[test]
    fn dates() {
        let t = parse_local("2026-11-06 19:30").unwrap();
        assert_eq!(fmt_time(t, "%Y-%m-%d %H:%M"), "2026-11-06 19:30");
        assert_eq!(fmt_time(t, "%Q"), "%Q");
        assert_eq!(from_nanos(to_nanos(t)), t);
    }
}
