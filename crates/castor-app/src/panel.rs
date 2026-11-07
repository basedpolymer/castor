//! Quick panel (DESIGN §5): a 360 × 520 borderless window under the menu bar icon (macOS) or
//! above the tray (Windows). Search, drop files and run a tool right away, clipboard actions,
//! favorites, last runs.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use castor_core::history::RunStatus;
use castor_core::runner::{OutputPolicy, RunEvent, RunRequest, Step};
use castor_core::{FileInfo, FileKind, Tool};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::{Nav, model, navigate};
use crate::theme::{fonts, tokens};
use crate::ui::{self, t, ta};

/// Tray icon rectangle in physical pixels: (x, y, w, h).
pub type Anchor = Option<(f64, f64, f64, f64)>;

const W: f32 = 360.0;
const H: f32 = 520.0;

/// Screenshot mode: the panel stays open when it loses focus.
pub struct KeepOpen;
impl Global for KeepOpen {}

#[derive(Default)]
struct PanelWindow {
    handle: Option<AnyWindowHandle>,
    /// When the panel was last opened: it does not close on focus loss while it settles.
    opened_at: Option<Instant>,
    /// Last toggle from the shortcut or the icon: repeats closer than [`SETTLE`] are ignored.
    toggled_at: Option<Instant>,
}
impl Global for PanelWindow {}

/// Time for the panel to appear (macOS orders it in with an animation). Removing it during that
/// time is the likely cause of the 0.9.0 crash on ⌥⌘Space (an AppKit exception in the Core
/// Animation commit): that default shortcut is also the Finder's "search window" shortcut, so
/// another app can take the focus while the panel appears, and the panel used to close at once.
const SETTLE: Duration = Duration::from_millis(450);

fn settling(cx: &App) -> bool {
    cx.try_global::<PanelWindow>().and_then(|p| p.opened_at).is_some_and(|t| t.elapsed() < SETTLE)
}

fn panel_bounds(anchor: Anchor, cx: &App) -> Bounds<Pixels> {
    let size = size(px(W), px(H));
    let Some(display) = cx.primary_display() else { return Bounds::centered(None, size, cx) };
    let vb = display.visible_bounds();
    let full = display.bounds();
    let margin = px(10.0);
    if cfg!(target_os = "macos") {
        // Under the icon. tray-icon reports physical pixels: guess the scale from the display.
        let x = match anchor {
            Some((x, _, w, _)) => {
                let scale = if x > f32::from(full.size.width) as f64 * 1.05 { 2.0 } else { 1.0 };
                px(((x + w / 2.0) / scale) as f32) - px(W / 2.0)
            }
            None => full.origin.x + full.size.width - px(W) - margin,
        };
        let x = x.max(full.origin.x + margin).min(full.origin.x + full.size.width - px(W) - margin);
        Bounds { origin: point(x, vb.origin.y + px(4.0)), size }
    } else if anchor.is_some() || cfg!(windows) {
        // Above the tray, at the bottom-right of the work area.
        Bounds {
            origin: point(vb.origin.x + vb.size.width - px(W) - margin, vb.origin.y + vb.size.height - px(H) - margin),
            size,
        }
    } else {
        Bounds::centered(None, size, cx)
    }
}

pub fn handle(cx: &App) -> Option<AnyWindowHandle> {
    cx.try_global::<PanelWindow>().and_then(|p| p.handle)
}

/// The panel window exists (a handle whose window is gone, closed by the system, counts as closed).
pub fn is_open(cx: &App) -> bool {
    handle(cx).is_some_and(|h| cx.windows().iter().any(|w| w.window_id() == h.window_id()))
}

pub fn close(cx: &mut App) {
    if !cx.has_global::<PanelWindow>() {
        return;
    }
    if let Some(h) = cx.global_mut::<PanelWindow>().handle.take() {
        // Never remove the window from inside an event or an activation callback of its own:
        // on the next turn of the loop, when AppKit is done with it.
        cx.defer(move |cx| {
            let _ = h.update(cx, |_, window, _| window.remove_window());
        });
    }
}

pub fn toggle(anchor: Anchor, cx: &mut App) {
    if !cx.has_global::<PanelWindow>() {
        cx.set_global(PanelWindow::default());
    }
    let now = Instant::now();
    let p = cx.global_mut::<PanelWindow>();
    // A held or repeated shortcut (and the tray icon's down/up pair) toggles once.
    if p.toggled_at.is_some_and(|t| now.duration_since(t) < SETTLE) {
        return;
    }
    p.toggled_at = Some(now);
    if is_open(cx) {
        close(cx);
    } else {
        open_panel(anchor, cx);
    }
}

pub fn open_panel(anchor: Anchor, cx: &mut App) {
    if !cx.has_global::<PanelWindow>() {
        cx.set_global(PanelWindow::default());
    }
    if is_open(cx) {
        return;
    }
    let bounds = panel_bounds(anchor, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        kind: WindowKind::PopUp,
        is_resizable: false,
        is_minimizable: false,
        focus: true,
        show: true,
        app_id: Some("eu.basedpolymer.castor".into()),
        ..Default::default()
    };
    // A stale handle (window closed by the system) must not block reopening.
    cx.global_mut::<PanelWindow>().handle = None;
    match gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| QuickPanel::new(window, cx))) {
        Ok((h, _)) => {
            let p = cx.global_mut::<PanelWindow>();
            p.handle = Some(h);
            p.opened_at = Some(Instant::now());
            let _ = h.update(cx, |_, window, _| window.activate_window());
        }
        Err(e) => tracing::warn!("could not open the quick panel: {e:#}"),
    }
}

#[derive(Clone)]
enum Clip {
    Image { png: Vec<u8>, w: u32, h: u32 },
    Files(Vec<PathBuf>),
}

struct QuickRun {
    tool: String,
    frac: f32,
    done: Option<(RunStatus, Vec<PathBuf>, String)>,
    to_clipboard: bool,
}

pub struct QuickPanel {
    input: Entity<InputState>,
    query: String,
    files: Vec<FileInfo>,
    clip: Option<Clip>,
    run: Option<QuickRun>,
    sel: usize,
    _subs: Vec<Subscription>,
}

impl QuickPanel {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.panel.search")));
        input.update(cx, |i, cx| i.focus(window, cx));
        let mut subs = vec![cx.subscribe(&input, |this, st, ev: &InputEvent, cx| match ev {
            InputEvent::Change => {
                this.query = st.read(cx).value().to_string();
                this.sel = 0;
                cx.notify();
            }
            InputEvent::PressEnter { .. } => this.open_selected(cx),
            _ => {}
        })];
        // Close when the panel loses focus (click elsewhere), except while it is appearing:
        // another app that grabs the focus then (the Finder answers ⌥⌘Space too) gets it back
        // to the panel once it has settled, instead of the panel being removed mid-animation.
        subs.push(cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() || cx.has_global::<KeepOpen>() {
                return;
            }
            if settling(cx) {
                let h = window.window_handle();
                cx.spawn(async move |_, cx| {
                    cx.background_executor().timer(SETTLE).await;
                    let _ = h.update(cx, |_, window, _| {
                        if !window.is_window_active() {
                            window.activate_window();
                        }
                    });
                })
                .detach();
                return;
            }
            cx.defer(|cx| close(cx));
        }));
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        subs.push(cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let key = ev.keystroke.key.as_str();
            match key {
                "escape" => {
                    cx.stop_propagation();
                    cx.defer(|cx| close(cx));
                }
                "up" | "down" => {
                    cx.stop_propagation();
                    let _ = weak.update(cx, |p, cx| {
                        p.sel = if key == "up" { p.sel.saturating_sub(1) } else { p.sel + 1 };
                        cx.notify();
                    });
                }
                _ => {}
            }
        }));
        let clip = read_clipboard(cx);
        QuickPanel { input, query: String::new(), files: Vec::new(), clip, run: None, sel: 0, _subs: subs }
    }

    fn results(&self, cx: &App) -> Vec<Arc<dyn Tool>> {
        let m = model(cx).read(cx);
        crate::search::tools(&m.registry, &self.query, &m.settings.favorites, &m.recent_tools(8)).into_iter().take(6).map(|(t, _)| t).collect()
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let res = self.results(cx);
        if let Some(t) = res.get(self.sel.min(res.len().saturating_sub(1))) {
            let id = t.meta().id.to_string();
            close(cx);
            navigate(Nav::Tool { id, files: vec![], values: None }, cx);
        }
    }

    /// Run a tool now on `inputs` with its last-used values.
    fn run_tool(&mut self, id: &str, inputs: Vec<FileInfo>, to_clipboard: bool, out: Option<PathBuf>, cx: &mut Context<Self>) {
        let m = model(cx).read(cx);
        let Some(tool) = m.registry.get(id) else { return };
        if tool.unavailable().is_some() || !tool.downloads(&m.tool_values(id)).iter().all(|a| a.is_present()) {
            // Needs a consent or a missing helper: hand over to the full tool screen.
            let files = inputs.iter().map(|f| f.path.clone()).collect();
            close(cx);
            navigate(Nav::Tool { id: id.to_string(), files, values: None }, cx);
            return;
        }
        let values = m.tool_values(id);
        let policy = match out {
            Some(p) => OutputPolicy::Folder(p),
            None => m.output_policy(id),
        };
        let req = RunRequest {
            steps: vec![Step { tool: tool.clone(), values }],
            inputs,
            output: policy,
            chain_name: None,
            max_parallel: m.settings.max_parallel,
            record: true,
        };
        let handle = crate::model::start_run(req, cx);
        self.run = Some(QuickRun { tool: tool.meta().name(), frac: 0.0, done: None, to_clipboard });
        cx.notify();
        let rx = handle.events.clone();
        cx.spawn(async move |this, cx| {
            let _keep = handle;
            let mut jobs = 1usize;
            let mut fracs: Vec<f32> = vec![0.0];
            loop {
                cx.background_executor().timer(Duration::from_millis(40)).await;
                let mut finished = None;
                while let Ok(ev) = rx.try_recv() {
                    match ev {
                        RunEvent::StepStarted { jobs: j, .. } => {
                            jobs = j.len().max(1);
                            fracs = vec![0.0; jobs];
                        }
                        RunEvent::JobProgress { job, frac, .. } => {
                            if let Some(f) = fracs.get_mut(job) {
                                *f = frac;
                            }
                        }
                        RunEvent::JobDone { job, .. } | RunEvent::JobFailed { job, .. } | RunEvent::JobCancelled { job, .. } => {
                            if let Some(f) = fracs.get_mut(job) {
                                *f = 1.0;
                            }
                        }
                        RunEvent::Finished(r) => finished = Some(r),
                        _ => {}
                    }
                }
                let frac = fracs.iter().sum::<f32>() / jobs as f32;
                let ok = this.update(cx, |p, cx| {
                    if let Some(run) = &mut p.run {
                        run.frac = frac;
                        if let Some(r) = &finished {
                            let summary = crate::tool_screen::summary_line(r);
                            run.done = Some((r.status, r.outputs.clone(), summary));
                            let to_clip = run.to_clipboard;
                            let outs = r.outputs.clone();
                            model(cx).update(cx, |m, cx| m.push_run((**r).clone(), cx));
                            if to_clip {
                                copy_to_clipboard(&outs, cx);
                            }
                        }
                    }
                    cx.notify();
                });
                if ok.is_err() || finished.is_some() {
                    break;
                }
            }
        })
        .detach();
    }

    fn run_on_clipboard(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(clip) = self.clip.clone() else { return };
        let dir = castor_core::paths::temp_dir().join("clipboard");
        let _ = std::fs::create_dir_all(&dir);
        let inputs = match clip {
            Clip::Image { png, .. } => {
                let p = dir.join(format!("clipboard-{}.png", castor_core::history::new_run_id()));
                if std::fs::write(&p, png).is_err() {
                    return;
                }
                vec![FileInfo::from_path(&p)]
            }
            Clip::Files(paths) => paths.iter().map(FileInfo::from_path).collect(),
        };
        self.run_tool(id, inputs, true, Some(dir), cx);
    }

    fn on_drop(&mut self, paths: &ExternalPaths, cx: &mut Context<Self>) {
        self.files = castor_core::files::expand(paths.paths(), false);
        self.run = None;
        cx.notify();
    }
}

fn read_clipboard(cx: &App) -> Option<Clip> {
    let item = cx.read_from_clipboard()?;
    for e in item.entries() {
        match e {
            ClipboardEntry::Image(img) => {
                let png = match img.format {
                    ImageFormat::Png => img.bytes.clone(),
                    _ => {
                        let d = image::load_from_memory(&img.bytes).ok()?;
                        let mut out = std::io::Cursor::new(Vec::new());
                        d.write_to(&mut out, image::ImageFormat::Png).ok()?;
                        out.into_inner()
                    }
                };
                let (w, h) = image::load_from_memory_with_format(&png, image::ImageFormat::Png).map(|i| (i.width(), i.height())).unwrap_or((0, 0));
                return Some(Clip::Image { png, w, h });
            }
            ClipboardEntry::ExternalPaths(p) if !p.paths().is_empty() => return Some(Clip::Files(p.paths().to_vec())),
            _ => {}
        }
    }
    None
}

/// Put results back on the clipboard: an image result as an image, otherwise the paths.
fn copy_to_clipboard(outs: &[PathBuf], cx: &mut App) {
    let Some(first) = outs.first() else { return };
    let ext = first.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let fmt = match ext.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        "bmp" => Some(ImageFormat::Bmp),
        "tif" | "tiff" => Some(ImageFormat::Tiff),
        _ => None,
    };
    if let (Some(fmt), Ok(bytes)) = (fmt, std::fs::read(first)) {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(fmt, bytes)));
    } else {
        let text = outs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

impl Render for QuickPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let m = model(cx).read(cx);
        let reg = m.registry.clone();
        let favorites: Vec<Arc<dyn Tool>> = m.settings.favorites.iter().filter_map(|id| reg.get(id)).take(6).collect();
        let last: Vec<castor_core::history::RunRecord> = m.history.iter().take(3).cloned().collect();
        let recent = m.recent_tools(8);
        let favs_ids = m.settings.favorites.clone();
        let searching = !self.query.trim().is_empty();

        let tool_row = |id_prefix: &'static str, i: usize, tool: &Arc<dyn Tool>, sel: bool| {
            let meta = tool.meta();
            div()
                .id((id_prefix, i))
                .h(px(30.0))
                .px(px(8.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .when(sel, |d| d.bg(k.surface))
                .hover(|s| s.bg(k.surface))
                .child(ui::family_swatch(&k, meta.family, 10.0))
                .child(ui::sans().flex_1().min_w(px(0.0)).truncate().text_color(k.ink).child(meta.name()))
        };

        let mut body = div().flex().flex_col().gap(px(12.0)).px(px(12.0)).pb(px(10.0));

        if searching {
            let res = self.results(cx);
            let sel = self.sel.min(res.len().saturating_sub(1));
            body = body.child(div().flex().flex_col().children(res.iter().enumerate().map(|(i, tool)| {
                let id = tool.meta().id.to_string();
                tool_row("pq-res", i, tool, i == sel).on_click(move |_, _, cx| {
                    let id = id.clone();
                    close(cx);
                    navigate(Nav::Tool { id, files: vec![], values: None }, cx);
                })
            })));
        } else {
            // Drop zone, or the best tools for the dropped files.
            let link = k.link;
            let tint = Hsla { a: 0.08, ..k.link };
            let drop: AnyElement = if self.files.is_empty() {
                div()
                    .id("pq-drop")
                    .h(px(70.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .border_1()
                    .border_dashed()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .bg(k.raised)
                    .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_color(link).bg(tint))
                    .on_drop(cx.listener(|p, paths: &ExternalPaths, _, cx| p.on_drop(paths, cx)))
                    .child(ui::roman().text_color(k.muted).child(t("app.panel.drop")))
                    .into_any_element()
            } else {
                let all = reg.all().to_vec();
                let best = crate::home::Home::suggested(&self.files, &all, &favs_ids, &recent, 6);
                let files = self.files.clone();
                div()
                    .id("pq-drop")
                    .flex()
                    .flex_col()
                    .gap(px(4.0))
                    .on_drop(cx.listener(|p, paths: &ExternalPaths, _, cx| p.on_drop(paths, cx)))
                    .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.panel.for_files", &[("files", &if files.len() == 1 { files[0].name() } else { castor_core::tr_args("common.n_files", &[("count", &files.len().to_string())]) })])))
                    .children(best.iter().filter_map(|id| reg.get(id)).enumerate().map(|(i, tool)| {
                        let id = tool.meta().id.to_string();
                        let files = files.clone();
                        tool_row("pq-best", i, &tool, false).on_click(cx.listener(move |p, _, _, cx| {
                            let inputs: Vec<FileInfo> = files.iter().filter(|f| p.results_accepts(&id, f, cx)).cloned().collect();
                            p.run_tool(&id, inputs, false, None, cx)
                        }))
                    }))
                    .into_any_element()
            };
            body = body.child(drop);

            // Current quick run.
            if let Some(run) = &self.run {
                let mut r = div().flex().flex_col().gap(px(6.0)).p(px(8.0)).border_1().border_color(k.rule).rounded(px(3.0)).bg(k.raised).child(ui::sans().font_weight(FontWeight::BOLD).child(run.tool.clone()));
                match &run.done {
                    None => r = r.child(Progress::new("pq-progress").value(run.frac * 100.0)),
                    Some((status, outs, summary)) => {
                        let ok = matches!(status, RunStatus::Complete | RunStatus::Partial);
                        r = r.child(ui::sans().text_size(px(13.0)).text_color(if ok { k.good } else { k.bad }).child(summary.clone()));
                        if let Some(first) = outs.first().cloned() {
                            let outs2 = outs.clone();
                            r = r.child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap(px(6.0))
                                    .child(Button::new("pq-show").xsmall().label(t("app.panel.show")).on_click(move |_, _, _| {
                                        let _ = castor_core::process::reveal(&first);
                                    }))
                                    .child(Button::new("pq-copy").xsmall().label(if run.to_clipboard { t("app.panel.copied") } else { t("app.panel.copy") }).on_click(move |_, _, cx| copy_to_clipboard(&outs2, cx))),
                            );
                        }
                    }
                }
                body = body.child(r);
            }

            // Clipboard line.
            if let Some(clip) = &self.clip {
                let (label, kind) = match clip {
                    Clip::Image { w, h, .. } => (format!("PNG {w} × {h}"), FileKind::Image),
                    Clip::Files(p) => (if p.len() == 1 { p[0].file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default() } else { castor_core::tr_args("common.n_files", &[("count", &p.len().to_string())]) }, FileKind::Other),
                };
                let probe = match clip {
                    Clip::Image { .. } => vec![FileInfo { path: PathBuf::from("clipboard.png"), kind, size: 1, is_dir: false }],
                    Clip::Files(p) => p.iter().map(FileInfo::from_path).collect(),
                };
                let all = reg.all().to_vec();
                let actions = crate::home::Home::suggested(&probe, &all, &favs_ids, &recent, 3);
                body = body.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap(px(6.0))
                                .child(ui::icon("clipboard", k.muted).size(px(14.0)))
                                .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.panel.clipboard", &[("what", &label)]))),
                        )
                        .child(div().flex().flex_row().flex_wrap().gap(px(4.0)).children(actions.iter().filter_map(|id| reg.get(id)).enumerate().map(|(i, tool)| {
                            let id = tool.meta().id.to_string();
                            Button::new(("pq-clip", i)).xsmall().label(tool.meta().name()).on_click(cx.listener(move |p, _, _, cx| p.run_on_clipboard(&id, cx)))
                        }))),
                );
            }

            // Favorites.
            if !favorites.is_empty() {
                body = body
                    .child(div().font_family(fonts().caps.clone()).text_size(px(13.0)).text_color(k.muted).child(ui::small_caps(&t("app.nav.favorites"))))
                    .child(div().flex().flex_col().children(favorites.iter().enumerate().map(|(i, tool)| {
                        let id = tool.meta().id.to_string();
                        tool_row("pq-fav", i, tool, false).on_click(cx.listener(move |p, _, _, cx| {
                            if p.files.is_empty() {
                                let id = id.clone();
                                close(cx);
                                navigate(Nav::Tool { id, files: vec![], values: None }, cx);
                            } else {
                                let inputs: Vec<FileInfo> = p.files.iter().filter(|f| p.results_accepts(&id, f, cx)).cloned().collect();
                                p.run_tool(&id, inputs, false, None, cx);
                            }
                        }))
                    })));
            }

            // Last runs.
            if !last.is_empty() {
                body = body
                    .child(div().font_family(fonts().caps.clone()).text_size(px(13.0)).text_color(k.muted).child(ui::small_caps(&t("app.panel.last_runs"))))
                    .child(div().flex().flex_col().gap(px(2.0)).children(last.iter().enumerate().map(|(i, r)| {
                        let out = r.outputs.first().cloned();
                        div()
                            .id(("pq-run", i))
                            .h(px(26.0))
                            .px(px(8.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .rounded(px(3.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(k.surface))
                            .on_click(move |_, _, _| {
                                if let Some(p) = &out {
                                    let _ = castor_core::process::reveal(p);
                                }
                            })
                            .child(ui::sans().flex_1().min_w(px(0.0)).truncate().text_size(px(13.5)).child(crate::runs::run_title(r)))
                            .child(ui::sans().text_size(px(12.5)).text_color(crate::runs::status_color(&k, r.status)).child(crate::runs::status_label(r.status)))
                    })));
            }
        }

        div()
            .id("quick-panel")
            .size_full()
            .flex()
            .flex_col()
            .bg(k.paper)
            .border_1()
            .border_color(k.rule)
            .text_color(k.ink)
            .font_family(fonts().sans.clone())
            .text_size(px(14.0))
            .on_drop(cx.listener(|p, paths: &ExternalPaths, _, cx| p.on_drop(paths, cx)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(12.0))
                    .pt(px(12.0))
                    .pb(px(8.0))
                    .child(img(if k.dark { crate::assets::MARK_DARK } else { crate::assets::MARK_LIGHT }).size(px(16.0)))
                    .child(div().flex_1().child(Input::new(&self.input).small().cleanable(true))),
            )
            .child(div().id("pq-body").flex_1().min_h(px(0.0)).overflow_y_scroll().child(body))
            .child(ui::hrule(k.rule, 1.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .px(px(12.0))
                    .py(px(8.0))
                    .gap(px(8.0))
                    .child(Button::new("pq-open").small().ghost().label(t("app.tray.open")).on_click(|_, _, cx| {
                        close(cx);
                        crate::app::show_main_window(cx);
                    }))
                    .child(div().flex_1())
                    .child(Button::new("pq-quit").small().ghost().label(t("app.tray.quit")).on_click(|_, _, cx| crate::app::quit(cx))),
            )
    }
}

impl QuickPanel {
    fn results_accepts(&self, id: &str, f: &FileInfo, cx: &App) -> bool {
        crate::model::registry(cx).get(id).is_some_and(|t| t.accepts(f))
    }
}
