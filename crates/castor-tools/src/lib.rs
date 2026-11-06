//! The registry of every tool, shared by the app, the CLI and the MCP server,
//! plus file thumbnails for the UI.

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
/// Images today; PDF pages and video frames are wired in once engine-pdf / engine-media
/// expose their `thumbnail` functions (coordinator task after wave 1).
/// Call it off the UI thread and cache the result.
pub fn thumbnail(f: &FileInfo, max_side: u32) -> Option<Vec<u8>> {
    match f.kind {
        FileKind::Image => {
            let img = image::ImageReader::open(&f.path).ok()?.with_guessed_format().ok()?.decode().ok()?;
            let t = img.thumbnail(max_side, max_side);
            let mut out = std::io::Cursor::new(Vec::new());
            t.write_to(&mut out, image::ImageFormat::Png).ok()?;
            Some(out.into_inner())
        }
        _ => None,
    }
}
