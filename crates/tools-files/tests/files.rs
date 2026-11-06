//! Integration tests: every files tool through the real runner, on files made here.

use std::path::{Path, PathBuf};

use castor_core::options::{Value, Values};
use castor_core::runner::{self, OutputPolicy, RunRequest, Step};
use castor_core::{CancelToken, FileInfo, Tool};

fn tool(id: &str) -> std::sync::Arc<dyn Tool> {
    tools_files::tools().into_iter().find(|t| t.meta().id == id).unwrap()
}

fn run(id: &str, inputs: &[PathBuf], set: &[(&str, Value)], out: Option<&Path>) -> castor_core::history::RunRecord {
    let t = tool(id);
    let mut v = Values::defaults(&t.options());
    for (k, val) in set {
        v.set(k, val.clone());
    }
    let req = RunRequest {
        steps: vec![Step { tool: t, values: v }],
        inputs: inputs.iter().map(FileInfo::from_path).collect(),
        output: match out {
            Some(o) => OutputPolicy::Folder(o.to_path_buf()),
            None => OutputPolicy::NextToInput { fallback: std::env::temp_dir() },
        },
        chain_name: None,
        max_parallel: 0,
        record: false,
    };
    runner::run_blocking(req, CancelToken::new(), |_| {})
}

fn t(s: &str) -> Value {
    Value::Text(s.into())
}

fn tree(root: &Path) {
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("a.txt"), "alpha").unwrap();
    std::fs::write(root.join("b été.txt"), "beta ünïcode").unwrap();
    std::fs::write(root.join("sub/c.bin"), vec![7u8; 300_000]).unwrap();
    std::fs::write(root.join("junk.tmp"), "x").unwrap();
}

#[test]
fn archive_then_extract_every_format() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("docs");
    tree(&src);
    for (fmt, pw) in [("zip", ""), ("zip", "s3cret"), ("7z", ""), ("7z", "s3cret"), ("tar.gz", ""), ("tar.zst", ""), ("tar.xz", ""), ("tar", "")] {
        let out = d.path().join(format!("out-{}-{}", fmt.replace('.', "_"), !pw.is_empty()));
        std::fs::create_dir_all(&out).unwrap();
        let r = run("files.archive", &[src.clone()], &[("format", t(fmt)), ("password", t(pw))], Some(&out));
        assert!(r.errors.is_empty(), "{fmt}: {:?}", r.errors);
        let archive = r.outputs[0].clone();
        assert!(archive.file_name().unwrap().to_string_lossy().starts_with("docs."));
        let x = out.join("x");
        std::fs::create_dir_all(&x).unwrap();
        // Wrong password fails clearly.
        if !pw.is_empty() {
            let bad = run("files.extract", &[archive.clone()], &[("password", t("nope"))], Some(&x));
            assert!(!bad.errors.is_empty(), "{fmt}: wrong password accepted");
        }
        let r = run("files.extract", &[archive.clone()], &[("password", t(pw))], Some(&x));
        assert!(r.errors.is_empty(), "{fmt}: {:?}", r.errors);
        let folder = &r.outputs[0];
        assert_eq!(std::fs::read_to_string(folder.join("b été.txt")).unwrap(), "beta ünïcode", "{fmt}");
        assert_eq!(std::fs::read(folder.join("sub/c.bin")).unwrap().len(), 300_000, "{fmt}");
        assert!(!folder.join("junk.tmp").exists(), "{fmt}: exclusion ignored");
    }
}

#[test]
fn split_volumes_round_trip() {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("big");
    std::fs::create_dir_all(&src).unwrap();
    // Incompressible data so the archive really exceeds 1 MB.
    let mut x = 12345u64;
    let data: Vec<u8> = (0..2_500_000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    std::fs::write(src.join("noise.bin"), &data).unwrap();
    let out = d.path().join("o");
    std::fs::create_dir_all(&out).unwrap();
    let r = run("files.archive", &[src.clone()], &[("format", t("zip")), ("split_mb", Value::Int(1))], Some(&out));
    assert!(r.outputs.len() >= 3, "{:?}", r.outputs);
    assert!(r.outputs[0].to_string_lossy().ends_with(".zip.001"));
    let r = run("files.extract", &r.outputs, &[], Some(&out));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(std::fs::read(r.outputs[0].join("noise.bin")).unwrap(), data);
}

#[test]
fn zip_slip_is_blocked() {
    let d = tempfile::tempdir().unwrap();
    let evil = d.path().join("evil.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&evil).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("../../escaped.txt", o).unwrap();
        std::io::Write::write_all(&mut z, b"nope").unwrap();
        z.start_file("ok/inside.txt", o).unwrap();
        std::io::Write::write_all(&mut z, b"fine").unwrap();
        z.finish().unwrap();
    }
    let out = d.path().join("deep/out");
    std::fs::create_dir_all(&out).unwrap();
    let r = run("files.extract", &[evil], &[("smart_folder", Value::Bool(false))], Some(&out));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(!d.path().join("escaped.txt").exists() && !d.path().join("deep/escaped.txt").exists());
    assert_eq!(std::fs::read_to_string(r.outputs[0].join("ok/inside.txt")).unwrap(), "fine");
}

#[test]
fn rename_with_tokens_and_undo() {
    let d = tempfile::tempdir().unwrap();
    for n in ["IMG 10.JPG", "IMG 2.JPG", "IMG 1.JPG"] {
        std::fs::write(d.path().join(n), n).unwrap();
    }
    let files: Vec<PathBuf> = ["IMG 10.JPG", "IMG 2.JPG", "IMG 1.JPG"].iter().map(|n| d.path().join(n)).collect();
    let r = run(
        "files.rename",
        &files,
        &[("pattern", t("trip-{n:03}")), ("extension", t("lower")), ("case", t("keep"))],
        None,
    );
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    // Natural order: 1, 2, 10.
    assert_eq!(std::fs::read_to_string(d.path().join("trip-001.jpg")).unwrap(), "IMG 1.JPG");
    assert_eq!(std::fs::read_to_string(d.path().join("trip-003.jpg")).unwrap(), "IMG 10.JPG");
    assert_eq!(r.undo.len(), 1);
    tool("files.rename").undo(&r.undo[0].1).unwrap();
    assert!(d.path().join("IMG 10.JPG").exists() && !d.path().join("trip-001.jpg").exists());
    // A swap (a ↔ b) works thanks to the two-phase moves.
    std::fs::write(d.path().join("x.txt"), "x").unwrap();
    std::fs::write(d.path().join("y.txt"), "y").unwrap();
    let r = run("files.rename", &[d.path().join("x.txt"), d.path().join("y.txt")], &[("find", t("^x$|^y$")), ("regex", Value::Bool(true)), ("replace", t("$0$0"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert_eq!(std::fs::read_to_string(d.path().join("xx.txt")).unwrap(), "x");
}

#[test]
fn dates_shift_and_undo() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("photo.jpg");
    std::fs::write(&f, "not really a photo").unwrap();
    let before = std::fs::metadata(&f).unwrap().modified().unwrap();
    let r = run("files.dates", &[f.clone()], &[("mode", t("shift")), ("shift_hours", Value::Int(-2)), ("which", t("modified"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let after = std::fs::metadata(&f).unwrap().modified().unwrap();
    assert_eq!(before.duration_since(after).unwrap().as_secs(), 7200);
    tool("files.dates").undo(&r.undo[0].1).unwrap();
    assert_eq!(std::fs::metadata(&f).unwrap().modified().unwrap(), before);
    let r = run("files.dates", &[f.clone()], &[("mode", t("set")), ("date", t("2001-02-03 04:05"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let m = std::fs::metadata(&f).unwrap();
    assert_eq!(tools_files::common::fmt_time(m.modified().unwrap(), "%Y-%m-%d %H:%M"), "2001-02-03 04:05");
    #[cfg(windows)]
    assert_eq!(tools_files::common::fmt_time(m.created().unwrap(), "%Y-%m-%d %H:%M"), "2001-02-03 04:05");
}

#[test]
fn sort_by_type_and_rules_then_undo() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("Downloads");
    std::fs::create_dir_all(&root).unwrap();
    for n in ["a.pdf", "b.jpg", "c.mp3", "d.zip", "e.xyz"] {
        std::fs::write(root.join(n), n).unwrap();
    }
    let files: Vec<PathBuf> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).collect();
    let r = run("files.sort", &files, &[("by", t("type"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(root.join("Documents/a.pdf").exists());
    assert!(root.join("Images/b.jpg").exists());
    assert!(root.join("Other/e.xyz").exists());
    tool("files.sort").undo(&r.undo[0].1).unwrap();
    assert!(root.join("a.pdf").exists() && !root.join("Documents").exists());
    let r = run("files.sort", &files, &[("by", t("rules")), ("rules", t("*.pdf, *.zip => Keep/{ext}\n* => Rest"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(root.join("Keep/pdf/a.pdf").exists() && root.join("Keep/zip/d.zip").exists() && root.join("Rest/c.mp3").exists());
}

#[test]
fn duplicates_report_and_move() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path().join("pics");
    std::fs::create_dir_all(root.join("copy")).unwrap();
    let big: Vec<u8> = (0..400_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(root.join("one.bin"), &big).unwrap();
    std::fs::write(root.join("copy/one (1).bin"), &big).unwrap();
    let mut other = big.clone();
    other[200_000] ^= 1; // same size, same head and tail: only the full hash differs
    std::fs::write(root.join("almost.bin"), &other).unwrap();
    std::fs::write(root.join("small.txt"), "hello").unwrap();
    std::fs::write(root.join("small2.txt"), "hello").unwrap();
    let files = castor_core::files::expand(&[root.clone()], true).into_iter().map(|f| f.path).collect::<Vec<_>>();
    let out = d.path().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let r = run("files.duplicates", &files, &[("action", t("move")), ("min_size", Value::Int(0))], Some(&out));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let report = std::fs::read_to_string(&r.outputs[0]).unwrap();
    assert_eq!(report.lines().count(), 1 + 4, "{report}");
    assert!(!report.contains("almost.bin"));
    // One copy of each pair moved aside, nothing deleted.
    assert_eq!(std::fs::read_dir(out.join("Duplicates")).unwrap().count(), 2);
    tool("files.duplicates").undo(&r.undo[0].1).unwrap();
    assert_eq!(castor_core::files::expand(&[root.clone()], true).len(), 5);
}

#[test]
fn identify_finds_real_types() {
    let d = tempfile::tempdir().unwrap();
    let mut png = Vec::new();
    image::RgbImage::from_pixel(32, 32, image::Rgb([200, 30, 30])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    std::fs::write(d.path().join("picture"), &png).unwrap();
    std::fs::write(d.path().join("photo.pdf"), &png).unwrap();
    std::fs::write(d.path().join("script.txt"), "import os\n\ndef main():\n    for x in range(10):\n        print(os.getcwd(), x)\n\nif __name__ == '__main__':\n    main()\n").unwrap();
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(d.path().join("archive.dat")).unwrap());
        z.start_file("readme.txt", zip::write::SimpleFileOptions::default()).unwrap();
        std::io::Write::write_all(&mut z, &vec![b'a'; 5000]).unwrap();
        z.finish().unwrap();
    }
    let id = tools_files::identify(&d.path().join("picture")).unwrap();
    assert_eq!(id.label, "png", "{id:?}");
    let id = tools_files::identify(&d.path().join("archive.dat")).unwrap();
    assert_eq!(id.label, "zip", "{id:?}");
    let id = tools_files::identify(&d.path().join("script.txt")).unwrap();
    assert_eq!(id.label, "python", "{id:?}");
    let files: Vec<PathBuf> = ["picture", "photo.pdf", "script.txt", "archive.dat"].iter().map(|n| d.path().join(n)).collect();
    let r = run("files.identify", &files, &[("fix_extensions", Value::Bool(true)), ("report", t("csv"))], None);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(d.path().join("picture.png").exists());
    assert!(d.path().join("photo.png").exists());
    assert!(d.path().join("script.txt").exists(), "text files keep their extension");
    assert!(d.path().join("archive.zip").exists());
    let undo = r.undo.iter().find(|(id, _)| id == "files.identify").unwrap();
    tool("files.identify").undo(&undo.1).unwrap();
    assert!(d.path().join("picture").exists() && d.path().join("photo.pdf").exists());
}

#[test]
fn metadata_audio_tags_round_trip() {
    if engine_media::ffmpeg().is_err() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let mp3 = d.path().join("tone.mp3");
    let st = std::process::Command::new(engine_media::ffmpeg().unwrap())
        .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "sine=f=440:d=1", "-metadata", "title=Old", "-metadata", "artist=Someone"])
        .arg(&mp3)
        .status()
        .unwrap();
    assert!(st.success());
    let out = d.path().join("o");
    std::fs::create_dir_all(&out).unwrap();
    let r = run("files.metadata", &[mp3.clone()], &[("action", t("edit")), ("title", t("Nouveau titre")), ("year", t("2026"))], Some(&out));
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let r2 = run("files.metadata", &[r.outputs[0].clone()], &[("action", t("view")), ("report", t("json"))], Some(&out));
    let j = std::fs::read_to_string(&r2.outputs[0]).unwrap();
    assert!(j.contains("Nouveau titre") && j.contains("Someone"), "{j}");
    let r3 = run("files.metadata", &[mp3], &[("action", t("strip"))], Some(&out));
    assert!(r3.errors.is_empty(), "{:?}", r3.errors);
    let r4 = run("files.metadata", &[r3.outputs[0].clone()], &[("action", t("view")), ("report", t("json"))], Some(&out));
    let j = std::fs::read_to_string(&r4.outputs[0]).unwrap();
    assert!(!j.contains("Someone"), "{j}");
}

#[test]
fn checksum_still_works() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("x.txt"), "abc").unwrap();
    let r = run("files.checksum", &[d.path().join("x.txt")], &[], None);
    let s = std::fs::read_to_string(&r.outputs[0]).unwrap();
    assert!(s.starts_with("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
}
