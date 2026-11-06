//! Trim & Caption Studio: one video or audio file. The frame at the playhead on the left, the
//! tools on the right (Cut · Transcript · Captions), the timeline (filmstrip, waveform, cuts,
//! pauses, selection, playhead) below, and the export in the bottom bar.
//!
//! Castor has no real-time player yet: the preview follows the playhead (scrub), and "Play"
//! opens the selected range in the system player.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use castor_core::history::RunRecord;
use castor_core::options::Value;
use castor_core::runner::{self, OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{FileInfo, units};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::{DialogAction, DialogClose, DialogFooter};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::media_engine::{self as eng, Cue, Export, MediaEdit, Word};
use crate::model::model;
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t, ta};

const PANEL_W: f32 = 320.0;
const TIMELINE_H: f32 = 156.0;
const STRIP_FRAMES: usize = 18;
const WAVE_BUCKETS: usize = 1600;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Cut,
    Transcript,
    Captions,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Cut, Tab::Transcript, Tab::Captions];
    fn key(self) -> &'static str {
        match self {
            Tab::Cut => "cut",
            Tab::Transcript => "transcript",
            Tab::Captions => "captions",
        }
    }
}

/// A job running through the runner from the studio (transcription or export).
struct Job {
    handle: Option<runner::RunHandle>,
    frac: f32,
    message: Option<String>,
    record: Option<RunRecord>,
    transcribe: bool,
}

/// Short time for the studio: "0:42", "1:05.4", "1:02:03".
fn tc(t: f64) -> String {
    let t = t.max(0.0);
    let (h, m, sec) = ((t / 3600.0) as u64, ((t / 60.0) as u64) % 60, t % 60.0);
    let s = if (sec - sec.round()).abs() < 0.05 { format!("{:02}", sec.round() as u64 % 60) } else { format!("{:04.1}", sec) };
    if h > 0 { format!("{h}:{m:02}:{s}") } else { format!("{m}:{s}") }
}

fn bgra_image(png: &[u8]) -> Option<Arc<RenderImage>> {
    let mut rgba = image::load_from_memory(png).ok()?.to_rgba8();
    for p in rgba.pixels_mut() {
        p.0.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(rgba)])))
}

pub struct MediaStudio {
    focus: FocusHandle,
    path: Option<PathBuf>,
    error: Option<String>,
    loading: bool,
    edit: Option<MediaEdit>,
    undo: Vec<MediaEdit>,
    size: (u32, u32),
    tab: Tab,
    playhead: f64,
    sel: Option<(f64, f64)>,
    dragging: Option<f64>,
    zoom: f32,
    frame: Option<Arc<RenderImage>>,
    frame_task: Option<Task<()>>,
    frame_gen: u64,
    strip: Vec<Option<Arc<RenderImage>>>,
    wave: Vec<(f32, f32)>,
    silences: Vec<(f64, f64)>,
    finding: bool,
    noise: i64,
    min_pause: f64,
    word_sel: Option<(usize, usize)>,
    cue_sel: Option<usize>,
    cue_text: Entity<InputState>,
    job: Option<Job>,
    timeline_origin: Rc<std::cell::Cell<(f32, f32)>>,
    timeline_scroll: ScrollHandle,
    old_images: Vec<Arc<RenderImage>>,
    _poll: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl MediaStudio {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let cue_text = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.media.cue_ph")));
        let subs = vec![cx.subscribe(&cue_text, |this, st, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let v = st.read(cx).value().to_string();
                if let (Some(i), Some(e)) = (this.cue_sel, this.edit.as_mut()) {
                    if let Some(c) = e.cues.get_mut(i) {
                        c.text = v;
                        cx.notify();
                    }
                }
            }
        })];
        MediaStudio {
            focus: cx.focus_handle(),
            path: None,
            error: None,
            loading: false,
            edit: None,
            undo: Vec::new(),
            size: (16, 9),
            tab: Tab::Cut,
            playhead: 0.0,
            sel: None,
            dragging: None,
            zoom: 1.0,
            frame: None,
            frame_task: None,
            frame_gen: 0,
            strip: Vec::new(),
            wave: Vec::new(),
            silences: Vec::new(),
            finding: false,
            noise: -35,
            min_pause: 0.7,
            word_sel: None,
            cue_sel: None,
            cue_text,
            job: None,
            timeline_origin: Rc::new(std::cell::Cell::new((0.0, 0.0))),
            timeline_scroll: ScrollHandle::new(),
            old_images: Vec::new(),
            _poll: None,
            _subs: subs,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    pub fn set_tab_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(tb) = Tab::ALL.iter().find(|x| x.key() == key) {
            self.tab = *tb;
            cx.notify();
        }
    }

    // ---- opening ------------------------------------------------------------------------

    pub fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if self.job.as_ref().is_some_and(|j| j.handle.is_some()) {
            return;
        }
        self.old_images.extend(self.frame.take());
        self.old_images.extend(self.strip.drain(..).flatten());
        self.path = Some(path.clone());
        self.error = None;
        self.loading = true;
        self.edit = None;
        self.undo.clear();
        self.wave.clear();
        self.silences.clear();
        self.sel = None;
        self.playhead = 0.0;
        self.word_sel = None;
        self.cue_sel = None;
        self.job = None;
        self.zoom = 1.0;
        let p = path.clone();
        let task = cx.background_executor().spawn(async move { engine_media::probe(&p) });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.loading = false;
                match res {
                    Ok(pr) if pr.duration > 0.0 && (pr.has_video || pr.has_audio) => {
                        let (w, h) = if pr.rotation % 180 == 90 { (pr.height, pr.width) } else { (pr.width, pr.height) };
                        s.size = if pr.has_video && w > 0 && h > 0 { (w, h) } else { (16, 9) };
                        let mut edit = MediaEdit::new(path.clone(), pr.duration, pr.has_video);
                        let setup = super::take_setup(cx);
                        if let Some(script) = setup.script.and_then(|p| std::fs::read_to_string(p).ok()) {
                            match serde_json::from_str::<MediaEdit>(&script) {
                                Ok(e) => edit = MediaEdit { source: edit.source, duration: edit.duration, has_video: edit.has_video, ..e },
                                Err(err) => tracing::warn!("--script: {err}"),
                            }
                        }
                        if let Some(m) = setup.mode {
                            s.set_tab_key(&m, cx);
                        }
                        s.playhead = edit.keep.first().map(|k| k.0).unwrap_or(0.0);
                        s.edit = Some(edit);
                        s.load_media(pr.has_video, window, cx);
                    }
                    Ok(_) => s.error = Some(t("app.media.err.unreadable").to_string()),
                    Err(e) => {
                        tracing::warn!("probe {}: {e:#}", path.display());
                        s.error = Some(format!("{e:#}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        self.start_poll(window, cx);
        cx.notify();
    }

    fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update_in(cx, |s, window, cx| s.open(p, window, cx));
                }
            }
        })
        .detach();
    }

    /// Waveform, filmstrip and first frame, in the background.
    fn load_media(&mut self, video: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        let (path, dur) = (e.source.clone(), e.duration);
        let p = path.clone();
        let wave = cx.background_executor().spawn(async move { engine_media::waveform(&p, WAVE_BUCKETS, &castor_core::CancelToken::new()) });
        cx.spawn_in(window, async move |this, cx| {
            let w = wave.await;
            let _ = this.update(cx, |s, cx| {
                if s.path.as_deref() == Some(path.as_path()) {
                    s.wave = w.unwrap_or_default();
                    cx.notify();
                }
            });
        })
        .detach();
        if video {
            self.strip = vec![None; STRIP_FRAMES];
            let path = self.path.clone().unwrap_or_default();
            cx.spawn_in(window, async move |this, cx| {
                for i in 0..STRIP_FRAMES {
                    let p = path.clone();
                    let at = dur * (i as f64 + 0.5) / STRIP_FRAMES as f64;
                    let img = cx.background_executor().spawn(async move { engine_media::frame_png(&p, at, 180, None).ok().and_then(|b| bgra_image(&b)) }).await;
                    let alive = this.update(cx, |s, cx| {
                        if s.path.as_deref() != Some(path.as_path()) {
                            return false;
                        }
                        if let Some(slot) = s.strip.get_mut(i) {
                            *slot = img;
                        }
                        cx.notify();
                        true
                    });
                    if !matches!(alive, Ok(true)) {
                        break;
                    }
                }
            })
            .detach();
            self.request_frame(cx);
        }
    }

    /// The frame at the playhead (debounced; the newest request wins).
    fn request_frame(&mut self, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        if !e.has_video {
            return;
        }
        self.frame_gen += 1;
        let generation = self.frame_gen;
        let (path, at) = (e.source.clone(), self.playhead.min((e.duration - 0.04).max(0.0)));
        self.frame_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(90)).await;
            let img = cx.background_executor().spawn(async move { engine_media::frame_png(&path, at, 960, None).ok().and_then(|b| bgra_image(&b)) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.frame_gen == generation {
                    if let Some(i) = img {
                        if let Some(old) = s.frame.replace(i) {
                            s.old_images.push(old);
                        }
                    }
                    cx.notify();
                }
            });
        }));
    }

    fn start_poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._poll = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                if this.update_in(cx, |s, window, cx| s.poll(window, cx)).is_err() {
                    break;
                }
            }
        }));
    }

    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for img in self.old_images.drain(..) {
            cx.drop_image(img, Some(window));
        }
        let Some(j) = self.job.as_mut() else { return };
        let Some(h) = j.handle.as_ref() else { return };
        let mut finished = None;
        let mut changed = false;
        while let Ok(ev) = h.events.try_recv() {
            changed = true;
            match ev {
                RunEvent::JobProgress { frac, .. } => j.frac = frac,
                RunEvent::JobMessage { msg, .. } => j.message = Some(msg),
                RunEvent::Finished(r) => finished = Some(*r),
                _ => {}
            }
        }
        if let Some(r) = finished {
            j.handle = None;
            j.record = Some(r.clone());
            if j.transcribe {
                self.after_transcribe(&r, window, cx);
            } else {
                model(cx).update(cx, |m, cx| m.push_run(r, cx));
            }
        }
        if changed {
            cx.notify();
        }
    }

    // ---- editing ------------------------------------------------------------------------

    fn change(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut MediaEdit)) {
        if let Some(e) = self.edit.as_mut() {
            self.undo.push(e.clone());
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            f(e);
            cx.notify();
        }
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if let Some(prev) = self.undo.pop() {
            self.edit = Some(prev);
            cx.notify();
        }
    }

    fn duration(&self) -> f64 {
        self.edit.as_ref().map(|e| e.duration).unwrap_or(0.0)
    }

    fn seek(&mut self, t: f64, cx: &mut Context<Self>) {
        self.playhead = t.clamp(0.0, self.duration());
        self.request_frame(cx);
        cx.notify();
    }

    fn set_in(&mut self, cx: &mut Context<Self>) {
        let (a, d) = (self.playhead, self.duration());
        let end = self.edit.as_ref().and_then(|e| e.keep.last().map(|k| k.1)).unwrap_or(d);
        if end - a > 0.1 {
            self.change(cx, |e| e.trim_to(a, end));
        }
    }

    fn set_out(&mut self, cx: &mut Context<Self>) {
        let b = self.playhead;
        let start = self.edit.as_ref().and_then(|e| e.keep.first().map(|k| k.0)).unwrap_or(0.0);
        if b - start > 0.1 {
            self.change(cx, |e| e.trim_to(start, b));
        }
    }

    fn cut_selection(&mut self, cx: &mut Context<Self>) {
        if let Some((a, b)) = self.sel {
            self.change(cx, |e| e.cut_range(a, b));
            self.sel = None;
        }
    }

    fn restore_selection(&mut self, cx: &mut Context<Self>) {
        if let Some((a, b)) = self.sel {
            self.change(cx, |e| e.restore_range(a, b));
        }
    }

    fn reset_cuts(&mut self, cx: &mut Context<Self>) {
        self.change(cx, |e| e.keep = vec![(0.0, e.duration)]);
    }

    fn find_pauses(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        let (path, dur, noise, min) = (e.source.clone(), e.duration, self.noise as f64, self.min_pause);
        self.finding = true;
        let task = cx.background_executor().spawn(async move {
            let dir = castor_core::paths::cache_dir().join("studio-tmp");
            let _ = std::fs::create_dir_all(&dir);
            let rcx = castor_core::RunCx::new(dir.clone(), dir, "", castor_core::CancelToken::new());
            engine_media::analyze::silences(&path, noise, min, dur, &rcx, (0.0, 1.0))
        });
        cx.spawn_in(window, async move |this, cx| {
            let res = task.await;
            let _ = this.update_in(cx, |s, window, cx| {
                s.finding = false;
                match res {
                    Ok(v) => s.silences = v,
                    Err(e) => window.push_notification(format!("{e:#}"), cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_pauses(&mut self, cx: &mut Context<Self>) {
        let pad = 0.12;
        let sil = self.silences.clone();
        self.change(cx, |e| {
            for (a, b) in sil {
                let (a2, b2) = (if a <= 0.0 { 0.0 } else { a + pad }, if b >= e.duration { e.duration } else { b - pad });
                if b2 - a2 > 0.05 {
                    e.cut_range(a2, b2);
                }
            }
        });
    }

    fn selected_words(&self) -> Option<(f64, f64)> {
        let e = self.edit.as_ref()?;
        let (a, b) = self.word_sel?;
        let (a, b) = (a.min(b), a.max(b));
        let (wa, wb) = (e.words.get(a)?, e.words.get(b)?);
        // Up to the next word, so the cut takes the pause after the last word too.
        let end = e.words.get(b + 1).map(|n| n.start).unwrap_or(wb.end).min(wb.end + 0.4);
        Some((wa.start, end))
    }

    fn cut_words(&mut self, cut: bool, cx: &mut Context<Self>) {
        if let Some((a, b)) = self.selected_words() {
            self.change(cx, |e| if cut { e.cut_range(a, b) } else { e.restore_range(a, b) });
        }
    }

    fn click_word(&mut self, i: usize, shift: bool, cx: &mut Context<Self>) {
        self.word_sel = Some(match (shift, self.word_sel) {
            (true, Some((a, _))) => (a, i),
            _ => (i, i),
        });
        if let Some(w) = self.edit.as_ref().and_then(|e| e.words.get(i)) {
            let t0 = w.start;
            self.seek(t0, cx);
        }
        cx.notify();
    }

    // ---- transcription and captions -----------------------------------------------------

    fn transcribe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        let reg = crate::model::registry(cx);
        let Some(tool) = reg.get("audio.transcribe") else {
            window.push_notification(t("app.media.err.no_transcribe").to_string(), cx);
            return;
        };
        if let Some(why) = tool.unavailable() {
            window.push_notification(why, cx);
            return;
        }
        let mut values = model(cx).read(cx).tool_values("audio.transcribe");
        values.set("format", Value::Text("json".into()));
        // Exact word timings for cutting by words and for captions.
        values.set("vad", Value::Bool(false));
        let input = FileInfo::from_path(&e.source);
        let missing: Vec<castor_core::download::Asset> = tool.downloads(&values).into_iter().filter(|a| !a.is_present()).collect();
        let this = cx.entity().downgrade();
        let start = move |cx: &mut App| {
            let _ = this.update(cx, |s, cx| {
                let dir = castor_core::paths::cache_dir().join("studio-transcripts").join(castor_core::history::new_run_id());
                let req = RunRequest {
                    steps: vec![Step { tool: tool.clone(), values: values.clone() }],
                    inputs: vec![input.clone()],
                    output: OutputPolicy::Folder(dir),
                    chain_name: None,
                    max_parallel: 1,
                    record: false,
                };
                s.job = Some(Job { handle: Some(runner::start(req)), frac: 0.0, message: None, record: None, transcribe: true });
                cx.notify();
            });
        };
        if missing.is_empty() {
            start(cx);
            return;
        }
        let total: u64 = missing.iter().map(|a| a.size).sum();
        let k = tokens(cx);
        let start = Rc::new(start);
        window.open_dialog(cx, move |dialog, _, _| {
            let start = start.clone();
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
                                .child(ui::sans().w(px(70.0)).text_right().child(units::bytes(a.size)))
                        }))
                        .child(ui::hrule(k.rule, 1.0))
                        .child(div().flex().flex_row().child(ui::sans().flex_1().font_weight(FontWeight::BOLD).child(t("app.consent.total"))).child(ui::sans().font_weight(FontWeight::BOLD).child(units::bytes(total))))
                        .child(ui::rule_strong(&k))
                        .child(ui::roman().text_size(px(13.5)).italic().text_color(k.muted).child(t("app.consent.once"))),
                )
                .footer(
                    DialogFooter::new()
                        .child(DialogClose::new().child(Button::new("tr-cancel").label(t("common.cancel"))))
                        .child(DialogAction::new().child(Button::new("tr-ok").primary().label(t("app.consent.ok")).on_click(move |_, _, cx| start(cx)))),
                )
        });
    }

    fn after_transcribe(&mut self, r: &RunRecord, window: &mut Window, cx: &mut Context<Self>) {
        let json = r.outputs.iter().find(|p| p.extension().is_some_and(|e| e == "json")).and_then(|p| std::fs::read_to_string(p).ok());
        match json.map(|j| eng::parse_transcript(&j)) {
            Some(Ok((words, cues))) => {
                let n = words.len();
                self.change(cx, |e| {
                    e.words = words;
                    if e.cues.is_empty() {
                        e.cues = cues;
                    }
                });
                window.push_notification(ta("app.media.transcribed", &[("count", &n.to_string())]).to_string(), cx);
            }
            Some(Err(e)) => window.push_notification(format!("{e:#}"), cx),
            None => {
                let err = r.errors.first().cloned().unwrap_or_else(|| t("app.status.failed").to_string());
                window.push_notification(err, cx);
            }
        }
        // The transcription is studio state, not a separate run: clear the job line.
        self.job = None;
    }

    fn import_subtitles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update_in(cx, |s, window, cx| match eng::read_subtitles(&p) {
                        Ok(cues) if !cues.is_empty() => s.change(cx, |e| e.cues = cues),
                        Ok(_) => window.push_notification(t("app.media.err.no_captions").to_string(), cx),
                        Err(e) => window.push_notification(format!("{e:#}"), cx),
                    });
                }
            }
        })
        .detach();
    }

    fn select_cue(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cue_sel = Some(i);
        if let Some(c) = self.edit.as_ref().and_then(|e| e.cues.get(i)).cloned() {
            self.cue_text.update(cx, |inp, cx| inp.set_value(c.text.replace('\n', " "), window, cx));
            self.seek(c.start, cx);
        }
    }

    fn nudge_cue(&mut self, start: bool, by: f64, cx: &mut Context<Self>) {
        let Some(i) = self.cue_sel else { return };
        self.change(cx, |e| {
            if let Some(c) = e.cues.get_mut(i) {
                if start {
                    c.start = (c.start + by).clamp(0.0, c.end - 0.1);
                } else {
                    c.end = (c.end + by).max(c.start + 0.1).min(e.duration);
                }
            }
        });
    }

    // ---- export -------------------------------------------------------------------------

    fn export(&mut self, what: Export, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        if self.job.as_ref().is_some_and(|j| j.handle.is_some()) {
            return;
        }
        let mut edit = e.clone();
        edit.export = what;
        let Ok(script) = serde_json::to_string(&edit) else { return };
        let reg = crate::model::registry(cx);
        let tool: Arc<dyn castor_core::Tool> = Arc::new(super::tools::MediaStudioTool::new(reg));
        let req = RunRequest {
            steps: vec![Step { tool, values: super::tools::values(script, false) }],
            inputs: vec![FileInfo::from_path(&edit.source)],
            output: OutputPolicy::NextToInput { fallback: castor_core::paths::default_output_dir() },
            chain_name: None,
            max_parallel: 1,
            record: true,
        };
        self.job = Some(Job { handle: Some(runner::start(req)), frac: 0.0, message: None, record: None, transcribe: false });
        cx.notify();
    }

    fn cancel_job(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.job.as_ref().and_then(|j| j.handle.as_ref()) {
            h.cancel();
        }
        cx.notify();
    }

    fn play(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.edit else { return };
        let (a, b) = self.sel.unwrap_or((self.playhead, (self.playhead + 10.0).min(e.duration)));
        let path = e.source.clone();
        let task = cx.background_executor().spawn(async move { eng::play_segment(&path, a, b) });
        cx.spawn_in(window, async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update_in(cx, |_, window, cx| window.push_notification(format!("{e:#}"), cx));
            }
        })
        .detach();
    }

    // ---- rendering ----------------------------------------------------------------------

    fn render_empty(&mut self, k: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let tint = Hsla { a: 0.08, ..k.link };
        let link = k.link;
        div()
            .id("media-empty")
            .size_full()
            .px(px(crate::home::PAGE_PAD))
            .pb(px(32.0))
            .flex()
            .flex_col()
            .gap(px(18.0))
            .child(super::crumbs(k, t("app.studio.media.name"), None))
            .child(ui::page_header(cx, t("app.studio.media.name"), None, Some(t("app.studio.media.desc").into_any_element())))
            .child(
                div()
                    .id("media-drop")
                    .flex_1()
                    .min_h(px(220.0))
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
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        if let Some(p) = paths.paths().first() {
                            this.open(p.clone(), window, cx);
                        }
                    }))
                    .child(ui::icon("film", k.muted).size(px(28.0)))
                    .child(ui::roman().text_color(k.ink).child(t("app.studio.media.drop")))
                    .child(Button::new("media-open").primary().label(format!("{} ({}O)", t("app.studio.open"), ui::cmd_key())).on_click(cx.listener(|s, _, w, cx| s.choose_file(w, cx))))
                    .when_some(self.error.clone(), |d, e| d.child(ui::sans().text_color(k.bad).max_w(px(520.0)).child(e))),
            )
            .into_any_element()
    }

    fn render_toolbar(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.path.as_ref().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let this = cx.entity().downgrade();
        div()
            .h(px(44.0))
            .flex_none()
            .px(px(14.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(12.0))
            .border_b_1()
            .border_color(k.rule)
            .child(div().flex_1().min_w(px(0.0)).child(super::crumbs(k, t("app.studio.media.name"), Some(name))))
            .child(ui::segmented(
                "media-tab",
                k,
                Tab::ALL.iter().map(|x| (x.key().to_string(), t(&format!("app.media.tab.{}", x.key())))).collect(),
                self.tab.key(),
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this.update(cx, |s, cx| s.set_tab_key(&v, cx));
                }),
            ))
    }

    fn render_preview(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = &self.edit else { return div() };
        let kept = e.is_kept(self.playhead);
        let body: AnyElement = if e.has_video {
            match &self.frame {
                Some(f) => img(f.clone()).size_full().object_fit(ObjectFit::Contain).into_any_element(),
                None => ui::roman().italic().text_color(k.faint).child(t("app.studio.loading")).into_any_element(),
            }
        } else {
            div().flex().flex_col().items_center().gap(px(8.0)).child(ui::icon("music", k.muted).size(px(40.0))).child(ui::roman().text_color(k.muted).child(t("app.media.audio_only"))).into_any_element()
        };
        // The caption under the playhead, previewed over the frame (Captions tab).
        let cue_now = (self.tab == Tab::Captions && e.has_video)
            .then(|| e.cues.iter().find(|c| self.playhead >= c.start && self.playhead < c.end))
            .flatten()
            .map(|c| (c.text.clone(), e.position == "top"));
        let caption = ta(
            "app.media.at",
            &[("time", &tc(self.playhead)), ("total", &tc(e.duration))],
        );
        div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .p(px(16.0))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(if e.has_video { gpui_kit::black() } else { k.raised })
                    .border_1()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .overflow_hidden()
                    .child(body)
                    .when_some(cue_now, |d, (text, top)| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .right_0()
                                .when(top, |d| d.top(px(18.0)))
                                .when(!top, |d| d.bottom(px(18.0)))
                                .flex()
                                .justify_center()
                                .child(
                                    div()
                                        .max_w(px(560.0))
                                        .px(px(10.0))
                                        .py(px(4.0))
                                        .rounded(px(3.0))
                                        .bg(Hsla { a: 0.62, ..gpui_kit::black() })
                                        .font_family(crate::theme::fonts().sans.clone())
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(19.0))
                                        .text_center()
                                        .text_color(gpui_kit::white())
                                        .child(text),
                                ),
                        )
                    })
                    .when(!kept, |d| {
                        d.child(div().absolute().top_0().left_0().size_full().bg(Hsla { a: 0.45, ..k.paper }).flex().items_center().justify_center().child(ui::sans().font_weight(FontWeight::BOLD).text_color(k.bad).child(t("app.media.cut_here"))))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(ui::caption(k, format!("{} 1", castor_core::tr("app.caption.figure")), "/", caption))
                    .child(div().flex_1())
                    .child(Button::new("play").small().icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("play"))).label(if self.sel.is_some() { t("app.media.play_sel") } else { t("app.media.play") }).on_click(cx.listener(|s, _, w, cx| s.play(w, cx)))),
            )
    }

    fn render_panel(&mut self, k: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body: AnyElement = match self.tab {
            Tab::Cut => self.panel_cut(k, window, cx).into_any_element(),
            Tab::Transcript => self.panel_transcript(k, cx).into_any_element(),
            Tab::Captions => self.panel_captions(k, cx).into_any_element(),
        };
        div()
            .id("media-panel")
            .w(px(PANEL_W))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .border_l_1()
            .border_color(k.rule)
            .child(div().p(px(16.0)).flex().flex_col().gap(px(14.0)).child(body))
    }

    fn panel_cut(&mut self, k: &Tokens, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.edit.clone() else { return div() };
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        let sel = self.sel;
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.media.trim")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.media.trim.help")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("set-in").small().label(t("app.media.set_in")).tooltip("I").on_click(cx.listener(|s, _, _, cx| s.set_in(cx))))
                    .child(Button::new("set-out").small().label(t("app.media.set_out")).tooltip("O").on_click(cx.listener(|s, _, _, cx| s.set_out(cx)))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("cut-sel").small().danger().label(t("app.media.cut_sel")).disabled(sel.is_none()).on_click(cx.listener(|s, _, _, cx| s.cut_selection(cx))))
                    .child(Button::new("restore-sel").small().label(t("app.media.restore_sel")).disabled(sel.is_none()).on_click(cx.listener(|s, _, _, cx| s.restore_selection(cx)))),
            )
            .when_some(sel, |d, (a, b)| d.child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.media.selection", &[("from", &tc(a)), ("to", &tc(b)), ("len", &units::duration(b - a))]))))
            .child(ui::caption(k, format!("{} 1", castor_core::tr("app.caption.table")), "—", t("app.media.kept")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(ui::rule_strong(k))
                    .children(e.keep.iter().enumerate().map(|(i, (a, b))| {
                        let a = *a;
                        div()
                            .id(("seg", i))
                            .h(px(28.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(k.surface))
                            .on_click(cx.listener(move |s, _, _, cx| s.seek(a, cx)))
                            .child(ui::sans().w(px(20.0)).text_color(k.muted).child(format!("{}", i + 1)))
                            .child(ui::sans().flex_1().text_color(k.ink).child(format!("{} → {}", tc(a), tc(*b))))
                            .child(ui::sans().text_color(k.muted).child(units::duration(b - a)))
                    }))
                    .child(ui::rule_strong(k)),
            )
            .when(e.has_cuts(), |d| d.child(Button::new("reset-cuts").small().ghost().label(t("app.media.reset")).on_click(cx.listener(|s, _, _, cx| s.reset_cuts(cx)))))
            .child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.media.cut_mode"))))
            .child(ui::segmented(
                "cut-mode",
                k,
                vec![("exact".into(), t("app.media.exact")), ("fast".into(), t("app.media.fast"))],
                &e.cut,
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this.update(cx, |s, cx| s.change(cx, |e| e.cut = v));
                }),
            ))
            .child(ui::roman().text_size(px(13.0)).text_color(k.muted).child(if e.cut == "fast" { t("app.media.fast.help") } else { t("app.media.exact.help") }))
            .child(ui::hrule(k.rule, 1.0))
            .child(ui::sub_title(k, 2, t("app.media.pauses")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.media.pauses.help")))
            .child(ui::segmented(
                "pause-level",
                k,
                vec![("-45".into(), t("app.media.pauses.quiet")), ("-35".into(), t("app.media.pauses.normal")), ("-25".into(), t("app.media.pauses.noisy"))],
                &self.noise.to_string(),
                Rc::new(move |v, _, cx| {
                    let n: i64 = v.parse().unwrap_or(-35);
                    let _ = this2.update(cx, |s, cx| {
                        s.noise = n;
                        cx.notify();
                    });
                }),
            ))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("find-pauses").small().label(if self.finding { t("app.media.pauses.finding") } else { t("app.media.pauses.find") }).disabled(self.finding).on_click(cx.listener(|s, _, w, cx| s.find_pauses(w, cx))))
                    .when(!self.silences.is_empty(), |d| d.child(Button::new("rm-pauses").small().danger().label(ta("app.media.pauses.remove", &[("count", &self.silences.len().to_string())])).on_click(cx.listener(|s, _, _, cx| s.remove_pauses(cx))))),
            )
            .when(!self.silences.is_empty(), |d| {
                let total: f64 = self.silences.iter().map(|(a, b)| b - a).sum();
                d.child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.media.pauses.found", &[("count", &self.silences.len().to_string()), ("total", &units::duration(total))])))
            })
    }

    fn panel_transcript(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.edit.clone() else { return div() };
        let busy = self.job.as_ref().is_some_and(|j| j.handle.is_some() && j.transcribe);
        let sel = self.word_sel.map(|(a, b)| (a.min(b), a.max(b)));
        let playing = e.words.iter().rposition(|w| w.start <= self.playhead + 0.01);
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.media.tab.transcript")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.media.transcript.help")));
        if e.words.is_empty() {
            col = col.child(
                Button::new("transcribe")
                    .primary()
                    .small()
                    .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("audio-lines")))
                    .label(if busy { t("app.media.transcribing") } else { t("app.media.transcribe") })
                    .disabled(busy)
                    .on_click(cx.listener(|s, _, w, cx| s.transcribe(w, cx))),
            );
            if busy {
                let (f, m) = self.job.as_ref().map(|j| (j.frac, j.message.clone())).unwrap_or((0.0, None));
                col = col.child(Progress::new("tr-progress").value(f * 100.0)).when_some(m, |d, m| d.child(ui::sans().text_size(px(12.5)).text_color(k.muted).child(m)));
            }
            return col;
        }
        col.child(
            div()
                .flex()
                .flex_row()
                .gap(px(6.0))
                .child(Button::new("cut-words").small().danger().label(t("app.media.cut_words")).disabled(sel.is_none()).on_click(cx.listener(|s, _, _, cx| s.cut_words(true, cx))))
                .child(Button::new("restore-words").small().label(t("app.media.restore_words")).disabled(sel.is_none()).on_click(cx.listener(|s, _, _, cx| s.cut_words(false, cx)))),
        )
        .child(
            div().flex().flex_row().flex_wrap().gap_x(px(4.0)).gap_y(px(2.0)).children(e.words.iter().enumerate().map(|(i, w)| {
                let cut = e.word_cut(w);
                let in_sel = sel.is_some_and(|(a, b)| i >= a && i <= b);
                ui::roman()
                    .id(("word", i))
                    .text_size(px(15.0))
                    .px(px(1.0))
                    .rounded(px(2.0))
                    .cursor_pointer()
                    .text_color(if cut { k.faint } else { k.ink })
                    .when(cut, |d| d.line_through())
                    .when(in_sel, |d| d.bg(Hsla { a: 0.22, ..k.link }))
                    .when(playing == Some(i) && !in_sel, |d| d.underline())
                    .on_click(cx.listener(move |s, ev: &ClickEvent, _, cx| s.click_word(i, ev.modifiers().shift, cx)))
                    .child(w.text.clone())
            })),
        )
    }

    fn panel_captions(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.edit.clone() else { return div() };
        let this = cx.entity().downgrade();
        let this2 = this.clone();
        let mapped = e.mapped_cues().len();
        let mut col = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::sub_title(k, 1, t("app.media.tab.captions")))
            .child(ui::roman().text_size(px(14.0)).text_color(k.muted).child(t("app.media.captions.help")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("cap-from-tr").small().label(if e.words.is_empty() { t("app.media.transcribe") } else { t("app.media.captions.from_transcript") }).on_click(cx.listener(|s, _, w, cx| {
                        if s.edit.as_ref().is_some_and(|e| e.words.is_empty()) {
                            s.tab = Tab::Transcript;
                            s.transcribe(w, cx);
                        } else {
                            s.tab = Tab::Transcript;
                            cx.notify();
                        }
                    })))
                    .child(Button::new("cap-import").small().label(t("app.media.captions.import")).on_click(cx.listener(|s, _, w, cx| s.import_subtitles(w, cx)))),
            );
        if e.cues.is_empty() {
            return col.child(ui::roman().italic().text_color(k.faint).child(t("app.media.captions.none")));
        }
        if let Some(i) = self.cue_sel.filter(|i| *i < e.cues.len()) {
            let c = &e.cues[i];
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .p(px(10.0))
                    .border_1()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .bg(k.raised)
                    .child(Input::new(&self.cue_text))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .child(Button::new("cs-").xsmall().label("−").on_click(cx.listener(|s, _, _, cx| s.nudge_cue(true, -0.1, cx))))
                            .child(ui::sans().text_size(px(13.0)).child(tc(c.start)))
                            .child(Button::new("cs+").xsmall().label("+").on_click(cx.listener(|s, _, _, cx| s.nudge_cue(true, 0.1, cx))))
                            .child(ui::sans().text_color(k.faint).child("→"))
                            .child(Button::new("ce-").xsmall().label("−").on_click(cx.listener(|s, _, _, cx| s.nudge_cue(false, -0.1, cx))))
                            .child(ui::sans().text_size(px(13.0)).child(tc(c.end)))
                            .child(Button::new("ce+").xsmall().label("+").on_click(cx.listener(|s, _, _, cx| s.nudge_cue(false, 0.1, cx)))),
                    ),
            );
        }
        col = col.child(
            div().flex().flex_col().child(ui::rule_strong(k)).children(e.cues.iter().enumerate().take(400).map(|(i, c)| {
                let sel = self.cue_sel == Some(i);
                let gone = e.map_time(c.start).is_none() && e.map_time(c.end).is_none();
                div()
                    .id(("cue", i))
                    .py(px(4.0))
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .cursor_pointer()
                    .when(sel, |d| d.bg(k.surface))
                    .hover(|s| s.bg(k.surface))
                    .on_click(cx.listener(move |s, _, w, cx| s.select_cue(i, w, cx)))
                    .child(ui::mono().w(px(64.0)).flex_none().text_size(px(12.5)).text_color(k.muted).child(tc(c.start)))
                    .child(ui::sans().flex_1().min_w(px(0.0)).text_size(px(13.5)).text_color(if gone { k.faint } else { k.ink }).when(gone, |d| d.line_through()).child(c.text.replace('\n', " ")))
            }))
            .child(ui::rule_strong(k)),
        );
        col.child(ui::caps().text_color(k.muted).child(ui::small_caps(&t("app.media.style"))))
            .child(ui::segmented(
                "cap-style",
                k,
                ["classic", "box", "karaoke", "minimal"].iter().map(|s| (s.to_string(), SharedString::from(castor_core::tr(&format!("video.captions.opt.style.{s}"))))).collect(),
                &e.style,
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this.update(cx, |s, cx| s.change(cx, |e| e.style = v));
                }),
            ))
            .child(ui::segmented(
                "cap-pos",
                k,
                ["bottom", "middle", "top"].iter().map(|s| (s.to_string(), SharedString::from(castor_core::tr(&format!("video.captions.opt.position.{s}"))))).collect(),
                &e.position,
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this2.update(cx, |s, cx| s.change(cx, |e| e.position = v));
                }),
            ))
            .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.media.captions.count", &[("count", &mapped.to_string())])))
    }

    fn render_timeline(&mut self, k: &Tokens, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.edit.clone() else { return div().id("timeline") };
        let avail = (window.viewport_size().width.as_f32() - crate::shell::SIDEBAR_W - 32.0).max(200.0);
        let width = avail * self.zoom;
        let pps = width / e.duration.max(0.01) as f32;
        let x = |t: f64| (t as f32) * pps;
        let origin = self.timeline_origin.clone();
        let wave = self.wave.clone();
        let strip = self.strip.clone();
        let (vw, vh) = self.size;
        let frame_w = (40.0 * vw as f32 / vh.max(1) as f32).clamp(24.0, 120.0);
        let mut content = div()
            .relative()
            .w(px(width))
            .h(px(TIMELINE_H - 26.0))
            .child(canvas(move |b, _, _| origin.set((b.origin.x.as_f32(), b.origin.y.as_f32())), |_, _, _, _| {}).absolute().size_full());
        // Ruler.
        let step = [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0].iter().copied().find(|s| (*s as f32) * pps >= 80.0).unwrap_or(3600.0);
        let mut t0 = 0.0;
        while t0 <= e.duration {
            content = content.child(div().absolute().left(px(x(t0))).top_0().h(px(16.0)).border_l_1().border_color(k.rule).pl(px(3.0)).child(ui::sans().text_size(px(11.5)).text_color(k.muted).child(tc(t0))));
            t0 += step;
        }
        // Filmstrip.
        if e.has_video {
            let n = ((width / frame_w).ceil() as usize).max(1);
            let mut row = div().absolute().top(px(18.0)).left_0().w(px(width)).h(px(44.0)).flex().flex_row().overflow_hidden().bg(gpui_kit::black());
            for i in 0..n {
                let t_mid = (i as f64 + 0.5) / n as f64;
                let slot = ((t_mid * STRIP_FRAMES as f64) as usize).min(STRIP_FRAMES.saturating_sub(1));
                let im = strip.get(slot).cloned().flatten();
                row = row.child(div().w(px(width / n as f32)).h_full().flex_none().overflow_hidden().border_r_1().border_color(gpui_kit::black()).when_some(im, |d, im| d.child(img(im).size_full().object_fit(ObjectFit::Cover))));
            }
            content = content.child(row);
        }
        // Waveform.
        let wave_top = if e.has_video { 66.0 } else { 18.0 };
        let wave_h = TIMELINE_H - 26.0 - wave_top - 4.0;
        let ink = k.muted;
        content = content.child(
            canvas(
                |_, _, _| {},
                move |b, _, window, _| {
                    if wave.is_empty() {
                        return;
                    }
                    let n = wave.len();
                    let cols = (b.size.width.as_f32() / 2.0).max(1.0) as usize;
                    let mid = b.origin.y.as_f32() + b.size.height.as_f32() / 2.0;
                    let half = b.size.height.as_f32() / 2.0;
                    for c in 0..cols {
                        let (a, z) = (c * n / cols, ((c + 1) * n / cols).max(c * n / cols + 1).min(n));
                        let (mut lo, mut hi) = (0.0f32, 0.0f32);
                        for (mn, mx) in &wave[a..z] {
                            lo = lo.min(*mn);
                            hi = hi.max(*mx);
                        }
                        let y0 = mid - hi.clamp(0.0, 1.0) * half;
                        let y1 = mid - lo.clamp(-1.0, 0.0) * half;
                        let xx = b.origin.x.as_f32() + c as f32 * 2.0;
                        window.paint_quad(fill(Bounds { origin: point(px(xx), px(y0)), size: size(px(1.4), px((y1 - y0).max(1.0))) }, ink));
                    }
                },
            )
            .absolute()
            .left_0()
            .top(px(wave_top))
            .w(px(width))
            .h(px(wave_h)),
        );
        // Pauses found, cut ranges, selection, playhead.
        for (a, b) in &self.silences {
            content = content.child(div().absolute().top(px(wave_top)).h(px(wave_h)).left(px(x(*a))).w(px((x(*b) - x(*a)).max(1.0))).bg(Hsla { a: 0.22, ..k.warn }));
        }
        let mut t_prev = 0.0;
        let mut cuts = Vec::new();
        for (a, b) in &e.keep {
            if *a - t_prev > 0.005 {
                cuts.push((t_prev, *a));
            }
            t_prev = *b;
        }
        if e.duration - t_prev > 0.005 {
            cuts.push((t_prev, e.duration));
        }
        for (a, b) in cuts {
            content = content.child(
                div()
                    .absolute()
                    .top(px(16.0))
                    .bottom_0()
                    .left(px(x(a)))
                    .w(px((x(b) - x(a)).max(1.0)))
                    .bg(Hsla { a: 0.62, ..k.paper })
                    .border_t_2()
                    .border_color(k.bad),
            );
        }
        if let Some((a, b)) = self.sel {
            content = content.child(div().absolute().top(px(16.0)).bottom_0().left(px(x(a))).w(px((x(b) - x(a)).max(1.0))).bg(Hsla { a: 0.18, ..k.link }).border_l_1().border_r_1().border_color(k.link));
        }
        let ph = x(self.playhead);
        content = content
            .child(div().absolute().top(px(10.0)).bottom_0().left(px(ph - 1.0)).w(px(2.0)).bg(k.ink))
            .child(ui::diamond_filled(10.0, k.ink).absolute().top(px(4.0)).left(px(ph - 5.0)));
        div()
            .id("timeline")
            .flex_none()
            .h(px(TIMELINE_H))
            .w_full()
            .px(px(16.0))
            .pt(px(8.0))
            .border_t_1()
            .border_color(k.rule)
            .bg(k.surface)
            .child(
                div()
                    .id("timeline-scroll")
                    .role(Role::Slider)
                    .aria_label(t("app.media.timeline_aria"))
                    .w_full()
                    .h_full()
                    .overflow_x_scroll()
                    .track_scroll(&self.timeline_scroll)
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(move |s, ev: &MouseDownEvent, _, cx| {
                        let t = s.time_at(ev.position, pps);
                        s.dragging = Some(t);
                        s.sel = None;
                        s.seek(t, cx);
                    }))
                    .on_mouse_move(cx.listener(move |s, ev: &MouseMoveEvent, _, cx| {
                        if let (Some(a), Some(MouseButton::Left)) = (s.dragging, ev.pressed_button) {
                            let b = s.time_at(ev.position, pps);
                            if (b - a).abs() > 0.05 {
                                s.sel = Some((a.min(b), a.max(b)));
                                s.seek(b, cx);
                            }
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| s.dragging = None))
                    .child(content),
            )
    }

    fn time_at(&self, pos: Point<Pixels>, pps: f32) -> f64 {
        let (ox, _) = self.timeline_origin.get();
        (((pos.x.as_f32() - ox) / pps.max(0.0001)) as f64).clamp(0.0, self.duration())
    }

    fn render_bottom(&mut self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(e) = self.edit.clone() else { return div() };
        let running = self.job.as_ref().filter(|j| j.handle.is_some() && !j.transcribe).map(|j| (j.frac, j.message.clone()));
        let done = self.job.as_ref().filter(|j| !j.transcribe).and_then(|j| j.record.clone());
        let busy = self.job.as_ref().is_some_and(|j| j.handle.is_some());
        let cuts = e.keep.len().saturating_sub(1) + usize::from(e.keep.first().is_some_and(|k| k.0 > 0.01)) + usize::from(e.keep.last().is_some_and(|k| k.1 < e.duration - 0.01));
        let has_caps = !e.mapped_cues().is_empty();
        div()
            .h(px(52.0))
            .flex_none()
            .px(px(16.0))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(10.0))
            .border_t_1()
            .border_color(k.rule)
            .child(ui::sans().text_color(k.ink).whitespace_nowrap().child(if e.has_cuts() {
                ta(if cuts == 1 { "app.media.status_cut.one" } else { "app.media.status_cut" }, &[("kept", &units::duration(e.kept())), ("total", &units::duration(e.duration)), ("count", &cuts.to_string())])
            } else {
                ta("app.media.status", &[("total", &units::duration(e.duration))])
            }))
            .child(div().flex_1().min_w(px(0.0)).flex().flex_row().items_center().gap(px(10.0)).map(|d| {
                if let Some((f, m)) = running {
                    d.child(div().w(px(150.0)).child(Progress::new("export-progress").value(f * 100.0)))
                        .child(ui::sans().text_size(px(13.0)).text_color(k.muted).truncate().child(m.unwrap_or_else(|| t("app.media.exporting").to_string())))
                } else if let Some(r) = done {
                    super::run_summary(k, &r, d)
                } else {
                    d
                }
            }))
            .child(Button::new("media-undo").ghost().small().tooltip(format!("{} ({}Z)", t("app.pdf.undo"), ui::cmd_key())).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("undo-2"))).disabled(self.undo.is_empty()).on_click(cx.listener(|s, _, _, cx| s.undo(cx))))
            .when(busy, |d| d.child(Button::new("media-cancel").label(t("common.cancel")).on_click(cx.listener(|s, _, _, cx| s.cancel_job(cx)))))
            .child(Button::new("export-srt").label(t("app.media.export_srt")).disabled(!has_caps || busy).on_click(cx.listener(|s, _, _, cx| s.export(Export::Srt, cx))))
            .when(e.has_video, |d| d.child(Button::new("export-burn").label(t("app.media.export_burn")).disabled(!has_caps || busy).on_click(cx.listener(|s, _, _, cx| s.export(Export::Burn, cx)))))
            .child(
                Button::new("export-media")
                    .primary()
                    .label(if e.has_video { t("app.media.export_video") } else { t("app.media.export_audio") })
                    .disabled(!e.has_cuts() || busy)
                    .on_click(cx.listener(|s, _, _, cx| s.export(Export::Media, cx))),
            )
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let ks = &ev.keystroke;
        let cmd = ks.modifiers.control || ks.modifiers.platform;
        let step = if ks.modifiers.shift { 1.0 } else { 0.1 };
        match ks.key.as_str() {
            "o" if cmd => self.choose_file(window, cx),
            "z" if cmd => self.undo(cx),
            _ if !self.focus.is_focused(window) => return,
            "left" => self.seek(self.playhead - step, cx),
            "right" => self.seek(self.playhead + step, cx),
            "home" => self.seek(0.0, cx),
            "end" => self.seek(self.duration(), cx),
            "i" => self.set_in(cx),
            "o" => self.set_out(cx),
            "delete" | "backspace" => {
                if self.tab == Tab::Transcript && self.word_sel.is_some() {
                    self.cut_words(true, cx)
                } else {
                    self.cut_selection(cx)
                }
            }
            "space" => self.play(window, cx),
            "=" | "+" => {
                self.zoom = (self.zoom * 1.5).min(40.0);
                cx.notify();
            }
            "-" => {
                self.zoom = (self.zoom / 1.5).max(1.0);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}

impl Focusable for MediaStudio {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MediaStudio {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let body: AnyElement = if self.edit.is_none() && !self.loading {
            self.render_empty(&k, cx)
        } else if self.loading {
            div().size_full().flex().items_center().justify_center().child(ui::roman().italic().text_color(k.faint).child(t("app.studio.loading"))).into_any_element()
        } else {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(self.render_toolbar(&k, cx))
                .child(div().flex_1().min_h(px(0.0)).flex().flex_row().child(self.render_preview(&k, cx)).child(self.render_panel(&k, window, cx)))
                .child(self.render_timeline(&k, window, cx))
                .child(self.render_bottom(&k, cx))
                .into_any_element()
        };
        div()
            .id("media-studio")
            .key_context("MediaStudio")
            .track_focus(&self.focus)
            .size_full()
            .on_key_down(cx.listener(|s, ev: &KeyDownEvent, w, cx| s.on_key(ev, w, cx)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(p) = paths.paths().first() {
                    this.open(p.clone(), window, cx);
                }
            }))
            .child(body)
    }
}

#[allow(dead_code)]
fn _types(_: Word, _: Cue) {}
