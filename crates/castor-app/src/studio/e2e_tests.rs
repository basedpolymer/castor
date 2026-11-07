//! End-to-end tests of the studios' saves and exports through the runner, on the generated
//! fixtures (`tests/fixtures/generated/`, made by the PDF and media test suites). Skipped with a
//! message when a fixture or ffmpeg is missing.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use castor_core::history::RunStatus;
use castor_core::runner::{OutputPolicy, RunRequest, Step, run_blocking};
use castor_core::{CancelToken, FileInfo, Tool};

use super::media_engine::{Cue, Export, MediaEdit};
use super::pdf_engine::{self as eng, Mark, PageRef, PdfEdit, PlacedMark, Signature};
use super::tools::{MediaStudioTool, PdfStudioTool, values};

fn fixture(rel: &str) -> Option<PathBuf> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/generated").join(rel);
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("skipped: fixture {} missing", p.display());
        None
    }
}

fn run(tool: Arc<dyn Tool>, script: String, overwrite: bool, input: &Path, out: &Path) -> castor_core::history::RunRecord {
    let req = RunRequest {
        steps: vec![Step { tool, values: values(script, overwrite) }],
        inputs: vec![FileInfo::from_path(input)],
        output: OutputPolicy::Folder(out.to_path_buf()),
        chain_name: None,
        max_parallel: 1,
        record: false,
    };
    run_blocking(req, CancelToken::new(), |_| {})
}

fn text_of(path: &Path, page: usize) -> String {
    let doc = engine_pdf::open_document(path, "").unwrap();
    engine_pdf::text::page_text(&doc.load_page(page as i32).unwrap()).unwrap()
}

#[test]
fn pdf_studio_saves_everything() {
    let Some(src) = fixture("pdf/arxiv-attention.pdf") else { return };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("paper.pdf");
    std::fs::copy(&src, &input).unwrap();
    let n = engine_pdf::page_count(&input).unwrap();
    let before = text_of(&input, 0);
    assert!(before.contains("Attention Is All You Need"));

    // Reorder (page 2 first), rotate one page, delete the last, redact the title, annotate, sign.
    let mut e = PdfEdit::new(input.clone(), n);
    e.pages.swap(0, 1);
    e.pages[1].rotate = 90;
    e.pages.pop();
    e.reviewer = "basedpolymer".into();
    let title = {
        // Find the title's box with MuPDF's own search, as the studio does.
        let doc = engine_pdf::open_document(&input, "").unwrap();
        let q = doc.load_page(0).unwrap().search("Attention Is All You Need", 4).unwrap();
        let q = q.iter().next().unwrap().clone();
        [q.ul.x - 1.0, q.ul.y - 1.0, q.lr.x + 1.0, q.lr.y + 1.0]
    };
    e.marks.push(PlacedMark { src: 0, page: 0, mark: Mark::Redact { rect: title } });
    e.marks.push(PlacedMark { src: 0, page: 0, mark: Mark::Highlight { rects: vec![[100.0, 400.0, 400.0, 412.0]], color: [1.0, 0.9, 0.6] } });
    e.marks.push(PlacedMark { src: 0, page: 0, mark: Mark::Note { at: [500.0, 300.0], text: "A comment".into(), color: [1.0, 0.9, 0.6] } });
    e.marks.push(PlacedMark { src: 0, page: 0, mark: Mark::Pen { strokes: vec![vec![[100.0, 600.0], [200.0, 610.0], [300.0, 600.0]]], color: [0.2, 0.4, 0.7], width: 1.5 } });
    e.marks.push(PlacedMark { src: 0, page: 0, mark: Mark::Arrow { from: [500.0, 500.0], to: [400.0, 450.0], color: [0.7, 0.2, 0.2], width: 1.5 } });
    e.marks.push(PlacedMark {
        src: 0,
        page: 0,
        mark: Mark::Signature { rect: [350.0, 700.0, 520.0, 750.0], sig: Signature::Typed { name: "basedpolymer".into(), color: [0.1, 0.16, 0.42] }, date: Some("2026-11-06".into()) },
    });
    let out_dir = dir.path().join("out");
    let r = run(Arc::new(PdfStudioTool::new()), serde_json::to_string(&e).unwrap(), false, &input, &out_dir);
    assert_eq!(r.status, RunStatus::Complete, "{:?}", r.errors);
    let out = r.outputs.first().unwrap().clone();
    assert!(out.file_name().unwrap().to_string_lossy().contains("-edited"), "{}", out.display());
    assert_eq!(engine_pdf::page_count(&out).unwrap(), n - 1);
    // Old page 1 (with the marks) is now page 2; its title is gone, the rest of the text is not.
    let p2 = text_of(&out, 1);
    assert!(!p2.contains("Attention Is All You Need"), "the redacted title must be gone");
    assert!(p2.contains("Abstract"));
    let doc = engine_pdf::open_pdf(&out, "").unwrap();
    let page = doc.load_pdf_page(1).unwrap();
    assert_eq!(page.rotation().unwrap() % 360, 90);
    let annots: Vec<_> = page.annotations().collect();
    assert!(annots.len() >= 4, "highlight, note, pen and arrow annotations: {}", annots.len());
    assert!(annots.iter().any(|a| a.author().ok().flatten() == Some("basedpolymer")));
    // The original is untouched.
    assert_eq!(text_of(&input, 0), before);

    // Overwrite with undo.
    let mut e2 = PdfEdit::new(input.clone(), n);
    e2.pages.truncate(2);
    let r2 = run(Arc::new(PdfStudioTool::new()), serde_json::to_string(&e2).unwrap(), true, &input, &out_dir);
    assert_eq!(r2.status, RunStatus::Complete, "{:?}", r2.errors);
    assert_eq!(engine_pdf::page_count(&input).unwrap(), 2);
    let (_, journal) = r2.undo.first().expect("undo journal").clone();
    PdfStudioTool::new().undo(&journal).unwrap();
    assert_eq!(engine_pdf::page_count(&input).unwrap(), n);
}

#[test]
fn pdf_studio_fills_forms_and_extracts() {
    let Some(src) = fixture("pdf/irs-fw9.pdf") else { return };
    let dir = tempfile::tempdir().unwrap();
    let fields = eng::read_fields(&src).unwrap();
    let text = fields.iter().find(|f| f.kind == eng::FieldKind::Text).expect("a text field").clone();
    let check = fields.iter().find(|f| f.kind == eng::FieldKind::Check).expect("a checkbox").clone();
    assert!(!text.label.is_empty());
    // No tooltips in this form: the labels come from the printed text.
    assert!(fields.iter().any(|f| f.label.contains("Individual/sole proprietor")), "{:?}", fields.iter().map(|f| &f.label).collect::<Vec<_>>());
    assert!(fields.iter().any(|f| f.label.contains("Name of entity")), "{:?}", fields.iter().map(|f| &f.label).collect::<Vec<_>>());
    let mut e = PdfEdit::new(src.clone(), engine_pdf::page_count(&src).unwrap());
    e.fields.insert(text.name.clone(), "basedpolymer".into());
    e.fields.insert(check.name.clone(), check.on.clone());
    let r = run(Arc::new(PdfStudioTool::new()), serde_json::to_string(&e).unwrap(), false, &src, dir.path());
    assert_eq!(r.status, RunStatus::Complete, "{:?}", r.errors);
    let out = &r.outputs[0];
    let after = eng::read_fields(out).unwrap();
    assert_eq!(after.iter().find(|f| f.name == text.name).unwrap().value, "basedpolymer");
    assert_eq!(after.iter().find(|f| f.name == check.name).unwrap().value, check.on);

    // Extract pages 2 and 1 (in that order) into a new file.
    let mut x = PdfEdit::new(src.clone(), 6);
    x.pages = vec![PageRef { src: 0, page: 1, rotate: 0 }, PageRef { src: 0, page: 0, rotate: 180 }];
    let r = run(Arc::new(PdfStudioTool::with_suffix("pages")), serde_json::to_string(&x).unwrap(), false, &src, dir.path());
    assert_eq!(r.status, RunStatus::Complete, "{:?}", r.errors);
    assert!(r.outputs[0].to_string_lossy().contains("-pages"));
    assert_eq!(engine_pdf::page_count(&r.outputs[0]).unwrap(), 2);
}

#[test]
fn pdf_studio_inserts_pages_from_another_pdf() {
    let (Some(a), Some(b)) = (fixture("pdf/irs-fw9.pdf"), fixture("pdf/arxiv-attention.pdf")) else { return };
    let dir = tempfile::tempdir().unwrap();
    let mut e = PdfEdit::new(a.clone(), engine_pdf::page_count(&a).unwrap());
    e.sources.push(b.clone());
    e.pages.insert(1, PageRef { src: 1, page: 0, rotate: 0 });
    e.marks.push(PlacedMark { src: 1, page: 0, mark: Mark::Rect { rect: [100.0, 100.0, 200.0, 200.0], color: [0.0; 3], width: 1.0 } });
    let r = run(Arc::new(PdfStudioTool::new()), serde_json::to_string(&e).unwrap(), false, &a, dir.path());
    assert_eq!(r.status, RunStatus::Complete, "{:?}", r.errors);
    let out = &r.outputs[0];
    assert_eq!(engine_pdf::page_count(out).unwrap(), 7);
    assert!(text_of(out, 1).contains("Attention"));
    let doc = engine_pdf::open_pdf(out, "").unwrap();
    assert_eq!(doc.load_pdf_page(1).unwrap().annotations().count(), 1, "the mark follows its page");
}

fn ffmpeg_ok() -> bool {
    let ok = engine_media::ffmpeg().is_ok();
    if !ok {
        eprintln!("skipped: ffmpeg missing");
    }
    ok
}

#[test]
fn media_studio_exports_cuts_and_captions() {
    let Some(src) = fixture("media/gaps.mp4") else { return };
    if !ffmpeg_ok() {
        return;
    }
    let reg = Arc::new(castor_tools::registry());
    let dir = tempfile::tempdir().unwrap();
    let p = engine_media::probe(&src).unwrap();
    let mut e = MediaEdit::new(src.clone(), p.duration, p.has_video);
    e.cut_range(2.0, 4.0);
    e.cut_range(6.0, 7.0);
    e.cues = vec![
        Cue { start: 0.5, end: 1.5, text: "One".into() },
        Cue { start: 2.5, end: 3.5, text: "Gone".into() },
        Cue { start: 5.0, end: 5.8, text: "Two".into() },
    ];
    let tool: Arc<dyn Tool> = Arc::new(MediaStudioTool::new(reg));
    for (export, cut) in [(Export::Media, "fast"), (Export::Media, "exact"), (Export::Srt, "exact"), (Export::Burn, "exact")] {
        e.export = export;
        e.cut = cut.into();
        let r = run(tool.clone(), serde_json::to_string(&e).unwrap(), false, &src, dir.path());
        assert_eq!(r.status, RunStatus::Complete, "{export:?} {cut}: {:?}", r.errors);
        let out = &r.outputs[0];
        if export == Export::Srt {
            let srt = std::fs::read_to_string(out).unwrap();
            assert!(srt.contains("One") && srt.contains("Two") && !srt.contains("Gone"), "{srt}");
            // "Two" moved back by the 2 s cut before it.
            assert!(srt.contains("00:00:03,000"), "{srt}");
        } else {
            let d = engine_media::probe(out).unwrap().duration;
            let want: f64 = if cut == "fast" {
                super::media_engine::snap_fast(&e.keep, &super::media_engine::keyframes(&src)).iter().map(|(a, b)| b - a).sum()
            } else {
                e.kept()
            };
            assert!(d <= p.duration + 0.1, "never longer than the source: {d}");
            assert!((d - want).abs() < 0.35, "{export:?} {cut}: {d} vs {want}");
        }
    }
}

/// A crash seen in 0.8: a PDF thumbnail rendered while `pdf.compress` rewrites the images of a
/// JBIG2 scan in another thread (access violation). engine-pdf now shares mupdf-rs's locks, so
/// the app no longer keeps the two apart (the old `pdf_gate`). Slow (≈ 1 min).
#[test]
#[ignore]
fn thumbnail_during_compress() {
    let Some(src) = fixture("pdf/yearbook-1890-scan.pdf") else { return };
    let dir = tempfile::tempdir().unwrap();
    let reg = castor_tools::registry();
    let tool = reg.get("pdf.compress").unwrap();
    let values = castor_core::Values::defaults(&tool.options());
    let s2 = src.clone();
    // As the app does it: thumbnails and tool jobs at the same time, nothing in between.
    let t = std::thread::spawn(move || {
        for _ in 0..20 {
            let _ = castor_tools::thumbnail(&FileInfo::from_path(&s2), 72);
        }
    });
    let req = RunRequest {
        steps: vec![Step { tool, values }],
        inputs: vec![FileInfo::from_path(&src)],
        output: OutputPolicy::Folder(dir.path().to_path_buf()),
        chain_name: None,
        max_parallel: 1,
        record: false,
    };
    let h = castor_core::runner::start(req);
    let mut status = None;
    for ev in h.events.iter() {
        if let castor_core::runner::RunEvent::Finished(r) = ev {
            status = Some(r.status);
        }
    }
    t.join().unwrap();
    assert_eq!(status, Some(RunStatus::Complete));
}
