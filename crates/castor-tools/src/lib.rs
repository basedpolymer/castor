//! The registry of every tool, shared by the app, the CLI and the MCP server,
//! plus file thumbnails for the UI.

pub mod downloads;

use castor_core::{FileInfo, FileKind, Registry};

pub fn registry() -> Registry {
    let mut all = Vec::new();
    all.extend(tools_pdf::tools());
    all.extend(tools_image::tools());
    all.extend(tools_media::tools());
    all.extend(tools_create::tools());
    all.extend(tools_files::tools());
    Registry::new(all)
}

/// PNG thumbnail of a file, at most `max_side` pixels, or None if this kind has no preview.
/// Images (every format Castor decodes: HEIC, AVIF, JXL, RAW, SVG…), PDF first pages, video
/// frames and audio covers / waveforms. Call it off the UI thread and cache the result.
pub fn thumbnail(f: &FileInfo, max_side: u32) -> Option<Vec<u8>> {
    match f.kind {
        FileKind::Image | FileKind::Raw | FileKind::Svg => image_thumbnail(f, max_side),
        FileKind::Pdf => engine_pdf::thumbnail(&f.path, 0, max_side).ok(),
        FileKind::Video | FileKind::Audio => engine_media::thumbnail(&f.path, None, max_side).ok(),
        _ => None,
    }
}

fn image_thumbnail(f: &FileInfo, max_side: u32) -> Option<Vec<u8>> {
    let opts = tools_image::codec::DecodeOpts { orient: true, frames: false, svg_side: max_side.max(256), ..Default::default() };
    let img = match tools_image::codec::decode(&f.path, opts) {
        Ok(d) => d.image,
        // Fall back to the plain `image` decoder (never worse than nothing).
        Err(_) => image::ImageReader::open(&f.path).ok()?.with_guessed_format().ok()?.decode().ok()?,
    };
    let t = img.thumbnail(max_side, max_side);
    let mut out = std::io::Cursor::new(Vec::new());
    t.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}
