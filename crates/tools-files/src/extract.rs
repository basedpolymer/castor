//! `files.extract`: ZIP (incl. AES / ZipCrypto passwords), 7z (incl. AES), TAR, TAR.GZ, TAR.BZ2,
//! TAR.XZ, TAR.ZST and single .gz/.bz2/.xz/.zst files, split volumes (.001…). The format is
//! read from the content, not the extension. Every entry name is sanitised (no "..", no
//! absolute paths, no links): nothing can be written outside the destination folder.
//! RAR is not supported: the only free RAR decoder (unrar) has a licence incompatible with GPL.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use castor_core::options::{OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, RunCx, Tool, ToolMeta, tr, tr_args, units};

use crate::common;

pub struct Extract {
    meta: ToolMeta,
}

impl Extract {
    pub fn new() -> Self {
        Extract {
            meta: common::meta(
                "files.extract",
                InputSpec::kinds(&[FileKind::Archive]).with_extensions(&["001", "tgz", "tbz", "tbz2", "txz", "tzst", "cbz", "jar", "apk", "epub"]),
                "Folder",
                &["unzip", "extract", "decompress", "zip", "7z", "tar", "gz", "open archive", "uncompress"],
                "",
                false,
            ),
        }
    }
}

impl Tool for Extract {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::password("password"),
            OptionSpec::toggle("smart_folder", true),
            OptionSpec::toggle("keep_dates", true).advanced(),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        // Only the first volume of a split set starts a job.
        let jobs: Vec<Job> = inputs
            .iter()
            .filter(|f| !f.is_dir)
            .filter(|f| !is_later_volume(&f.path))
            .cloned()
            .map(Job::single)
            .collect();
        anyhow::ensure!(!jobs.is_empty(), "{}", tr("files.extract.err.no_archive"));
        Ok(jobs)
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let mut src = Source::open(&input.path)?;
        let kind = sniff(&mut src)?;
        let name = folder_name(&input.path);
        let dest = cx.output_named(&name);
        std::fs::create_dir_all(&dest)?;
        let pw = v.str("password");
        let res = match kind {
            Kind::Zip => zip_extract(src, &dest, pw, v.bool("keep_dates"), cx),
            Kind::SevenZ => sevenz_extract(src, &dest, pw, cx),
            Kind::Rar => Err(anyhow!("{}", tr("files.extract.err.rar"))),
            Kind::Tar => tar_extract(Box::new(src), &dest, v.bool("keep_dates"), cx),
            Kind::Gz | Kind::Bz2 | Kind::Xz | Kind::Zst => {
                let total = src.len;
                let dec = decoder(kind, src)?;
                // A compressed tar, or a single compressed file?
                let mut peek = std::io::BufReader::with_capacity(1 << 16, dec);
                let head = peek.fill_buf_owned(512)?;
                if head.len() >= 262 && &head[257..262] == b"ustar" {
                    tar_extract(Box::new(peek), &dest, v.bool("keep_dates"), cx)
                } else {
                    let out_name = single_name(&input.path);
                    let p = dest.join(castor_core::job::sanitize_file_name(&out_name));
                    let mut w = std::io::BufWriter::new(std::fs::File::create(&p)?);
                    copy_cancellable(&mut peek, &mut w, cx, total)?;
                    w.flush()?;
                    Ok(1)
                }
            }
        };
        let count = match res {
            Ok(n) => n,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dest);
                return Err(e);
            }
        };
        let dest = if v.bool("smart_folder") { flatten_single(&dest, cx)? } else { dest };
        let size = dir_size(&dest);
        Ok(vec![Output::new(&dest).with_note(tr_args("files.extract.note", &[("count", &count.to_string()), ("size", &units::bytes(size))]))])
    }
}

trait FillOwned {
    fn fill_buf_owned(&mut self, n: usize) -> std::io::Result<Vec<u8>>;
}
impl<R: Read> FillOwned for std::io::BufReader<R> {
    fn fill_buf_owned(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        use std::io::BufRead;
        // Fill until we have n bytes or EOF, without consuming.
        loop {
            let b = self.fill_buf()?;
            if b.len() >= n || b.is_empty() {
                return Ok(b[..b.len().min(n)].to_vec());
            }
            // BufReader only refills when empty; a short first read is good enough for sniffing.
            return Ok(b.to_vec());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Zip,
    SevenZ,
    Rar,
    Tar,
    Gz,
    Bz2,
    Xz,
    Zst,
}

fn sniff(src: &mut Source) -> Result<Kind> {
    let mut head = [0u8; 512];
    let n = read_up_to(src, &mut head)?;
    src.seek(SeekFrom::Start(0))?;
    let h = &head[..n];
    Ok(if h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") || h.starts_with(b"PK\x07\x08") {
        Kind::Zip
    } else if h.starts_with(&[0x37, 0x7A, 0xBC, 0xAF, 0x27, 0x1C]) {
        Kind::SevenZ
    } else if h.starts_with(b"Rar!\x1a\x07") {
        Kind::Rar
    } else if h.starts_with(&[0x1F, 0x8B]) {
        Kind::Gz
    } else if h.starts_with(b"BZh") {
        Kind::Bz2
    } else if h.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0]) {
        Kind::Xz
    } else if h.starts_with(&[0x28, 0xB5, 0x2F, 0xFD]) {
        Kind::Zst
    } else if n >= 262 && &h[257..262] == b"ustar" {
        Kind::Tar
    } else {
        bail!("{}", tr("files.extract.err.unknown"))
    })
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        let k = r.read(&mut buf[n..])?;
        if k == 0 {
            break;
        }
        n += k;
    }
    Ok(n)
}

fn decoder(kind: Kind, src: Source) -> Result<Box<dyn Read>> {
    let r = std::io::BufReader::new(src);
    Ok(match kind {
        Kind::Gz => Box::new(flate2::read::MultiGzDecoder::new(r)),
        Kind::Bz2 => Box::new(bzip2::read::MultiBzDecoder::new(r)),
        Kind::Xz => Box::new(lzma_rust2::XzReader::new(r, true)),
        Kind::Zst => Box::new(zstd::stream::read::Decoder::with_buffer(r)?),
        _ => bail!("not a stream format"),
    })
}

/// "backup.tar.gz" → "backup", "photos.zip.001" → "photos".
fn folder_name(p: &Path) -> String {
    let mut name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    loop {
        let lower = name.to_lowercase();
        let Some(i) = name.rfind('.') else { break };
        let ext = &lower[i + 1..];
        let known = ["zip", "7z", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "tzst", "cbz", "jar", "apk", "epub"];
        if known.contains(&ext) || (ext.len() == 3 && ext.chars().all(|c| c.is_ascii_digit())) {
            name.truncate(i);
        } else {
            break;
        }
    }
    if name.is_empty() { "extracted".into() } else { name }
}

/// Name of the single file inside "notes.txt.gz" → "notes.txt".
fn single_name(p: &Path) -> String {
    let n = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    for ext in [".gz", ".bz2", ".xz", ".zst", ".GZ", ".BZ2", ".XZ", ".ZST"] {
        if let Some(s) = n.strip_suffix(ext) {
            return s.to_string();
        }
    }
    format!("{n}.out")
}

fn is_later_volume(p: &Path) -> bool {
    let n = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    n.len() == 3 && n.chars().all(|c| c.is_ascii_digit()) && n != "001"
}

/// A safe destination path for an archive entry name, or None to skip it.
pub fn safe_path(dest: &Path, name: &str) -> Option<PathBuf> {
    let norm = name.replace('\\', "/");
    let mut out = dest.to_path_buf();
    let mut any = false;
    for part in norm.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            p => {
                // Drive letters ("C:") and other reserved characters are neutralised.
                let s = castor_core::job::sanitize_file_name(p);
                if Path::new(&s).components().any(|c| !matches!(c, Component::Normal(_))) {
                    return None;
                }
                out.push(s);
                any = true;
            }
        }
    }
    (any && out.starts_with(dest)).then_some(out)
}

fn copy_cancellable(r: &mut impl Read, w: &mut impl Write, cx: &RunCx, total: u64) -> Result<u64> {
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        cx.check_cancel()?;
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(done);
        }
        w.write_all(&buf[..n])?;
        done += n as u64;
        if total > 0 {
            cx.progress((done as f32 / total as f32).min(0.99));
        }
    }
}

fn zip_extract(src: Source, dest: &Path, pw: &str, keep_dates: bool, cx: &RunCx) -> Result<usize> {
    let mut z = zip::ZipArchive::new(src).map_err(|e| anyhow!("{}: {e}", tr("files.extract.err.corrupt")))?;
    let n = z.len();
    let mut count = 0;
    for i in 0..n {
        cx.check_cancel()?;
        let mut f = if pw.is_empty() {
            match z.by_index(i) {
                Ok(f) => f,
                Err(zip::result::ZipError::UnsupportedArchive(m)) if m.contains("assword") => bail!("{}", tr("files.extract.err.password_needed")),
                Err(e) => return Err(anyhow!("{e}")),
            }
        } else {
            match z.by_index_decrypt(i, pw.as_bytes()) {
                Ok(f) => f,
                Err(zip::result::ZipError::InvalidPassword) => bail!("{}", tr("files.extract.err.password_wrong")),
                Err(e) => return Err(anyhow!("{e}")),
            }
        };
        let name = f.name().to_string();
        let Some(p) = safe_path(dest, &name) else { continue };
        if f.is_dir() {
            std::fs::create_dir_all(&p)?;
            continue;
        }
        if f.is_symlink() {
            continue;
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let p = castor_core::job::unique_path(&p);
        let mut w = std::io::BufWriter::new(std::fs::File::create(&p)?);
        let res = std::io::copy(&mut f, &mut w);
        if let Err(e) = res {
            drop(w);
            let _ = std::fs::remove_file(&p);
            if e.kind() == std::io::ErrorKind::InvalidData && !pw.is_empty() {
                bail!("{}", tr("files.extract.err.password_wrong"));
            }
            return Err(anyhow!("{name}: {e}"));
        }
        w.flush()?;
        drop(w);
        if keep_dates {
            if let Some(t) = f.last_modified().and_then(|d| time::PrimitiveDateTime::try_from(d).ok()) {
                let st: std::time::SystemTime = t.assume_utc().into();
                let _ = common::set_times(&p, None, Some(st));
            }
        }
        count += 1;
        cx.progress((i + 1) as f32 / n as f32);
    }
    Ok(count)
}

fn sevenz_extract(src: Source, dest: &Path, pw: &str, cx: &RunCx) -> Result<usize> {
    let password = sevenz_rust2::Password::new(pw);
    let mut r = match sevenz_rust2::ArchiveReader::new(src, password) {
        Ok(r) => r,
        Err(e) => {
            let msg = e.to_string();
            if msg.to_lowercase().contains("password") {
                bail!("{}", tr(if pw.is_empty() { "files.extract.err.password_needed" } else { "files.extract.err.password_wrong" }));
            }
            bail!("{}: {msg}", tr("files.extract.err.corrupt"));
        }
    };
    let total = r.archive().files.len().max(1);
    let mut count = 0usize;
    let mut idx = 0usize;
    let mut err: Option<anyhow::Error> = None;
    let res = r.for_each_entries(|entry, reader| {
        idx += 1;
        if cx.is_cancelled() {
            return Ok(false);
        }
        let Some(p) = safe_path(dest, entry.name()) else {
            std::io::copy(reader, &mut std::io::sink())?;
            return Ok(true);
        };
        if entry.is_directory() {
            std::fs::create_dir_all(&p)?;
            return Ok(true);
        }
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        let p = castor_core::job::unique_path(&p);
        let mut w = std::io::BufWriter::new(std::fs::File::create(&p)?);
        if let Err(e) = std::io::copy(reader, &mut w) {
            err = Some(anyhow!("{}: {e}", entry.name()));
            return Ok(false);
        }
        w.flush()?;
        drop(w);
        if entry.has_last_modified_date {
            let st: std::time::SystemTime = entry.last_modified_date().into();
            let _ = common::set_times(&p, None, Some(st));
        }
        count += 1;
        cx.progress(idx as f32 / total as f32);
        Ok(true)
    });
    cx.check_cancel()?;
    if let Some(e) = err {
        return Err(e);
    }
    res.map_err(|e| {
        let msg = e.to_string();
        if msg.to_lowercase().contains("password") || msg.to_lowercase().contains("checksum") && !pw.is_empty() {
            anyhow!("{}", tr("files.extract.err.password_wrong"))
        } else {
            anyhow!("{}: {msg}", tr("files.extract.err.corrupt"))
        }
    })?;
    Ok(count)
}

fn tar_extract(r: Box<dyn Read + '_>, dest: &Path, keep_dates: bool, cx: &RunCx) -> Result<usize> {
    let mut a = tar::Archive::new(r);
    let mut count = 0;
    for e in a.entries().map_err(|e| anyhow!("{}: {e}", tr("files.extract.err.corrupt")))? {
        cx.check_cancel()?;
        let mut e = e.map_err(|e| anyhow!("{}: {e}", tr("files.extract.err.corrupt")))?;
        let name = String::from_utf8_lossy(&e.path_bytes()).into_owned();
        let Some(p) = safe_path(dest, &name) else { continue };
        match e.header().entry_type() {
            tar::EntryType::Directory => {
                std::fs::create_dir_all(&p)?;
            }
            tar::EntryType::Regular | tar::EntryType::Continuous | tar::EntryType::GNUSparse => {
                if let Some(d) = p.parent() {
                    std::fs::create_dir_all(d)?;
                }
                let p = castor_core::job::unique_path(&p);
                let mut w = std::io::BufWriter::new(std::fs::File::create(&p)?);
                std::io::copy(&mut e, &mut w)?;
                w.flush()?;
                drop(w);
                if keep_dates {
                    if let Ok(m) = e.header().mtime() {
                        let _ = common::set_times(&p, None, Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(m)));
                    }
                }
                count += 1;
                if count % 16 == 0 {
                    cx.message(name.clone());
                }
            }
            // Links, devices, FIFOs: never created.
            _ => {}
        }
    }
    Ok(count)
}

/// If the folder holds exactly one folder, use that one instead (no "x/x/…").
fn flatten_single(dest: &Path, _cx: &RunCx) -> Result<PathBuf> {
    let items: Vec<_> = std::fs::read_dir(dest)?.flatten().collect();
    if items.len() == 1 && items[0].path().is_dir() {
        let inner = items[0].path();
        let target = castor_core::job::unique_path(&dest.with_file_name(items[0].file_name()));
        let tmp = dest.with_file_name(format!(".castor-x-{}", std::process::id()));
        std::fs::rename(&inner, &tmp)?;
        std::fs::remove_dir(dest)?;
        let target = if target == dest { castor_core::job::unique_path(dest) } else { target };
        std::fs::rename(&tmp, &target)?;
        return Ok(target);
    }
    Ok(dest.to_path_buf())
}

fn dir_size(p: &Path) -> u64 {
    castor_core::files::expand(&[p.to_path_buf()], true).iter().map(|f| f.size).sum()
}

/// The archive bytes, joining split volumes (`x.001`, `x.002`…) transparently.
pub struct Source {
    parts: Vec<(std::fs::File, u64)>,
    len: u64,
    pos: u64,
}

impl Source {
    pub fn open(p: &Path) -> Result<Source> {
        let mut files = vec![p.to_path_buf()];
        let ext = p.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        if ext == "001" {
            let base = p.with_extension("");
            for i in 2..10_000 {
                let next = PathBuf::from(format!("{}.{i:03}", base.display()));
                if !next.is_file() {
                    break;
                }
                files.push(next);
            }
        }
        let mut parts = Vec::new();
        let mut len = 0;
        for f in files {
            let h = std::fs::File::open(&f).map_err(|e| anyhow!("{}: {e}", f.display()))?;
            let l = h.metadata()?.len();
            len += l;
            parts.push((h, l));
        }
        anyhow::ensure!(len > 0, "{}", tr("files.extract.err.empty"));
        Ok(Source { parts, len, pos: 0 })
    }
}

impl Read for Source {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut off = self.pos;
        for (f, l) in &mut self.parts {
            if off < *l {
                f.seek(SeekFrom::Start(off))?;
                let k = buf.len().min((*l - off) as usize);
                let n = f.read(&mut buf[..k])?;
                self.pos += n as u64;
                return Ok(n);
            }
            off -= *l;
        }
        Ok(0)
    }
}

impl Seek for Source {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if p < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn zip_slip() {
        let d = Path::new("/out");
        assert!(safe_path(d, "../evil").is_none());
        assert!(safe_path(d, "a/../../evil").is_none());
        assert_eq!(safe_path(d, "/etc/passwd").unwrap(), Path::new("/out/etc/passwd"));
        assert_eq!(safe_path(d, "C:\\x\\y.txt").unwrap(), Path::new("/out/C_/x/y.txt"));
        assert!(safe_path(d, "./").is_none());
    }
    #[test]
    fn names() {
        assert_eq!(folder_name(Path::new("backup.tar.gz")), "backup");
        assert_eq!(folder_name(Path::new("photos.zip.001")), "photos");
        assert_eq!(folder_name(Path::new("v1.2.zip")), "v1.2");
    }
}
