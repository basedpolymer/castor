//! `files.archive`: ZIP (Deflate or Zstandard, optional AES-256 password), 7z (LZMA2, optional
//! AES-256 with encrypted file names), TAR, TAR.GZ, TAR.ZST, TAR.XZ. Exclude patterns, split
//! volumes (.001, .002… — opened by 7-Zip and by `files.extract`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, RunCx, Tool, ToolMeta, tr, tr_args, units};

use crate::common;

pub struct Archive {
    meta: ToolMeta,
}

impl Archive {
    pub fn new() -> Self {
        Archive {
            meta: common::meta(
                "files.archive",
                InputSpec::kinds(common::ANY).folders(),
                "ZIP · 7Z · TAR.GZ · TAR.ZST",
                &["zip", "compress", "archive", "7z", "tar", "gz", "zstd", "password", "split", "backup"],
                "",
                false,
            ),
        }
    }
}

impl Tool for Archive {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![
                    Choice::lit("zip", "ZIP"),
                    Choice::lit("7z", "7Z"),
                    Choice::lit("tar.gz", "TAR.GZ"),
                    Choice::lit("tar.zst", "TAR.ZST"),
                    Choice::lit("tar.xz", "TAR.XZ"),
                    Choice::lit("tar", "TAR"),
                ],
                "zip",
            ),
            OptionSpec::choice("level", ["store", "fast", "normal", "max"].map(Choice::key).to_vec(), "normal"),
            OptionSpec::toggle("zstd", false).when(Cond::Eq("format", "zip")).advanced(),
            OptionSpec::password("password"),
            OptionSpec::text("name", ""),
            OptionSpec::toggle("each", false),
            OptionSpec::text("exclude", ".DS_Store, Thumbs.db, desktop.ini, *.tmp, ~$*"),
            OptionSpec::int("split_mb", 0, 100_000, 0).unit("MB").advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", tr("files.err.no_files"));
        Ok(if v.bool("each") { inputs.iter().cloned().map(Job::single).collect() } else { vec![Job::many(inputs.to_vec())] })
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let fmt = v.str("format");
        let pw = v.str("password");
        anyhow::ensure!(pw.is_empty() || matches!(fmt, "zip" | "7z"), "{}", tr("files.archive.err.password_format"));
        let entries = collect(&job.inputs, v.str("exclude"))?;
        anyhow::ensure!(entries.iter().any(|e| !e.is_dir), "{}", tr("files.archive.err.empty"));
        let total: u64 = entries.iter().map(|e| e.size).sum::<u64>().max(1);
        let name = archive_name(&job.inputs, v.str("name"));
        let out = cx.output_named(&format!("{name}.{fmt}"));
        let tmp = out.with_file_name(format!(
            ".{}.castor-tmp",
            out.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
        ));
        let res = (|| -> Result<()> {
            match fmt {
                "7z" => write_7z(&tmp, &entries, v, pw, total, cx),
                "zip" => write_zip(&tmp, &entries, v, pw, total, cx),
                _ => write_tar(&tmp, &entries, fmt, v.str("level"), total, cx),
            }
        })();
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        std::fs::rename(&tmp, &out)?;
        let size = std::fs::metadata(&out)?.len();
        let files = entries.iter().filter(|e| !e.is_dir).count();
        let note = tr_args(
            "files.archive.note",
            &[("files", &files.to_string()), ("before", &units::bytes(total)), ("after", &units::bytes(size)), ("change", &units::change(total, size))],
        );
        let split = v.int("split_mb").max(0) as u64 * 1_000_000;
        if split > 0 && size > split {
            let parts = split_file(&out, split, cx)?;
            return Ok(parts.into_iter().enumerate().map(|(i, p)| if i == 0 { Output::new(p).with_note(note.clone()) } else { Output::new(p) }).collect());
        }
        Ok(vec![Output::new(out).with_note(note)])
    }
}

/// One thing to put in the archive.
pub struct Entry {
    pub path: PathBuf,
    /// Name inside the archive, '/' separated.
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Walk the inputs. Dropped folders keep their own name as the top folder; loose files that
/// share a parent are stored relative to their common folder.
pub fn collect(inputs: &[FileInfo], exclude: &str) -> Result<Vec<Entry>> {
    let mut out = Vec::new();
    let loose: Vec<&FileInfo> = inputs.iter().filter(|f| !f.is_dir).collect();
    let base = common_parent(&loose.iter().map(|f| f.path.as_path()).collect::<Vec<_>>());
    for f in &loose {
        if common::wildcard_any(exclude, &f.name()) {
            continue;
        }
        let rel = base.as_ref().and_then(|b| f.path.strip_prefix(b).ok()).map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(f.name()));
        out.push(Entry { path: f.path.clone(), name: slash(&rel), is_dir: false, size: f.size });
    }
    for d in inputs.iter().filter(|f| f.is_dir) {
        let top = d.name();
        let mut stack = vec![d.path.clone()];
        out.push(Entry { path: d.path.clone(), name: format!("{top}/"), is_dir: true, size: 0 });
        while let Some(dir) = stack.pop() {
            let mut items: Vec<_> = std::fs::read_dir(&dir).map_err(|e| anyhow!("{}: {e}", dir.display()))?.flatten().collect();
            items.sort_by_key(|e| e.file_name());
            for e in items {
                let p = e.path();
                let n = e.file_name().to_string_lossy().into_owned();
                if common::wildcard_any(exclude, &n) {
                    continue;
                }
                let rel = format!("{top}/{}", slash(p.strip_prefix(&d.path).unwrap_or(&p)));
                let md = std::fs::symlink_metadata(&p)?;
                if md.file_type().is_symlink() {
                    continue; // never follow links out of the folder
                }
                if md.is_dir() {
                    out.push(Entry { path: p.clone(), name: format!("{rel}/"), is_dir: true, size: 0 });
                    stack.push(p);
                } else {
                    out.push(Entry { path: p, name: rel, is_dir: false, size: md.len() });
                }
            }
        }
    }
    // Same name twice (two loose files from different disks): keep both, rename the second.
    let mut seen = std::collections::HashSet::new();
    for e in &mut out {
        let mut n = 2;
        let orig = e.name.clone();
        while !seen.insert(e.name.to_lowercase()) {
            e.name = match orig.rfind('.') {
                Some(i) if !e.is_dir => format!("{} ({n}){}", &orig[..i], &orig[i..]),
                _ => format!("{orig} ({n})"),
            };
            n += 1;
        }
    }
    Ok(out)
}

fn slash(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn common_parent(paths: &[&Path]) -> Option<PathBuf> {
    let first = paths.first()?.parent()?.to_path_buf();
    let mut base = first;
    for p in paths.iter().skip(1) {
        while !p.starts_with(&base) {
            base = base.parent()?.to_path_buf();
        }
    }
    Some(base)
}

fn archive_name(inputs: &[FileInfo], custom: &str) -> String {
    if !custom.trim().is_empty() {
        return castor_core::job::sanitize_file_name(custom.trim().trim_end_matches(".zip").trim_end_matches(".7z"));
    }
    match inputs {
        [one] if one.is_dir => one.name(),
        [one] => one.stem(),
        many => common_parent(&many.iter().map(|f| f.path.as_path()).collect::<Vec<_>>())
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| tr("files.archive.default_name")),
    }
}

/// A reader that reports progress and stops on cancel.
struct Progress<'a, R> {
    inner: R,
    done: &'a mut u64,
    total: u64,
    cx: &'a RunCx,
}

impl<R: Read> Read for Progress<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cx.is_cancelled() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = self.inner.read(buf)?;
        *self.done += n as u64;
        self.cx.progress(*self.done as f32 / self.total as f32);
        Ok(n)
    }
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn write_zip(out: &Path, entries: &[Entry], v: &Values, pw: &str, total: u64, cx: &RunCx) -> Result<()> {
    use zip::write::SimpleFileOptions;
    let f = std::fs::File::create(out)?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let level = v.str("level");
    let (method, lvl) = match (level, v.bool("zstd")) {
        ("store", _) => (zip::CompressionMethod::Stored, None),
        ("fast", false) => (zip::CompressionMethod::Deflated, Some(1)),
        ("max", false) => (zip::CompressionMethod::Deflated, Some(9)),
        (_, false) => (zip::CompressionMethod::Deflated, Some(6)),
        ("fast", true) => (zip::CompressionMethod::Zstd, Some(1)),
        ("max", true) => (zip::CompressionMethod::Zstd, Some(19)),
        (_, true) => (zip::CompressionMethod::Zstd, Some(6)),
    };
    let mut done = 0u64;
    for e in entries {
        cx.check_cancel()?;
        let mut o = SimpleFileOptions::default().compression_method(method).compression_level(lvl).large_file(e.size >= u32::MAX as u64);
        if let Some(t) = mtime(&e.path) {
            let odt = time::OffsetDateTime::from(t);
            if let Ok(dt) = zip::DateTime::try_from(time::PrimitiveDateTime::new(odt.date(), odt.time())) {
                o = o.last_modified_time(dt);
            }
        }
        if !pw.is_empty() {
            o = o.with_aes_encryption(zip::AesMode::Aes256, pw);
        }
        if e.is_dir {
            z.add_directory(e.name.trim_end_matches('/'), o)?;
            continue;
        }
        z.start_file(e.name.as_str(), o)?;
        let f = std::fs::File::open(&e.path).map_err(|err| anyhow!("{}: {err}", e.path.display()))?;
        let mut r = Progress { inner: f, done: &mut done, total, cx };
        std::io::copy(&mut r, &mut z).map_err(|err| cancelled_or(err, cx))?;
    }
    z.finish()?.flush()?;
    Ok(())
}

fn cancelled_or(e: std::io::Error, cx: &RunCx) -> anyhow::Error {
    if cx.is_cancelled() { castor_core::job::Cancelled.into() } else { anyhow!("{e}") }
}

fn write_7z(out: &Path, entries: &[Entry], v: &Values, pw: &str, total: u64, cx: &RunCx) -> Result<()> {
    use sevenz_rust2::encoder_options::{AesEncoderOptions, Lzma2Options};
    use sevenz_rust2::{ArchiveEntry, ArchiveWriter, EncoderConfiguration, EncoderMethod, Password};
    let mut w = ArchiveWriter::create(out).map_err(|e| anyhow!("{e}"))?;
    let lzma: EncoderConfiguration = match v.str("level") {
        "store" => EncoderMethod::COPY.into(),
        "fast" => Lzma2Options::from_level(1).into(),
        "max" => Lzma2Options::from_level(9).into(),
        _ => Lzma2Options::from_level(6).into(),
    };
    if pw.is_empty() {
        w.set_content_methods(vec![lzma]);
    } else {
        w.set_content_methods(vec![AesEncoderOptions::new(Password::new(pw)).into(), lzma]);
        w.set_encrypt_header(true);
    }
    let mut done = 0u64;
    for e in entries {
        cx.check_cancel()?;
        let entry = ArchiveEntry::from_path(&e.path, e.name.trim_end_matches('/').to_string());
        if e.is_dir {
            w.push_archive_entry::<std::fs::File>(entry, None).map_err(|err| anyhow!("{err}"))?;
        } else {
            let f = std::fs::File::open(&e.path).map_err(|err| anyhow!("{}: {err}", e.path.display()))?;
            let r = Progress { inner: f, done: &mut done, total, cx };
            w.push_archive_entry(entry, Some(r)).map_err(|err| if cx.is_cancelled() { castor_core::job::Cancelled.into() } else { anyhow!("{err}") })?;
        }
    }
    w.finish().map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn write_tar(out: &Path, entries: &[Entry], fmt: &str, level: &str, total: u64, cx: &RunCx) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(out)?);
    let lvl = |fast: u32, normal: u32, max: u32| match level {
        "store" => 0,
        "fast" => fast,
        "max" => max,
        _ => normal,
    };
    let w: Box<dyn Write> = match fmt {
        "tar.gz" => Box::new(flate2::write::GzEncoder::new(f, flate2::Compression::new(lvl(1, 6, 9)))),
        "tar.zst" => {
            Box::new(zstd::stream::write::Encoder::new(f, lvl(1, 6, 19) as i32)?.auto_finish())
        }
        "tar.xz" => {
            let opts = lzma_rust2::XzOptions::with_preset(lvl(1, 6, 9));
            Box::new(lzma_rust2::XzWriter::new(f, opts)?.auto_finish())
        }
        _ => Box::new(f),
    };
    let mut b = tar::Builder::new(w);
    b.follow_symlinks(false);
    let mut done = 0u64;
    for e in entries {
        cx.check_cancel()?;
        if e.is_dir {
            b.append_dir(e.name.trim_end_matches('/'), &e.path)?;
            continue;
        }
        let f = std::fs::File::open(&e.path).map_err(|err| anyhow!("{}: {err}", e.path.display()))?;
        let md = f.metadata()?;
        let mut h = tar::Header::new_gnu();
        h.set_metadata(&md);
        h.set_size(md.len());
        let mut r = Progress { inner: f, done: &mut done, total, cx };
        b.append_data(&mut h, &e.name, &mut r).map_err(|err| cancelled_or(err, cx))?;
    }
    let mut w = b.into_inner()?;
    w.flush()?;
    drop(w);
    Ok(())
}

/// Split a file into `name.001`, `name.002`… of `size` bytes and remove the original.
pub fn split_file(p: &Path, size: u64, cx: &RunCx) -> Result<Vec<PathBuf>> {
    let mut f = std::fs::File::open(p)?;
    let total = f.metadata()?.len();
    let mut parts = Vec::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    for i in 1.. {
        if done >= total {
            break;
        }
        cx.check_cancel()?;
        let part = PathBuf::from(format!("{}.{i:03}", p.display()));
        anyhow::ensure!(!part.exists(), "{}", tr_args("files.err.exists", &[("name", &part.display().to_string())]));
        let mut w = std::io::BufWriter::new(std::fs::File::create(&part)?);
        let mut left = size.min(total - done);
        while left > 0 {
            let n = f.read(&mut buf[..left.min(1 << 20) as usize])?;
            anyhow::ensure!(n > 0, "unexpected end of file while splitting");
            w.write_all(&buf[..n])?;
            left -= n as u64;
            done += n as u64;
        }
        w.flush()?;
        parts.push(part);
    }
    drop(f);
    std::fs::remove_file(p)?;
    Ok(parts)
}
