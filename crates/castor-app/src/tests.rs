//! Tests of the app layer: pure logic (search, suggestions, run events) and GPUI integration
//! tests of the generated form and the palette with test tools.

use std::sync::Arc;

use castor_core::history::{RunRecord, RunStatus};
use castor_core::job::Output;
use castor_core::options::{Choice, Cond, OptionSpec, Value, Values};
use castor_core::runner::RunEvent;
use castor_core::{Family, FileInfo, FileKind, InputSpec, Job, Registry, RunCx, Tool, ToolMeta, ToolStatus};

use crate::tool_screen::{JobStatus, RunView};

struct T(ToolMeta, Vec<OptionSpec>);
impl Tool for T {
    fn meta(&self) -> &ToolMeta {
        &self.0
    }
    fn options(&self) -> Vec<OptionSpec> {
        self.1.clone()
    }
    fn run(&self, _job: &Job, _v: &Values, _cx: &RunCx) -> castor_core::Result<Vec<Output>> {
        Ok(vec![])
    }
}

fn tool(id: &'static str, family: Family, kinds: &'static [FileKind], keywords: &'static [&'static str], options: Vec<OptionSpec>) -> Arc<dyn Tool> {
    Arc::new(T(
        ToolMeta {
            id,
            family,
            inputs: InputSpec::kinds(kinds),
            output: "X",
            keywords,
            status: ToolStatus::Ready,
            requires: &[],
            suffix: "x",
            in_place: false,
            serial: false,
        },
        options,
    ))
}

fn registry() -> Registry {
    Registry::new(vec![
        tool("pdf.compress", Family::Pdf, &[FileKind::Pdf], &["smaller", "shrink", "reduce", "size"], vec![]),
        tool("pdf.merge", Family::Pdf, &[FileKind::Pdf], &["combine", "join"], vec![]),
        tool("image.compress", Family::Image, &[FileKind::Image], &["smaller", "shrink"], vec![]),
        tool("image.convert", Family::Image, &[FileKind::Image], &["format", "jpg", "png"], vec![]),
        tool("files.checksum", Family::Files, &[FileKind::Any], &["hash", "sha"], vec![]),
    ])
}

#[test]
fn sentence_finds_pdf_compress() {
    let reg = registry();
    let res = crate::search::tools(&reg, "make this pdf smaller", &[], &[]);
    assert_eq!(res.first().map(|(t, _)| t.meta().id), Some("pdf.compress"));
    let res = crate::search::tools(&reg, "rendre ce pdf plus léger", &[], &[]);
    assert_eq!(res.first().map(|(t, _)| t.meta().id), Some("pdf.compress"));
}

#[test]
fn favorites_and_recency_boost() {
    let reg = registry();
    let plain = crate::search::tools(&reg, "compress", &[], &[]);
    assert_eq!(plain.len(), 2);
    let boosted = crate::search::tools(&reg, "compress", &[], &["image.compress".to_string()]);
    assert_eq!(boosted[0].0.meta().id, "image.compress");
}

#[test]
fn suggestions_prefer_specific_tools() {
    let reg = registry();
    let files = vec![FileInfo { path: "a.pdf".into(), kind: FileKind::Pdf, size: 10, is_dir: false }];
    let s = crate::home::Home::suggested(&files, reg.all(), &[], &[], 4);
    assert_eq!(s.len(), 3, "two PDF tools + the any-file tool: {s:?}");
    assert!(s[0].starts_with("pdf."));
    assert_eq!(s.last().map(String::as_str), Some("files.checksum"));
}

fn record(status: RunStatus) -> RunRecord {
    serde_json::from_value(serde_json::json!({
        "id": "r1", "tool_id": "pdf.compress", "steps": ["pdf.compress"], "started": 0, "duration_secs": 1.5,
        "status": status, "files_in": 2, "files_out": 1, "bytes_in": 2000, "bytes_out": 500,
        "inputs": [], "outputs": ["a-x.pdf"], "values": {}, "errors": ["b.pdf: broken"], "cancelled": 0
    }))
    .unwrap()
}

#[test]
fn run_events_drive_rows() {
    let files = vec![
        FileInfo { path: "a.pdf".into(), kind: FileKind::Pdf, size: 1000, is_dir: false },
        FileInfo { path: "b.pdf".into(), kind: FileKind::Pdf, size: 1000, is_dir: false },
    ];
    let handle = castor_core::runner::start(castor_core::runner::RunRequest {
        steps: vec![],
        inputs: vec![],
        output: castor_core::runner::OutputPolicy::Folder(std::env::temp_dir()),
        chain_name: None,
        max_parallel: 1,
        record: false,
    });
    let mut v = RunView::new(handle, files);
    v.apply(RunEvent::StepStarted { step: 0, steps: 1, tool_id: "pdf.compress".into(), jobs: vec!["a.pdf".into(), "b.pdf".into()] });
    assert_eq!(v.jobs.len(), 2);
    assert!(v.jobs[0].file.is_some(), "1 → 1 jobs map to the queued files");
    v.apply(RunEvent::JobStarted { step: 0, job: 0 });
    v.apply(RunEvent::JobProgress { step: 0, job: 0, frac: 0.5 });
    assert_eq!(v.jobs[0].status, JobStatus::Running);
    assert!((v.overall() - 0.25).abs() < 1e-6);
    v.apply(RunEvent::JobDone { step: 0, job: 0, outputs: vec![Output::new("a-x.pdf")], bytes_in: 1000, bytes_out: 500, secs: 1.0 });
    v.apply(RunEvent::JobFailed { step: 0, job: 1, error: "broken".into() });
    assert_eq!(v.jobs[1].status, JobStatus::Failed);
    assert!(v.running());
    let done = v.apply(RunEvent::Finished(Box::new(record(RunStatus::Partial))));
    assert!(done && !v.running());
    assert_eq!(crate::tool_screen::summary_line(v.record.as_ref().unwrap()), "2 files · 2.0 KB → 500 B (−75 %) · 1.5 s");
}

#[test]
fn runs_filters() {
    use crate::runs::Filter;
    assert!(Filter::Failed.accepts(RunStatus::Partial));
    assert!(!Filter::Complete.accepts(RunStatus::Failed));
    assert!(Filter::All.accepts(RunStatus::Cancelled));
}

mod ui {
    use std::sync::Arc;

    use castor_core::options::{Choice, Cond, OptionSpec, Value};
    use castor_core::settings::Settings;
    use castor_core::{Family, FileKind};
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px, size};

    use crate::form::OptionsForm;
    use crate::model::{Model, ModelHandle};

    fn setup(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::theme::apply(castor_core::settings::Appearance::Light, None, cx);
            let reg = Arc::new(super::registry());
            let m = cx.new(|_| Model::new(reg, Settings::default()));
            cx.set_global(ModelHandle(m));
        });
    }

    fn options() -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("preset", vec![Choice::key("light"), Choice::key("strong")], "light"),
            OptionSpec::toggle("mark", false),
            OptionSpec::anchor("where", castor_core::options::Anchor::Center).when(Cond::True("mark")),
            OptionSpec::int("quality", 1, 100, 80).advanced(),
        ]
    }

    #[gpui_kit::test]
    fn form_generates_and_updates_values(cx: &mut TestAppContext) {
        setup(cx);
        let tool = super::tool("image.compress", Family::Image, &[FileKind::Image], &[], options());
        let (handle, form): (_, Entity<OptionsForm>) = cx.update(|cx| {
            gpui_kit::open_window(
                WindowOptions { window_bounds: Some(WindowBounds::Windowed(Bounds::new(Default::default(), size(px(400.), px(600.))))), ..Default::default() },
                cx,
                |window, cx| {
                    let v = castor_core::Values::defaults(&tool.options());
                    cx.new(|cx| OptionsForm::new(tool.clone(), v, window, cx))
                },
            )
            .expect("window")
        });
        cx.update(|cx| {
            let v = form.read(cx).values();
            assert_eq!(v.str("preset"), "light");
            assert!(!v.bool("mark"));
            assert_eq!(v.int("quality"), 80);
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            // Hidden by visible_if while "mark" is off.
            assert!(window.try_find("anchor-image.compress-where-center").is_none());
            window.click("seg-image.compress-preset-strong", cx);
            window.click("cb-image.compress-mark", cx);
            window.render_frame(cx);
            assert!(window.try_find("anchor-image.compress-where-center").is_some());
        })
        .unwrap();
        cx.update(|cx| {
            let v = form.read(cx).values();
            assert_eq!(v.get("preset"), Some(&Value::Text("strong".into())));
            assert!(v.bool("mark"));
        });
    }

    #[gpui_kit::test]
    fn palette_lists_tools_for_a_sentence(cx: &mut TestAppContext) {
        setup(cx);
        cx.update(|cx| {
            let items = crate::palette::entries("make this pdf smaller", cx);
            let first = items.first().expect("results");
            assert_eq!(first.title.as_ref(), castor_core::tr("pdf.compress.name"));
            let pages = crate::palette::entries("settings", cx);
            assert!(pages.iter().any(|i| matches!(i.target, crate::palette::Target::Nav(crate::model::Nav::Page(crate::model::PageId::Settings)))));
        });
    }
}

#[allow(dead_code)]
fn _unused(_: Choice, _: Cond, _: Value) {}
