//! The tool screen (DESIGN §4): breadcrumb, title, files (or live preview) on the left, the
//! generated options + output + run button on the right; runs stream `RunEvent`s into the rows.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use castor_core::history::{RunRecord, RunStatus};
use castor_core::job::Output;
use castor_core::runner::{self, OutputPolicy, RunEvent, RunHandle, RunRequest, Step};
use castor_core::{FileInfo, FileKind, Tool, ToolStatus, Values, units};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Selectable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::*;

use crate::form::{FormEvent, OptionsForm, persistable};
use crate::home::PAGE_PAD;
use crate::model::{Nav, PageId, model, navigate};
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t, ta};

pub const OPTIONS_W: f32 = 340.0;
const STATUS_W: f32 = 170.0;

thread_local! {
    /// The files column is narrow: drop the size column and shrink the status column.
    static COMPACT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
fn compact() -> bool {
    COMPACT.with(|c| c.get())
}
fn status_w() -> f32 {
    if compact() { 116.0 } else { STATUS_W }
}

/// Page padding and options width for the current window width (narrow windows get tighter).
fn layout(window: &Window) -> (f32, f32) {
    if window.viewport_size().width.as_f32() < 1100.0 { (24.0, 290.0) } else { (PAGE_PAD, OPTIONS_W) }
}
const ROW_H: f32 = 44.0;
const PREVIEW_SIDE: u32 = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct JobRow {
    pub label: String,
    pub file: Option<FileInfo>,
    pub status: JobStatus,
    pub progress: f32,
    pub message: Option<String>,
    pub outputs: Vec<Output>,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub secs: f32,
    pub error: Option<String>,
}

/// A run in progress (or just finished) on this screen.
pub struct RunView {
    pub handle: Option<RunHandle>,
    pub jobs: Vec<JobRow>,
    pub record: Option<RunRecord>,
    /// Inputs that were handed to the runner (to map jobs to rows).
    pub inputs: Vec<FileInfo>,
}

impl RunView {
    pub fn new(handle: RunHandle, inputs: Vec<FileInfo>) -> RunView {
        RunView { handle: Some(handle), jobs: Vec::new(), record: None, inputs }
    }

    pub fn running(&self) -> bool {
        self.record.is_none()
    }

    /// Apply one runner event. Returns true when the run is over.
    pub fn apply(&mut self, ev: RunEvent) -> bool {
        match ev {
            RunEvent::StepStarted { jobs, .. } => {
                let one_to_one = jobs.len() == self.inputs.len();
                self.jobs = jobs
                    .into_iter()
                    .enumerate()
                    .map(|(i, label)| JobRow {
                        label,
                        file: if one_to_one { self.inputs.get(i).cloned() } else { None },
                        status: JobStatus::Queued,
                        progress: 0.0,
                        message: None,
                        outputs: Vec::new(),
                        bytes_in: 0,
                        bytes_out: 0,
                        secs: 0.0,
                        error: None,
                    })
                    .collect();
            }
            RunEvent::JobStarted { job, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.status = JobStatus::Running;
                }
            }
            RunEvent::JobProgress { job, frac, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.progress = frac;
                }
            }
            RunEvent::JobMessage { job, msg, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.message = Some(msg);
                }
            }
            RunEvent::JobDone { job, outputs, bytes_in, bytes_out, secs, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.status = JobStatus::Done;
                    j.progress = 1.0;
                    j.outputs = outputs;
                    j.bytes_in = bytes_in;
                    j.bytes_out = bytes_out;
                    j.secs = secs;
                    j.message = None;
                }
            }
            RunEvent::JobFailed { job, error, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.status = JobStatus::Failed;
                    j.error = Some(error);
                    j.message = None;
                }
            }
            RunEvent::JobCancelled { job, .. } => {
                if let Some(j) = self.jobs.get_mut(job) {
                    j.status = JobStatus::Cancelled;
                    j.message = None;
                }
            }
            RunEvent::Finished(r) => {
                for j in &mut self.jobs {
                    if matches!(j.status, JobStatus::Queued | JobStatus::Running) {
                        j.status = JobStatus::Cancelled;
                    }
                }
                self.record = Some(*r);
                self.handle = None;
                return true;
            }
        }
        false
    }

    pub fn overall(&self) -> f32 {
        if self.jobs.is_empty() {
            return 0.0;
        }
        self.jobs
            .iter()
            .map(|j| match j.status {
                JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled => 1.0,
                _ => j.progress,
            })
            .sum::<f32>()
            / self.jobs.len() as f32
    }
}

/// Summary line: "3 files · 32 MB → 9.8 MB (−69 %) · 4.2 s".
pub fn summary_line(r: &RunRecord) -> String {
    let n = r.files_in.max(r.inputs.len());
    let mut parts = vec![if n == 1 { castor_core::tr("app.tool.one_file") } else { castor_core::tr_args("common.n_files", &[("count", &n.to_string())]) }];
    if r.bytes_in > 0 && r.bytes_out > 0 {
        parts.push(format!("{} → {} ({})", units::bytes(r.bytes_in), units::bytes(r.bytes_out), units::change(r.bytes_in, r.bytes_out)));
    } else if r.bytes_out > 0 {
        parts.push(units::bytes(r.bytes_out));
    }
    parts.push(units::duration(r.duration_secs as f64));
    parts.join(" · ")
}

#[derive(Clone, Copy, PartialEq)]
enum CompareMode {
    Side,
    Swipe,
}

pub struct ToolScreen {
    tool: Arc<dyn Tool>,
    id: String,
    focus: FocusHandle,
    files: Vec<FileInfo>,
    selected: Option<usize>,
    form: Entity<OptionsForm>,
    out_override: Option<PathBuf>,
    run: Option<RunView>,
    error_open: Option<usize>,
    preview: Option<(Arc<Image>, Option<String>)>,
    preview_error: Option<String>,
    preview_loading: bool,
    has_preview: Option<bool>,
    preview_gen: u64,
    preview_task: Option<Task<()>>,
    compare: CompareMode,
    compare_fit: bool,
    swipe: f32,
    list: UniformListScrollHandle,
    options_w: f32,
    /// Files being read in the background.
    loading: usize,
    /// `--run` arrived before the files were read: start once they are.
    start_pending: bool,
    _subs: Vec<Subscription>,
}

impl ToolScreen {
    pub fn new(id: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = model(cx).read(cx);
        let tool = m.registry.get(id).expect("tool exists");
        let values = m.tool_values(id);
        let form = cx.new(|cx| OptionsForm::new(tool.clone(), values, window, cx));
        let mut s = ToolScreen {
            tool,
            id: id.to_string(),
            focus: cx.focus_handle(),
            files: Vec::new(),
            selected: None,
            form: form.clone(),
            out_override: None,
            run: None,
            error_open: None,
            preview: None,
            preview_error: None,
            preview_loading: false,
            has_preview: None,
            preview_gen: 0,
            preview_task: None,
            compare: CompareMode::Side,
            compare_fit: true,
            swipe: 0.5,
            list: UniformListScrollHandle::new(),
            options_w: OPTIONS_W,
            loading: 0,
            start_pending: false,
            _subs: Vec::new(),
        };
        s.watch_form(&form, cx);
        s.refresh_preview(cx);
        s
    }

    fn watch_form(&mut self, form: &Entity<OptionsForm>, cx: &mut Context<Self>) {
        self._subs.push(cx.subscribe(form, |this, form, _: &FormEvent, cx| {
            let v = persistable(this.tool.as_ref(), form.read(cx).values());
            let id = this.id.clone();
            model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| {
                s.tool_values.insert(id, v);
            }));
            this.refresh_preview(cx);
        }));
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// Replace the option values (e.g. "Run again with the same settings").
    pub fn set_values(&mut self, values: Values, window: &mut Window, cx: &mut Context<Self>) {
        let mut v = Values::defaults(&self.tool.options());
        for (k, val) in values.0 {
            if v.0.contains_key(&k) {
                v.set(&k, val);
            }
        }
        let form = cx.new(|cx| OptionsForm::new(self.tool.clone(), v, window, cx));
        self.form = form.clone();
        self.watch_form(&form, cx);
        self.refresh_preview(cx);
        cx.notify();
    }

    /// Queue exactly these files (replacing the queue if no run is going on).
    pub fn set_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        self.files.clear();
        self.run = None;
        self.add_files(paths, window, cx);
    }

    pub fn add_files(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        if self.run.as_ref().is_some_and(|r| !r.running()) {
            // A finished run is on screen: new files start a new batch.
            self.run = None;
            self.files.clear();
        }
        let folders = self.tool.meta().inputs.folders;
        let tool = self.tool.clone();
        self.loading += 1;
        cx.spawn_in(window, async move |this, cx| {
            let infos = cx
                .background_executor()
                .spawn(async move {
                    let mut out = Vec::new();
                    for p in paths {
                        if p.is_dir() && !folders {
                            out.extend(castor_core::files::expand(&[p], true));
                        } else {
                            out.push(FileInfo::from_path(&p));
                        }
                    }
                    out.retain(|f| tool.accepts(f));
                    out
                })
                .await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.loading -= 1;
                for f in infos {
                    if !s.files.iter().any(|x| x.path == f.path) {
                        s.files.push(f);
                    }
                }
                if s.selected.is_none() && !s.files.is_empty() {
                    s.selected = Some(0);
                }
                s.refresh_preview(cx);
                if s.start_pending && s.loading == 0 {
                    s.start_pending = false;
                    s.start(window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn choose_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folders = self.tool.meta().inputs.folders;
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: folders, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update_in(cx, |s, window, cx| s.add_files(paths, window, cx));
            }
        })
        .detach();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        if self.is_running() {
            return;
        }
        self.files.clear();
        self.run = None;
        self.selected = None;
        self.refresh_preview(cx);
        cx.notify();
    }

    pub fn is_running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.running())
    }

    fn output_policy(&self, cx: &App) -> OutputPolicy {
        match &self.out_override {
            Some(p) => OutputPolicy::Folder(p.clone()),
            None => crate::model::settings(cx).output_policy(&self.id),
        }
    }

    fn change_output(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(t("app.tool.output_choose")) });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update(cx, |s, cx| {
                        s.out_override = Some(p);
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    // ---- preview -------------------------------------------------------------------------

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        if self.has_preview == Some(false) {
            return;
        }
        self.preview_gen += 1;
        let generation = self.preview_gen;
        let tool = self.tool.clone();
        let values = self.form.read(cx).values().clone();
        let input = self.selected.and_then(|i| self.files.get(i)).cloned();
        if tool.meta().inputs.min > 0 && input.is_none() && self.has_preview.is_some() {
            self.preview = None;
            cx.notify();
            return;
        }
        self.preview_loading = true;
        let had_input = input.is_some();
        self.preview_task = Some(cx.spawn(async move |this, cx| {
            // Debounce: a new change within 150 ms replaces this task (and drops it).
            cx.background_executor().timer(Duration::from_millis(150)).await;
            let res = cx
                .background_executor()
                .spawn(async move { tool.preview(input.as_ref(), &values, PREVIEW_SIDE).map_err(|e| format!("{e:#}")) })
                .await;
            let _ = this.update(cx, |s, cx| {
                if s.preview_gen != generation {
                    return;
                }
                s.preview_loading = false;
                match res {
                    Ok(Some(p)) => {
                        s.has_preview = Some(true);
                        s.preview_error = None;
                        s.preview = Some((Arc::new(Image::from_bytes(ImageFormat::Png, p.png)), p.caption));
                    }
                    Ok(None) => {
                        if s.has_preview.is_none() && (had_input || input_is_none_ok(&s.tool)) {
                            s.has_preview = Some(false);
                        }
                        s.preview = None;
                    }
                    Err(e) => {
                        s.has_preview = Some(true);
                        s.preview_error = Some(e);
                    }
                }
                cx.notify();
            });
        }));
    }

    // ---- run -----------------------------------------------------------------------------

    /// Start (after asking to download what is missing).
    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading > 0 {
            self.start_pending = true;
            return;
        }
        if self.is_running() || self.tool.unavailable().is_some() || self.tool.meta().status == ToolStatus::Soon {
            return;
        }
        let min = self.tool.meta().inputs.min;
        if self.files.len() < min {
            let msg = if min <= 1 { t("app.tool.need_one_file") } else { ta("app.tool.need_files", &[("count", &min.to_string())]) };
            window.push_notification(msg.to_string(), cx);
            return;
        }
        let values = self.form.read(cx).values().clone();
        let missing: Vec<castor_core::download::Asset> = self.tool.downloads(&values).into_iter().filter(|a| !a.is_present()).collect();
        if missing.is_empty() {
            self.launch(values, cx);
            return;
        }
        let total: u64 = missing.iter().map(|a| a.size).sum();
        let this = cx.entity().downgrade();
        let k = tokens(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let this = this.clone();
            let values = values.clone();
            dialog
                .title(t("app.consent.title"))
                .w(px(480.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .child(ui::roman().text_color(k.ink).child(t("app.consent.body")))
                        .child(ui::rule_strong(&k))
                        .children(missing.iter().map(|a| {
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(10.0))
                                .child(ui::mono().flex_1().truncate().child(a.id.to_string()))
                                .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(a.license.to_string()))
                                .child(ui::sans().w(px(70.0)).text_right().child(if a.size > 0 { units::bytes(a.size) } else { "?".into() }))
                        }))
                        .child(ui::hrule(k.rule, 1.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .child(ui::sans().flex_1().font_weight(FontWeight::BOLD).child(t("app.consent.total")))
                                .child(ui::sans().font_weight(FontWeight::BOLD).child(units::bytes(total))),
                        )
                        .child(ui::rule_strong(&k))
                        .child(ui::roman().text_size(px(13.5)).italic().text_color(k.muted).child(t("app.consent.once"))),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().child(Button::new("consent-cancel").label(t("common.cancel"))))
                        .child(DialogAction::new().child(Button::new("consent-ok").primary().label(t("app.consent.ok")).on_click({
                            let this = this.clone();
                            let values = values.clone();
                            move |_, _, cx| {
                                let values = values.clone();
                                let _ = this.update(cx, |s, cx| s.launch(values, cx));
                            }
                        }))),
                )
        });
    }

    fn launch(&mut self, values: Values, cx: &mut Context<Self>) {
        let usable: Vec<FileInfo> = self.files.iter().filter(|f| self.tool.accepts(f)).cloned().collect();
        let s = crate::model::settings(cx);
        let req = RunRequest {
            steps: vec![Step { tool: self.tool.clone(), values }],
            inputs: usable.clone(),
            output: self.output_policy(cx),
            chain_name: None,
            max_parallel: s.max_parallel,
            record: true,
        };
        let handle = runner::start(req);
        let rx = handle.events.clone();
        self.run = Some(RunView::new(handle, usable));
        self.error_open = None;
        self.form.update(cx, |f, cx| f.set_disabled(true, cx));
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(33)).await;
                let mut evs = Vec::new();
                let mut closed = false;
                loop {
                    match rx.try_recv() {
                        Ok(e) => evs.push(e),
                        Err(crossbeam_channel::TryRecvError::Empty) => break,
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            closed = true;
                            break;
                        }
                    }
                }
                if evs.is_empty() && !closed {
                    continue;
                }
                let done = this.update(cx, |s, cx| s.apply_events(evs, cx)).unwrap_or(true);
                if done || closed {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_events(&mut self, evs: Vec<RunEvent>, cx: &mut Context<Self>) -> bool {
        let mut finished = false;
        if let Some(run) = &mut self.run {
            for e in evs {
                if run.apply(e) {
                    finished = true;
                }
            }
        }
        if finished {
            self.form.update(cx, |f, cx| f.set_disabled(false, cx));
            if let Some(r) = self.run.as_ref().and_then(|r| r.record.clone()) {
                let reveal = crate::model::settings(cx).reveal_when_done;
                if reveal {
                    if let Some(p) = r.outputs.first() {
                        let _ = castor_core::process::reveal(p);
                    }
                }
                model(cx).update(cx, |m, cx| m.push_run(r, cx));
            }
            // Show the first result in the compare view.
            if self.selected.is_none() {
                self.selected = Some(0);
            }
        }
        cx.notify();
        finished
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.run.as_ref().and_then(|r| r.handle.as_ref()) {
            h.cancel();
        }
        cx.notify();
    }

    fn remove_file(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.is_running() || ix >= self.files.len() {
            return;
        }
        self.files.remove(ix);
        if self.files.is_empty() {
            self.selected = None;
        } else if let Some(s) = self.selected {
            self.selected = Some(s.min(self.files.len() - 1));
        }
        self.refresh_preview(cx);
        cx.notify();
    }

    fn verb(&self) -> String {
        let k = format!("{}.verb", self.id);
        if castor_core::i18n::has(&k) { castor_core::tr(&k) } else { castor_core::tr("common.run") }
    }

    fn run_label(&self) -> String {
        let n = self.files.iter().filter(|f| self.tool.accepts(f)).count();
        let verb = self.verb();
        match n {
            0 => verb,
            1 => format!("{verb} {}", castor_core::tr("app.tool.one_file")),
            n => format!("{verb} {}", castor_core::tr_args("common.n_files", &[("count", &n.to_string())])),
        }
    }
}

fn input_is_none_ok(tool: &Arc<dyn Tool>) -> bool {
    tool.meta().inputs.min == 0
}

fn is_image(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "tif" | "tiff")
    )
}

impl Focusable for ToolScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ToolScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let (pad, opt_w) = layout(window);
        self.options_w = opt_w;
        let left_w = window.viewport_size().width.as_f32() - crate::shell::SIDEBAR_W - 2.0 * pad - opt_w - 28.0;
        COMPACT.with(|c| c.set(left_w < 520.0));
        let meta = self.tool.meta();
        let unavailable = self.tool.unavailable();
        let no_input = meta.inputs.min == 0 && meta.inputs.kinds.is_empty() && meta.inputs.extensions.is_empty();
        let n_files = self.files.len();

        let crumbs = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.0))
            .text_size(px(13.5))
            .child(ui::link("crumb-tools", &k, t("app.nav.tools"), |_, _, cx| navigate(Nav::Page(PageId::Tools), cx)))
            .child(ui::sans().text_color(k.faint).child("/"))
            .child(ui::sans().text_color(k.muted).child(meta.family.name()))
            .child(ui::sans().text_color(k.faint).child("/"))
            .child(ui::sans().text_color(k.ink).child(meta.name()));

        let header = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(crumbs)
            .child(ui::page_header(
                cx,
                meta.name().into(),
                (n_files > 0).then(|| n_files.to_string()),
                Some(meta.desc().into_any_element()),
            ))
            .when_some(unavailable.clone(), |d, why| {
                d.child(
                    div()
                        .mt(px(10.0))
                        .px(px(12.0))
                        .py(px(8.0))
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .border_1()
                        .border_color(k.warn)
                        .rounded(px(3.0))
                        .bg(Hsla { a: 0.08, ..k.warn })
                        .child(ui::icon("triangle-alert", k.warn))
                        .child(ui::sans().text_color(k.ink).child(why)),
                )
            });

        let left = if no_input {
            self.render_preview_figure(&k, 1, true).into_any_element()
        } else {
            let mut col = div().id("files-col").flex().flex_col().gap(px(14.0)).size_full().min_h(px(0.0)).overflow_y_scroll();
            if self.has_preview == Some(true) && n_files > 0 {
                col = col.child(div().h(px(280.0)).flex_none().child(self.render_preview_figure(&k, 1, false)));
            }
            let fig_n = if self.has_preview == Some(true) && n_files > 0 { 2 } else { 1 };
            col = col.child(self.render_files(&k, window, cx));
            if let Some(cmp) = self.render_compare(&k, fig_n, window, cx) {
                col = col.child(cmp);
            }
            col.into_any_element()
        };

        div()
            .id(SharedString::from(format!("tool-{}", self.id)))
            .key_context("ToolScreen")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let ks = &ev.keystroke;
                if ks.key == "enter" && (ks.modifiers.control || ks.modifiers.platform) {
                    this.start(window, cx);
                }
            }))
            .child(div().px(px(pad)).pb(px(18.0)).child(header))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(28.0))
                    .px(px(pad))
                    .pb(px(20.0))
                    .child(div().flex_1().min_w(px(0.0)).h_full().child(left))
                    .child(self.render_options(&k, cx)),
            )
    }
}

impl ToolScreen {
    fn render_preview_figure(&self, k: &Tokens, n: usize, big: bool) -> impl IntoElement {
        let caption = self.preview.as_ref().and_then(|p| p.1.clone()).unwrap_or_else(|| castor_core::tr("app.tool.preview"));
        let body: AnyElement = if let Some(e) = &self.preview_error {
            ui::roman().text_color(k.bad).child(e.clone()).into_any_element()
        } else if let Some((img_, _)) = &self.preview {
            img(img_.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element()
        } else {
            ui::roman().italic().text_color(k.faint).child(if self.preview_loading { t("app.tool.preview_loading") } else { t("app.tool.preview_none") }).into_any_element()
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .size_full()
            .child(
                div()
                    .flex_1()
                    .min_h(px(if big { 240.0 } else { 120.0 }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p(px(12.0))
                    .border_1()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .bg(k.raised)
                    .child(body),
            )
            .child(div().flex().justify_center().child(ui::caption(k, format!("{} {n}", castor_core::tr("app.caption.figure")), "/", caption)))
    }

    fn render_files(&mut self, k: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let k = *k;
        let running = self.is_running();
        if self.files.is_empty() && self.run.is_none() {
            let link = k.link;
            let tint = Hsla { a: 0.08, ..k.link };
            return div()
                .id("tool-drop")
                .size_full()
                .min_h(px(200.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.0))
                .border_1()
                .border_dashed()
                .border_color(k.rule)
                .rounded(px(3.0))
                .bg(k.raised)
                .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(link).bg(tint))
                .child(ui::icon("file-down", k.muted).size(px(28.0)))
                .child(ui::roman().text_color(k.ink).child(ta("app.tool.drop", &[("kinds", &crate::home::input_label(self.tool.as_ref()))])))
                .child(
                    Button::new("tool-choose")
                        .label(t("app.home.choose"))
                        .on_click(cx.listener(|this, _, window, cx| this.choose_files(window, cx))),
                )
                .into_any_element();
        }

        // Rows: job rows during/after a run, otherwise the queue.
        let rows: Vec<JobRow> = match &self.run {
            Some(r) if !r.jobs.is_empty() => r.jobs.clone(),
            _ => self
                .files
                .iter()
                .map(|f| JobRow {
                    label: f.name(),
                    file: Some(f.clone()),
                    status: JobStatus::Queued,
                    progress: 0.0,
                    message: None,
                    outputs: vec![],
                    bytes_in: f.size,
                    bytes_out: 0,
                    secs: 0.0,
                    error: None,
                })
                .collect(),
        };
        let n = rows.len();
        // Natural height up to what the window allows (the list scrolls beyond).
        let avail = (window.viewport_size().height.as_f32() - 330.0 - if self.has_preview == Some(true) { 294.0 } else { 0.0 }).max(ROW_H * 3.0);
        let list_h = (ROW_H * n as f32).min(avail).max(ROW_H);
        let in_run = self.run.is_some();
        let selected = self.selected;
        let error_open = self.error_open;
        let this = cx.entity().downgrade();
        let rows_rc = std::rc::Rc::new(rows);
        let rows2 = rows_rc.clone();
        let list = uniform_list("file-rows", n, move |range, _window, cx| {
            range
                .map(|i| {
                    let r = &rows2[i];
                    file_row(&k, i, r, selected == Some(i), error_open == Some(i), in_run, running, this.clone(), cx)
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.list)
        .w_full()
        .h(px(list_h));

        let error_details: Option<String> = error_open.and_then(|i| rows_rc.get(i)).and_then(|r| r.error.clone());

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(30.0))
            .px(px(6.0))
            .gap(px(10.0))
            .child(div().w(px(36.0)))
            .child(ui::sans().flex_1().font_weight(FontWeight::BOLD).child(t("app.table.name")))
            .when(!compact(), |d| d.child(ui::sans().w(px(72.0)).text_right().font_weight(FontWeight::BOLD).child(t("app.table.size"))))
            .child(ui::sans().w(px(status_w())).font_weight(FontWeight::BOLD).pl(px(12.0)).child(t("app.table.status")))
            .child(div().w(px(70.0)));

        let finished = self.run.as_ref().and_then(|r| r.record.clone());

        div()
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .gap(px(6.0))
            .child(ui::caption(&k, format!("{} 1", castor_core::tr("app.caption.table")), "—", ta("app.tool.files_caption", &[("count", &n.to_string())])))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .child(ui::rule_strong(&k))
                    .child(header)
                    .child(ui::hrule(k.rule, 1.0))
                    .child(list)
                    .child(ui::rule_strong(&k)),
            )
            .when_some(error_details, |d, e| {
                d.child(
                    div()
                        .id("error-details")
                        .max_h(px(140.0))
                        .overflow_y_scroll()
                        .p(px(10.0))
                        .border_1()
                        .border_color(k.bad)
                        .rounded(px(3.0))
                        .bg(Hsla { a: 0.06, ..k.bad })
                        .child(ui::mono().text_color(k.ink).child(e)),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        Button::new("add-files")
                            .small()
                            .icon(gpui_kit::component::IconName::Plus)
                            .label(t("app.tool.add_files"))
                            .disabled(running)
                            .on_click(cx.listener(|this, _, window, cx| this.choose_files(window, cx))),
                    )
                    .child(
                        Button::new("clear-files")
                            .small()
                            .label(t("app.common.clear"))
                            .disabled(running)
                            .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                    )
            )
            .when_some(finished, |d, r| d.child(self.render_summary(&k, &r, cx)))
            .into_any_element()
    }

    fn render_summary(&self, k: &Tokens, r: &RunRecord, _cx: &mut Context<Self>) -> impl IntoElement {
        let color = match r.status {
            RunStatus::Complete => k.good,
            RunStatus::Partial => k.warn,
            RunStatus::Failed => k.bad,
            _ => k.muted,
        };
        let big_saving = r.status == RunStatus::Complete && r.bytes_in > 0 && r.bytes_out > 0 && r.bytes_out * 2 < r.bytes_in;
        let done_key = format!("{}.done", self.id);
        let quip = if r.status == RunStatus::Complete && castor_core::i18n::has(&done_key) {
            Some(castor_core::tr_args(&done_key, &[("count", &r.files_out.to_string()), ("saved", &units::bytes(r.bytes_in.saturating_sub(r.bytes_out)))]))
        } else if big_saving {
            Some(castor_core::tr("app.tool.done_big_saving"))
        } else {
            None
        };
        let first = r.outputs.first().cloned();
        let status_text = match r.status {
            RunStatus::Complete => t("app.status.complete"),
            RunStatus::Partial => t("app.status.partial"),
            RunStatus::Failed => t("app.status.failed"),
            RunStatus::Cancelled => t("app.status.cancelled"),
            RunStatus::Running => t("app.status.running"),
        };
        div()
            .flex()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(10.0))
            .pt(px(4.0))
            .child(ui::sans().text_color(color).font_weight(FontWeight::BOLD).child(status_text))
            .child(ui::sans().text_color(k.ink).child(summary_line(r)))
            .when_some(quip, |d, q| d.child(ui::roman().italic().text_color(k.muted).child(q)))
            .when_some(first, |d, p| {
                d.child(
                    Button::new("show-in-folder")
                        .small()
                        .primary()
                        .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("folder-open")))
                        .label(t("common.reveal"))
                        .on_click(move |_, _, _| {
                            let _ = castor_core::process::reveal(&p);
                        }),
                )
            })
    }

    fn render_compare(&mut self, k: &Tokens, fig: usize, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.run.as_ref()?;
        run.record.as_ref()?;
        let i = self.selected.unwrap_or(0);
        let job = run.jobs.get(i).or_else(|| run.jobs.iter().find(|j| j.status == JobStatus::Done))?;
        let before = job.file.as_ref().map(|f| f.path.clone()).or_else(|| run.inputs.first().map(|f| f.path.clone()))?;
        let after = job.outputs.iter().map(|o| o.path.clone()).find(|p| is_image(p))?;
        if !is_image(&before) {
            return None;
        }
        let (pad, opt_w) = layout(window);
        let avail_w = (window.viewport_size().width.as_f32() - crate::shell::SIDEBAR_W - 2.0 * pad - opt_w - 28.0).max(320.0);
        LAST_AVAIL_W.with(|w| *w.borrow_mut() = avail_w);
        LAST_PAD.with(|p| *p.borrow_mut() = pad);
        let h = 300.0;
        let fit = self.compare_fit;
        let pane = |p: PathBuf, label: SharedString| {
            div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .id(SharedString::from(format!("cmp-{}", p.display())))
                        .h(px(h))
                        .border_1()
                        .border_color(k.rule)
                        .bg(k.raised)
                        .when(fit, |d| d.overflow_hidden())
                        .when(!fit, |d| d.overflow_scroll())
                        .child(if fit { img(p).size_full().object_fit(ObjectFit::Contain) } else { img(p).object_fit(ObjectFit::None) }),
                )
                .child(ui::sans().text_size(px(12.5)).text_color(k.muted).child(label))
        };
        let body: AnyElement = match self.compare {
            CompareMode::Side => div()
                .flex()
                .flex_row()
                .gap(px(10.0))
                .child(pane(before.clone(), t("app.compare.before")))
                .child(pane(after.clone(), t("app.compare.after")))
                .into_any_element(),
            CompareMode::Swipe => {
                let frac = self.swipe;
                let w = avail_w;
                div()
                    .id("swipe")
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .border_1()
                    .border_color(k.rule)
                    .bg(k.raised)
                    .overflow_hidden()
                    .cursor_col_resize()
                    .child(img(after.clone()).absolute().top_0().left_0().w(px(w)).h(px(h)).object_fit(ObjectFit::Contain))
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .h(px(h))
                            .w(px(w * frac))
                            .overflow_hidden()
                            .child(img(before.clone()).w(px(w)).h(px(h)).object_fit(ObjectFit::Contain)),
                    )
                    .child(div().absolute().top_0().left(px(w * frac - 1.0)).w(px(2.0)).h(px(h)).bg(k.ink))
                    .on_mouse_move(cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                        if ev.pressed_button == Some(MouseButton::Left) {
                            let _ = ev;
                            this.swipe_from(ev.position, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, _, cx| this.swipe_from(ev.position, cx)))
                    .into_any_element()
            }
        };
        let k2 = *k;
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .flex_none()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(10.0))
                        .child(ui::segmented(
                            "cmp-mode",
                            &k2,
                            vec![("side".into(), t("app.compare.side")), ("swipe".into(), t("app.compare.swipe"))],
                            if self.compare == CompareMode::Side { "side" } else { "swipe" },
                            std::rc::Rc::new(move |v, _, cx| {
                                let v = v.to_string();
                                let _ = this.update(cx, |s, cx| {
                                    s.compare = if v == "side" { CompareMode::Side } else { CompareMode::Swipe };
                                    cx.notify();
                                });
                            }),
                        ))
                        .when(self.compare == CompareMode::Side, |d| {
                            d.child(ui::segmented(
                                "cmp-fit",
                                &k2,
                                vec![("fit".into(), t("app.compare.fit")), ("100".into(), "100 %".into())],
                                if fit { "fit" } else { "100" },
                                std::rc::Rc::new(move |v, _, cx| {
                                    let v = v.to_string();
                                    let _ = this2.update(cx, |s, cx| {
                                        s.compare_fit = v == "fit";
                                        cx.notify();
                                    });
                                }),
                            ))
                        }),
                )
                .child(body)
                .child(ui::caption(k, format!("{} {fig}", castor_core::tr("app.caption.figure")), "/", t("app.compare.caption")))
                .into_any_element(),
        )
    }

    fn swipe_from(&mut self, pos: Point<Pixels>, cx: &mut Context<Self>) {
        // The swipe area starts after the sidebar and the page padding.
        let x = pos.x.as_f32() - crate::shell::SIDEBAR_W - LAST_PAD.with(|p| *p.borrow());
        let w = self.swipe_width(cx);
        self.swipe = (x / w).clamp(0.0, 1.0);
        cx.notify();
    }

    fn swipe_width(&self, _cx: &App) -> f32 {
        // Matches render_compare's avail_w; recomputed from the last known viewport.
        LAST_AVAIL_W.with(|w| *w.borrow())
    }

    fn render_options(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.is_running();
        let unavailable = self.tool.unavailable().is_some() || self.tool.meta().status == ToolStatus::Soon;
        let min = self.tool.meta().inputs.min;
        let enough = self.files.iter().filter(|f| self.tool.accepts(f)).count() >= min.max(if min == 0 { 0 } else { 1 });
        let policy = self.output_policy(cx);
        let out_text: String = match (&policy, self.out_override.is_some()) {
            (OutputPolicy::NextToInput { .. }, _) => castor_core::tr("app.tool.output_next_to"),
            (OutputPolicy::Folder(p), _) => p.display().to_string(),
        };
        let progress = self.run.as_ref().filter(|r| r.running()).map(|r| r.overall());
        let label = self.run_label();
        let is_fav = model(cx).read(cx).is_favorite(&self.id);
        let id = self.id.clone();
        div()
            .id("options-col")
            .w(px(self.options_w))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .pr(px(4.0))
                    .pb(px(12.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .child(ui::sub_title(k, 1, t("app.tool.options")).flex_1())
                            .child(
                                Button::new("fav-toggle")
                                    .ghost()
                                    .small()
                                    .tooltip(if is_fav { t("app.tool.unfavorite") } else { t("app.tool.favorite") })
                                    .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("star")))
                                    .when(is_fav, |b| b.selected(true))
                                    .on_click(move |_, _, cx| {
                                        let id = id.clone();
                                        model(cx).update(cx, |m, cx| m.toggle_favorite(&id, cx));
                                    }),
                            ),
                    )
                    .child(self.form.clone())
                    .child(ui::hrule(k.rule, 1.0))
                    .child(ui::sub_title(k, 2, t("app.tool.output")))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(ui::icon("folder", k.muted).size(px(14.0)))
                            .child(ui::sans().flex_1().min_w(px(0.0)).text_size(px(13.5)).text_color(k.ink).text_ellipsis_start().whitespace_nowrap().overflow_hidden().child(out_text))
                            .when(self.out_override.is_some(), |d| {
                                d.child(ui::link("out-reset", k, t("app.tool.output_reset"), cx.listener(|this, _, _, cx| {
                                    this.out_override = None;
                                    cx.notify();
                                })))
                            })
                            .child(ui::link("out-change", k, t("app.tool.output_change"), cx.listener(|this, _, window, cx| this.change_output(window, cx)))),
                    )
                    .child(ui::hrule(k.rule, 1.0))
                    .when_some(progress, |d, p| {
                        d.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(6.0))
                                .child(Progress::new("overall").value(p * 100.0))
                                .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(format!("{:.0} %", p * 100.0))),
                        )
                    })
                    .child(if running {
                        Button::new("cancel-run")
                            .label(t("common.cancel"))
                            .w_full()
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                            .into_any_element()
                    } else {
                        Button::new("run")
                            .primary()
                            .w_full()
                            .label(label)
                            .disabled(unavailable || !enough)
                            .on_click(cx.listener(|this, _, window, cx| this.start(window, cx)))
                            .into_any_element()
                    })
                    .child(ui::sans().text_size(px(12.5)).text_color(k.faint).child(format!("{}↵", ui::cmd_key()))),
            )
    }
}

thread_local! {
    static LAST_PAD: std::cell::RefCell<f32> = const { std::cell::RefCell::new(PAGE_PAD) };
    static LAST_AVAIL_W: std::cell::RefCell<f32> = const { std::cell::RefCell::new(600.0) };
}

#[allow(clippy::too_many_arguments)]
fn file_row(
    k: &Tokens,
    i: usize,
    r: &JobRow,
    selected: bool,
    error_open: bool,
    in_run: bool,
    running: bool,
    this: WeakEntity<ToolScreen>,
    cx: &mut App,
) -> AnyElement {
    let thumb = r.file.as_ref().and_then(|f| crate::thumbs::get(f, cx));
    let kind = r.file.as_ref().map(|f| f.kind).unwrap_or(FileKind::Other);
    let size_text = if r.bytes_in > 0 { units::bytes(r.bytes_in) } else { r.file.as_ref().map(|f| units::bytes(f.size)).unwrap_or_default() };
    let status: AnyElement = match r.status {
        JobStatus::Queued => ui::sans().text_color(k.faint).child(if in_run { t("app.status.waiting") } else { t("app.status.queued") }).into_any_element(),
        JobStatus::Running => div()
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(Progress::new(("row-progress", i)).value(r.progress * 100.0))
            .when_some(r.message.clone(), |d, m| d.child(ui::sans().text_size(px(12.5)).text_color(k.muted).truncate().child(m)))
            .into_any_element(),
        JobStatus::Done => {
            let change = if r.bytes_in > 0 && r.bytes_out > 0 { format!(" ({})", units::change(r.bytes_in, r.bytes_out)) } else { String::new() };
            let note = r.outputs.iter().find_map(|o| o.note.clone());
            div()
                .flex()
                .flex_col()
                .child(ui::sans().text_color(k.good).child(format!("✓ {}{change}", units::bytes(r.bytes_out))))
                .when_some(note, |d, n| d.child(ui::sans().text_size(px(12.5)).text_color(k.muted).truncate().child(n)))
                .into_any_element()
        }
        JobStatus::Failed => {
            let first = r.error.clone().unwrap_or_default().lines().next().unwrap_or("").to_string();
            let t2 = this.clone();
            div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(ui::sans().flex_1().min_w(px(0.0)).text_color(k.bad).truncate().child(format!("✗ {first}")))
                .child(
                    ui::sans()
                        .id(("err-details", i))
                        .text_size(px(12.5))
                        .text_color(k.link)
                        .cursor_pointer()
                        .child(if error_open { t("app.tool.hide_details") } else { t("app.tool.details") })
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = t2.update(cx, |s, cx| {
                                s.error_open = if s.error_open == Some(i) { None } else { Some(i) };
                                cx.notify();
                            });
                        }),
                )
                .into_any_element()
        }
        JobStatus::Cancelled => ui::sans().text_color(k.muted).child(t("app.status.cancelled")).into_any_element(),
    };
    let out = r.outputs.first().map(|o| o.path.clone());
    let src = r.file.as_ref().map(|f| f.path.clone());
    let open_path = out.clone().or(src.clone());
    let reveal_path = out.or(src);
    let t_sel = this.clone();
    let t_rm = this.clone();
    div()
        .id(("file-row", i))
        .w_full()
        .h(px(ROW_H))
        .px(px(6.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(10.0))
        .when(selected, |d| d.bg(k.surface))
        .hover(|s| s.bg(k.surface))
        .on_click(move |_, _, cx| {
            let _ = t_sel.update(cx, |s, cx| {
                s.selected = Some(i);
                s.refresh_preview(cx);
                cx.notify();
            });
        })
        .child(
            div()
                .size(px(36.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(2.0))
                .overflow_hidden()
                .bg(k.surface)
                .child(match thumb {
                    Some(im) => img(im).size(px(36.0)).object_fit(ObjectFit::Cover).into_any_element(),
                    None => ui::icon(kind_icon(kind), k.muted).into_any_element(),
                }),
        )
        .child(ui::sans().flex_1().min_w(px(0.0)).text_color(k.ink).truncate().child(r.label.clone()))
        .when(!compact(), |d| d.child(ui::sans().w(px(72.0)).text_right().text_color(k.muted).child(size_text)))
        .child(div().w(px(status_w())).pl(px(12.0)).child(status))
        .child(
            div()
                .w(px(70.0))
                .flex()
                .flex_row()
                .justify_end()
                .gap(px(2.0))
                .when_some(open_path, |d, p| {
                    d.child(Button::new(("row-open", i)).ghost().xsmall().tooltip(t("common.open")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("external-link"))).on_click(move |_, _, _| {
                        let _ = castor_core::process::open(&p);
                    }))
                })
                .when_some(reveal_path, |d, p| {
                    d.child(Button::new(("row-reveal", i)).ghost().xsmall().tooltip(t("common.reveal")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("folder-open"))).on_click(move |_, _, _| {
                        let _ = castor_core::process::reveal(&p);
                    }))
                })
                .when(!in_run && !running, |d| {
                    d.child(Button::new(("row-remove", i)).ghost().xsmall().tooltip(t("app.tool.remove")).icon(gpui_kit::component::IconName::Close).on_click(move |_, _, cx| {
                        let _ = t_rm.update(cx, |s, cx| s.remove_file(i, cx));
                    }))
                }),
        )
        .into_any_element()
}

pub fn kind_icon(k: FileKind) -> &'static str {
    match k {
        FileKind::Pdf => "file-text",
        FileKind::Image | FileKind::Raw | FileKind::Svg => "image",
        FileKind::Video => "film",
        FileKind::Audio => "music",
        FileKind::Archive => "file-archive",
        FileKind::Table => "sheet",
        FileKind::Font => "type",
        _ => "file",
    }
}
