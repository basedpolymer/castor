//! Helpers shared by the PDF tools.

use std::path::{Path, PathBuf};

use castor_core::options::{parse_page_range, OptionSpec, Values};
use castor_core::{units, Family, FileKind, InputSpec, Requirement, Result, RunCx, ToolMeta, ToolStatus};

pub use engine_pdf::mupdf;

pub const PDF_IN: InputSpec = InputSpec::kinds(&[FileKind::Pdf]);

/// ToolMeta with the usual defaults.
pub fn meta(
    id: &'static str,
    family: Family,
    inputs: InputSpec,
    output: &'static str,
    keywords: &'static [&'static str],
    suffix: &'static str,
) -> ToolMeta {
    ToolMeta {
        id,
        family,
        inputs,
        output,
        keywords,
        status: ToolStatus::Ready,
        requires: &[] as &[Requirement],
        suffix,
        in_place: false,
        serial: false,
    }
}

pub fn tr(key: &str) -> String {
    castor_core::tr(key)
}

pub fn tr_args(key: &str, args: &[(&str, &str)]) -> String {
    castor_core::tr_args(key, args)
}

/// The shared, advanced "password" option for encrypted inputs.
pub fn password_opt() -> OptionSpec {
    OptionSpec::password("password").common().advanced()
}

pub fn password(v: &Values) -> &str {
    v.str("password")
}

/// "19 MB → 212 KB (−98 %)".
pub fn size_change(before: u64, after: u64) -> String {
    format!("{} → {} ({})", units::bytes(before), units::bytes(after), units::change(before, after))
}

pub fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// 0-based page indices from a page-range option; error if nothing is selected.
pub fn pages(v: &Values, key: &str, count: usize) -> Result<Vec<usize>> {
    let sel = parse_page_range(v.str(key), count).map_err(|e| anyhow::anyhow!("{}: {e}", tr("pdf.err.pages")))?;
    anyhow::ensure!(!sel.is_empty(), "{}", tr("pdf.err.no_pages"));
    Ok(sel)
}

/// Output path for one input with the tool suffix.
pub fn out_for(cx: &RunCx, input: &castor_core::FileInfo, ext: &str) -> PathBuf {
    cx.output_path(Some(input), ext)
}

/// Progress for page `i` of `n` inside the `[from, to]` part of the job.
pub fn prog(cx: &RunCx, i: usize, n: usize, from: f32, to: f32) {
    cx.progress(from + (to - from) * (i as f32 / n.max(1) as f32));
}

/// Page message "Page 3 of 12".
pub fn page_msg(cx: &RunCx, i: usize, n: usize) {
    cx.message(tr_args("pdf.msg.page", &[("n", &(i + 1).to_string()), ("total", &n.to_string())]));
}

/// mm → points.
pub fn mm(v: f64) -> f32 {
    (v * 72.0 / 25.4) as f32
}

/// Remove a partially written output on error.
pub struct Cleanup(pub Vec<PathBuf>, pub bool);
impl Cleanup {
    pub fn new() -> Self {
        Cleanup(Vec::new(), false)
    }
    pub fn add(&mut self, p: &Path) {
        self.0.push(p.to_path_buf());
    }
    pub fn keep(mut self) {
        self.1 = true;
    }
}
impl Drop for Cleanup {
    fn drop(&mut self) {
        if !self.1 {
            for p in &self.0 {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// Parse a hex colour option into (rgb 0..1, alpha 0..1).
pub fn color(v: &Values, key: &str) -> ([f32; 3], f32) {
    engine_pdf::draw::color_from_rgba(v.color(key))
}

/// Text substitution for page labels: {n}, {total}, {roman}, {ROMAN}.
pub fn roman(mut n: usize, upper: bool) -> String {
    if n == 0 {
        return "0".into();
    }
    const T: &[(usize, &str)] = &[
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= *v {
            s.push_str(r);
            n -= v;
        }
    }
    if upper { s.to_uppercase() } else { s }
}

/// Anchor position of a box of size (w, h) inside `area` with `margin`.
pub fn anchor_pos(a: castor_core::options::Anchor, area: mupdf::Rect, w: f32, h: f32, margin: f32) -> (f32, f32) {
    let (gx, gy) = a.grid();
    let x = match gx {
        0 => area.x0 + margin,
        1 => (area.x0 + area.x1 - w) / 2.0,
        _ => area.x1 - margin - w,
    };
    let y = match gy {
        0 => area.y0 + margin,
        1 => (area.y0 + area.y1 - h) / 2.0,
        _ => area.y1 - margin - h,
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn romans() {
        assert_eq!(roman(1994, false), "mcmxciv");
        assert_eq!(roman(4, true), "IV");
    }
}
