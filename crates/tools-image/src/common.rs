//! Helpers shared by the image tools: metadata builders, loading, saving, notes, previews.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Preview, Requirement, RunCx, ToolMeta, ToolStatus};
use image::{DynamicImage, RgbaImage};

use crate::codec::{self, DecodeOpts, Decoded, EncodeOpts, Fmt};

/// Every image-like file.
pub const IMAGE_INPUTS: InputSpec = InputSpec::kinds(&[FileKind::Image, FileKind::Raw, FileKind::Svg]);

pub fn meta(
    id: &'static str,
    output: &'static str,
    keywords: &'static [&'static str],
    suffix: &'static str,
    status: ToolStatus,
    requires: &'static [Requirement],
    serial: bool,
) -> ToolMeta {
    ToolMeta {
        id,
        family: castor_core::Family::Image,
        inputs: IMAGE_INPUTS,
        output,
        keywords,
        status,
        requires,
        suffix,
        in_place: false,
        serial,
    }
}

/// Localised error text with the file name.
pub fn err(key: &str, path: &Path) -> String {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    castor_core::tr_args(&format!("image.{key}"), &[("file", &name)])
}

pub fn load(f: &FileInfo, o: DecodeOpts) -> Result<Decoded> {
    codec::decode(&f.path, o)
}

/// Write bytes to a fresh output path (never overwrites; clear error on read-only folders).
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).with_context(|| err("err.write", path))?;
    }
    std::fs::write(path, bytes).with_context(|| err("err.write", path))
}

/// "1.2 MB → 300 KB (−75 %)".
pub fn size_note(before: u64, after: u64) -> String {
    format!("{} → {} ({})", castor_core::units::bytes(before), castor_core::units::bytes(after), castor_core::units::change(before, after))
}

/// Output format option shared by tools that write images ("keep" = same as input).
pub fn format_option(default: &'static str) -> OptionSpec {
    OptionSpec::choice(
        "format",
        vec![
            Choice::key("keep"),
            Choice::lit("jpg", "JPG"),
            Choice::lit("png", "PNG"),
            Choice::lit("webp", "WebP"),
            Choice::lit("avif", "AVIF"),
        ],
        default,
    )
}

pub fn quality_option(default: i64) -> OptionSpec {
    OptionSpec::int("quality", 1, 100, default).slider().unit("%").common()
}

/// The format to write for an input, from a "format" value ("keep" falls back to PNG/JPEG
/// for formats we cannot write).
pub fn out_format(values: &Values, input: Fmt, has_alpha: bool) -> Fmt {
    let v = values.str("format");
    match v {
        "" | "keep" => keep_format(input, has_alpha),
        other => Fmt::from_ext(other).filter(|f| f.writable()).unwrap_or(Fmt::Png),
    }
}

/// Same format if we can write it, else PNG (alpha / lossless sources) or JPEG (photos).
pub fn keep_format(input: Fmt, has_alpha: bool) -> Fmt {
    match input {
        f if f.writable() && f != Fmt::Ico && f != Fmt::Heic => f,
        Fmt::Heic | Fmt::Raw => {
            if has_alpha {
                Fmt::Png
            } else {
                Fmt::Jpeg
            }
        }
        _ => Fmt::Png,
    }
}

/// Encoder options for re-saving an edited image: keeps ICC (+ EXIF/XMP if asked; EXIF
/// orientation reset because pixels are now upright) and the source's JPEG quality.
pub fn resave_opts(d: &Decoded, src: &Path, quality: Option<u8>, keep_meta: bool) -> EncodeOpts {
    let q = quality.or_else(|| if d.format == Fmt::Jpeg { std::fs::read(src).ok().and_then(|b| crate::jpegq::estimate(&b)) } else { None });
    EncodeOpts {
        quality: q.unwrap_or(90).clamp(1, 100),
        icc: d.icc.clone(),
        exif: if keep_meta { d.exif.as_deref().map(crate::meta::reset_orientation) } else { None },
        xmp: if keep_meta { d.xmp.clone() } else { None },
        ..Default::default()
    }
}

/// Encode and write `img` next to `input` with this format; returns the path.
pub fn save(img: &DynamicImage, fmt: Fmt, o: &EncodeOpts, input: Option<&FileInfo>, cx: &RunCx) -> Result<PathBuf> {
    let bytes = codec::encode(img, fmt, o)?;
    let out = cx.output_path(input, fmt.ext());
    write(&out, &bytes)?;
    Ok(out)
}

/// PNG preview of at most `max_side` pixels.
pub fn preview(img: &RgbaImage, max_side: u32, caption: Option<String>) -> Result<Option<Preview>> {
    let (w, h) = img.dimensions();
    let small = if w.max(h) > max_side {
        let (tw, th) = crate::ops::fit_size(w, h, max_side, max_side);
        crate::ops::resize_rgba(img, tw, th)
    } else {
        img.clone()
    };
    let mut out = std::io::Cursor::new(Vec::new());
    small.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(Some(Preview { png: out.into_inner(), caption }))
}

/// Decode an input for a preview: small and fast (thumbnail-sized work copy).
pub fn load_preview(f: &FileInfo, max_side: u32) -> Result<DynamicImage> {
    let d = codec::decode(&f.path, DecodeOpts { orient: true, frames: false, svg_side: max_side.max(256) })?;
    let img = d.image;
    let limit = (max_side * 2).max(512);
    Ok(if img.width().max(img.height()) > limit {
        let (w, h) = crate::ops::fit_size(img.width(), img.height(), limit, limit);
        crate::ops::resize_dyn(&img, w, h)
    } else {
        img
    })
}

/// "1920 × 1080".
pub fn dims(w: u32, h: u32) -> String {
    format!("{w} × {h}")
}

/// The FileInfo for a path we created (for chaining helpers).
pub fn info(p: &Path) -> FileInfo {
    FileInfo::from_path(p)
}
