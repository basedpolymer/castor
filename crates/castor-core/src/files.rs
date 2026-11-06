//! Input files and kind detection (extension first, magic bytes as a fallback).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Pdf,
    Image,
    /// RAW camera files (also `Image` for tools that decode them).
    Raw,
    Svg,
    Video,
    Audio,
    /// Word, Excel, PowerPoint, OpenDocument.
    Office,
    /// EPUB, MOBI, AZW3, FB2.
    Ebook,
    Html,
    Markdown,
    Text,
    /// CSV / TSV.
    Table,
    Subtitles,
    Archive,
    Font,
    Other,
    /// Wildcard used in `InputSpec`; never the kind of a real file.
    Any,
}

const EXT: &[(FileKind, &[&str])] = &[
    (FileKind::Pdf, &["pdf"]),
    (
        FileKind::Image,
        &[
            "jpg", "jpeg", "jfif", "png", "gif", "webp", "avif", "heic", "heif", "bmp", "tif", "tiff", "ico", "tga",
            "qoi", "dds", "pnm", "pbm", "pgm", "ppm", "pam", "exr", "hdr", "jxl", "icns",
        ],
    ),
    (FileKind::Raw, &["cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "raf", "srw", "pef", "nrw", "3fr", "iiq"]),
    (FileKind::Svg, &["svg", "svgz"]),
    (
        FileKind::Video,
        &["mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "mpg", "mpeg", "ts", "mts", "m2ts", "3gp", "ogv"],
    ),
    (
        FileKind::Audio,
        &["mp3", "wav", "flac", "aac", "m4a", "ogg", "oga", "opus", "wma", "aiff", "aif", "alac", "caf", "amr"],
    ),
    (FileKind::Office, &["doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "rtf", "pages", "key", "numbers"]),
    (FileKind::Ebook, &["epub", "mobi", "azw", "azw3", "fb2", "fbz"]),
    (FileKind::Html, &["html", "htm", "xhtml", "mhtml"]),
    (FileKind::Markdown, &["md", "markdown"]),
    (FileKind::Text, &["txt", "log", "json", "xml", "yaml", "yml", "toml", "ini", "tex", "typ"]),
    (FileKind::Table, &["csv", "tsv"]),
    (FileKind::Subtitles, &["srt", "vtt", "ass", "ssa", "sub"]),
    (FileKind::Archive, &["zip", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "rar"]),
    (FileKind::Font, &["ttf", "otf", "woff", "woff2", "ttc"]),
];

impl FileKind {
    pub fn from_extension(ext: &str) -> FileKind {
        let ext = ext.to_ascii_lowercase();
        EXT.iter().find(|(_, list)| list.contains(&ext.as_str())).map(|(k, _)| *k).unwrap_or(FileKind::Other)
    }

    pub fn from_mime(mime: &str) -> FileKind {
        match mime {
            "application/pdf" => FileKind::Pdf,
            "image/svg+xml" => FileKind::Svg,
            m if m.starts_with("image/") => FileKind::Image,
            m if m.starts_with("video/") => FileKind::Video,
            m if m.starts_with("audio/") => FileKind::Audio,
            "application/epub+zip" | "application/x-mobipocket-ebook" => FileKind::Ebook,
            "application/zip" | "application/x-7z-compressed" | "application/x-tar" | "application/gzip"
            | "application/x-bzip2" | "application/x-xz" | "application/zstd" | "application/vnd.rar" => {
                FileKind::Archive
            }
            m if m.starts_with("font/") || m == "application/font-woff" => FileKind::Font,
            m if m.contains("officedocument") || m.contains("opendocument") || m == "application/msword" => {
                FileKind::Office
            }
            "text/html" => FileKind::Html,
            _ => FileKind::Other,
        }
    }

    /// Image-like kinds that an image tool can usually decode.
    pub fn is_image_like(self) -> bool {
        matches!(self, FileKind::Image | FileKind::Raw | FileKind::Svg)
    }
}

/// A file (or folder) given to a tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: PathBuf,
    pub kind: FileKind,
    pub size: u64,
    pub is_dir: bool,
}

impl FileInfo {
    /// Inspect a path. Never fails: unreadable files get kind Other and size 0.
    pub fn from_path(path: impl AsRef<Path>) -> FileInfo {
        let path = path.as_ref().to_path_buf();
        let md = std::fs::metadata(&path).ok();
        let is_dir = md.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        let size = md.as_ref().map(|m| if m.is_dir() { 0 } else { m.len() }).unwrap_or(0);
        let mut kind = if is_dir {
            FileKind::Other
        } else {
            FileKind::from_extension(path.extension().and_then(|e| e.to_str()).unwrap_or(""))
        };
        if kind == FileKind::Other && !is_dir {
            if let Ok(Some(t)) = infer::get_from_path(&path) {
                kind = FileKind::from_mime(t.mime_type());
            }
        }
        FileInfo { path, kind, size, is_dir }
    }

    pub fn name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
    pub fn stem(&self) -> String {
        self.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
    /// Lowercase extension without dot ("" if none).
    pub fn extension(&self) -> String {
        self.path.extension().map(|s| s.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
    }
}

/// Expand folders into the files they contain (recursively), keeping plain files as they are.
/// Hidden files (dot-files) and OS clutter (Thumbs.db, desktop.ini, .DS_Store) are skipped.
pub fn expand(paths: &[PathBuf], recursive: bool) -> Vec<FileInfo> {
    fn walk(dir: &Path, recursive: bool, out: &mut Vec<FileInfo>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        entries.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
        for p in entries {
            let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if name.starts_with('.') || name.eq_ignore_ascii_case("thumbs.db") || name.eq_ignore_ascii_case("desktop.ini") {
                continue;
            }
            if p.is_dir() {
                if recursive {
                    walk(&p, recursive, out);
                }
            } else {
                out.push(FileInfo::from_path(&p));
            }
        }
    }
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() { walk(p, recursive, &mut out) } else { out.push(FileInfo::from_path(p)) }
    }
    out
}

/// Natural ("human") ordering: "page2" < "page10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let mut na = String::new();
                while let Some(c) = ai.peek().copied().filter(|c| c.is_ascii_digit()) {
                    na.push(c);
                    ai.next();
                }
                let mut nb = String::new();
                while let Some(c) = bi.peek().copied().filter(|c| c.is_ascii_digit()) {
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                let o = x.to_lowercase().cmp(y.to_lowercase());
                if o != Ordering::Equal {
                    return o;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn natural() {
        let mut v = vec!["p10", "p2", "P1"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["P1", "p2", "p10"]);
    }
    #[test]
    fn kinds() {
        assert_eq!(FileKind::from_extension("JPG"), FileKind::Image);
        assert_eq!(FileKind::from_extension("epub"), FileKind::Ebook);
    }
}
