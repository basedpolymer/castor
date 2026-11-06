//! Small helpers shared by the create tools.

use std::path::{Path, PathBuf};

use anyhow::Result;
use castor_core::{FileKind, InputSpec, Preview, RunCx, ToolMeta, ToolStatus, Family, Requirement, tr, tr_args};

/// Metadata for a create tool.
pub fn meta(
    id: &'static str,
    inputs: InputSpec,
    output: &'static str,
    keywords: &'static [&'static str],
    suffix: &'static str,
) -> ToolMeta {
    ToolMeta {
        id,
        family: Family::Create,
        inputs,
        output,
        keywords,
        status: ToolStatus::Ready,
        requires: &[],
        suffix,
        in_place: false,
        serial: false,
    }
}

pub fn with_requires(mut m: ToolMeta, r: &'static [Requirement]) -> ToolMeta {
    m.requires = r;
    m
}

pub fn status(mut m: ToolMeta, s: ToolStatus) -> ToolMeta {
    m.status = s;
    m
}

pub fn serial(mut m: ToolMeta) -> ToolMeta {
    m.serial = true;
    m
}

pub const IMAGES: &[FileKind] = &[FileKind::Image, FileKind::Svg];

/// A preview with a "Figure 1 / …" style caption.
pub fn preview(png: Vec<u8>, caption: impl Into<String>) -> Preview {
    let c = caption.into();
    Preview { png, caption: if c.is_empty() { None } else { Some(c) } }
}

/// Write bytes atomically-ish (to a temp name in the same folder, then rename).
pub fn write(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.castor-tmp",
        path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    std::fs::write(&tmp, data).map_err(|e| anyhow::anyhow!("{}: {e}", path.parent().unwrap_or(path).display()))?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Output path for a tool with no input: `{suffix}.{ext}` or `{stem}.{ext}` if a name is given.
pub fn out_named(cx: &RunCx, name: &str, ext: &str) -> PathBuf {
    let name = name.trim();
    if name.is_empty() { cx.output_path(None, ext) } else { cx.output_named(&format!("{}.{ext}", slug_keep_case(name))) }
}

/// File-name friendly text: keeps letters (any script), digits, spaces become '-'.
pub fn slug_keep_case(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
            o.push(c);
        } else if (c.is_whitespace() || c == '/') && !o.ends_with('-') {
            o.push('-');
        }
    }
    let o = o.trim_matches(['-', '.']).chars().take(80).collect::<String>();
    if o.is_empty() { "castor".into() } else { o }
}

/// Lowercase ASCII slug ("Été 2026 — Menu" → "ete-2026-menu").
pub fn slug(s: &str) -> String {
    let folded = castor_core::registry::fold(s);
    let mut o = String::new();
    for c in folded.chars() {
        if c.is_ascii_alphanumeric() {
            o.push(c);
        } else if !o.ends_with('-') && !o.is_empty() {
            o.push('-');
        }
    }
    let o = o.trim_matches('-').chars().take(60).collect::<String>();
    if o.is_empty() { "castor".into() } else { o }
}

/// "Figure 1 / …" caption text.
pub fn figure(text: &str) -> String {
    tr_args("create.common.figure", &[("text", text)])
}

/// Localised "W × H px".
pub fn px(w: u32, h: u32) -> String {
    format!("{w} × {h} px")
}

/// Localised yes/no for scans.
pub fn scans_label(ok: bool) -> String {
    tr(if ok { "create.common.scans" } else { "create.common.no_scan" })
}

/// Standard base64 (for data URIs).
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        o.push(T[(n >> 18) as usize & 63] as char);
        o.push(T[(n >> 12) as usize & 63] as char);
        o.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        o.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    o
}

/// Decode standard base64 (whitespace ignored).
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => anyhow::bail!("invalid base64"),
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Escape text for SVG/XML.
pub fn xml(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// Deterministic pseudo-random numbers (SplitMix64): same seed, same picture.
#[derive(Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn b64() {
        assert_eq!(super::base64(b"Man"), "TWFu");
        assert_eq!(super::base64(b"Ma"), "TWE=");
        assert_eq!(super::base64(b"M"), "TQ==");
        assert_eq!(super::base64_decode("TWFu TWE=").unwrap(), b"ManMa");
    }
    #[test]
    fn slugs() {
        assert_eq!(super::slug("Été 2026 — Menu!"), "ete-2026-menu");
    }
}
