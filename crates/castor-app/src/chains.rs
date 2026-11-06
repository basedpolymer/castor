//! Chains page (DESIGN §4): templates and user chains; an editor with steps
//! ("Step 1 ◇ Resize → Step 2 ◇ Compress"), each with the generated form; run on files or a
//! folder; export / import TOML.

use std::path::PathBuf;
use std::time::Duration;

use castor_core::chain::{self, Chain, ChainStep};
use castor_core::history::RunRecord;
use castor_core::runner::{self, RunEvent, RunRequest, Step};
use castor_core::{FileInfo, Values};
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::*;

use crate::form::{OptionsForm, Opt, persistable};
use crate::home::PAGE_PAD;
use crate::model::model;
use crate::theme::{Tokens, fonts, tokens};
use crate::ui::{self, t, ta};

struct EditStep {
    tool_id: String,
    form: Entity<OptionsForm>,
}

struct Editor {
    name: Entity<InputState>,
    steps: Vec<EditStep>,
    /// File of a user chain (None for a template or a new chain).
    path: Option<PathBuf>,
    builtin: bool,
    /// Steps of a template whose tools are not installed.
    skipped: Vec<String>,
}

struct ChainRun {
    step: usize,
    steps: usize,
    jobs: Vec<f32>,
    record: Option<RunRecord>,
    handle: Option<runner::RunHandle>,
}

pub struct ChainsPage {
    templates: Vec<Chain>,
    user: Vec<(PathBuf, Chain)>,
    selected: Option<String>,
    editor: Option<Editor>,
    picker: Entity<SelectState<SearchableVec<Opt>>>,
    run: Option<ChainRun>,
    list_w: f32,
    _subs: Vec<Subscription>,
}

impl ChainsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let reg = crate::model::registry(cx);
        let mut items: Vec<Opt> = reg
            .all()
            .iter()
            .filter(|t| t.meta().status != castor_core::ToolStatus::Soon)
            .map(|t| Opt { label: format!("{} — {}", t.meta().family.name(), t.meta().name()).into(), value: t.meta().id.to_string(), font: None })
            .collect();
        items.sort_by(|a, b| a.label.cmp(&b.label));
        let picker = cx.new(|cx| SelectState::new(SearchableVec::from(items), None, window, cx).searchable(true));
        let sub = cx.subscribe_in(&picker, window, |this, _, ev: &SelectEvent<SearchableVec<Opt>>, window, cx| {
            if let SelectEvent::Confirm(Some(id)) = ev {
                this.add_step(id.clone(), None, window, cx);
            }
        });
        let mut p = ChainsPage { templates: chain::templates(), user: Vec::new(), selected: None, editor: None, picker, run: None, list_w: 300.0, _subs: vec![sub] };
        p.reload(cx);
        p
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let user = cx.background_executor().spawn(async move { chain::load_all() }).await;
            let _ = this.update(cx, |p, cx| {
                p.user = user;
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_by_name(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((p, c)) = self.user.iter().find(|(_, c)| c.name == name).cloned() {
            self.open(c, Some(p), window, cx);
        } else if let Some(c) = self.templates.iter().find(|c| c.name == name).cloned() {
            self.open(c, None, window, cx);
        } else if let Some((p, c)) = chain::load_all().into_iter().find(|(_, c)| c.name == name) {
            self.open(c, Some(p), window, cx);
        }
    }

    fn open(&mut self, c: Chain, path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let reg = crate::model::registry(cx);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(c.name.clone()).placeholder(t("app.chains.name_placeholder")));
        let mut ed = Editor { name, steps: Vec::new(), path, builtin: c.builtin, skipped: Vec::new() };
        for s in &c.steps {
            let Some(tool) = reg.get(&s.tool) else {
                ed.skipped.push(s.tool.clone());
                continue;
            };
            let mut v = Values::defaults(&tool.options());
            for (k, val) in &s.values.0 {
                v.set(k, val.clone());
            }
            let form = cx.new(|cx| OptionsForm::new(tool.clone(), v, window, cx));
            ed.steps.push(EditStep { tool_id: s.tool.clone(), form });
        }
        self.selected = Some(c.name.clone());
        self.editor = Some(ed);
        self.run = None;
        cx.notify();
    }

    fn new_chain(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let c = Chain { name: castor_core::tr("app.chains.untitled"), steps: vec![], builtin: false };
        self.open(c, None, window, cx);
    }

    fn add_step(&mut self, id: String, values: Option<Values>, window: &mut Window, cx: &mut Context<Self>) {
        let reg = crate::model::registry(cx);
        let Some(tool) = reg.get(&id) else { return };
        if self.editor.is_none() {
            self.new_chain(window, cx);
        }
        let v = values.unwrap_or_else(|| model(cx).read(cx).tool_values(&id));
        let form = cx.new(|cx| OptionsForm::new(tool.clone(), v, window, cx));
        if let Some(ed) = &mut self.editor {
            ed.steps.push(EditStep { tool_id: id, form });
        }
        cx.notify();
    }

    fn current_chain(&self, cx: &App) -> Option<Chain> {
        let ed = self.editor.as_ref()?;
        let reg = crate::model::registry(cx);
        let name = ed.name.read(cx).value().trim().to_string();
        Some(Chain {
            name: if name.is_empty() { castor_core::tr("app.chains.untitled") } else { name },
            steps: ed
                .steps
                .iter()
                .filter_map(|s| {
                    let tool = reg.get(&s.tool_id)?;
                    Some(ChainStep { tool: s.tool_id.clone(), values: persistable(tool.as_ref(), s.form.read(cx).values()) })
                })
                .collect(),
            builtin: false,
        })
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.current_chain(cx) else { return };
        match chain::save(&c) {
            Ok(p) => {
                if let Some(ed) = &mut self.editor {
                    if let Some(old) = &ed.path {
                        if *old != p {
                            let _ = std::fs::remove_file(old);
                        }
                    }
                    ed.path = Some(p);
                    ed.builtin = false;
                }
                self.selected = Some(c.name.clone());
                window.push_notification(t("app.chains.saved").to_string(), cx);
                self.reload(cx);
            }
            Err(e) => window.push_notification(format!("{e:#}"), cx),
        }
    }

    fn delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.editor.as_ref().and_then(|e| e.path.clone()) else { return };
        let this = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let path = path.clone();
            let this = this.clone();
            alert
                .title(t("app.chains.delete_title"))
                .description(t("app.chains.delete_body"))
                .show_cancel(true)
                .cancel_text(t("common.cancel"))
                .ok_text(t("app.common.delete"))
                .ok_variant(ButtonVariant::Danger)
                .on_ok(move |_, _, cx| {
                    let _ = std::fs::remove_file(&path);
                    let _ = this.update(cx, |p, cx| {
                        p.editor = None;
                        p.selected = None;
                        p.reload(cx);
                    });
                    true
                })
        });
    }

    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = self.current_chain(cx) else { return };
        let Ok(text) = c.to_toml() else { return };
        let dir = directories_home();
        let rx = cx.prompt_for_new_path(&dir, Some(&format!("{}.toml", castor_core::job::sanitize_file_name(&c.name))));
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(p))) = rx.await {
                let res = std::fs::write(&p, text);
                let _ = cx.update(|window, cx| match res {
                    Ok(()) => window.push_notification(t("app.chains.exported").to_string(), cx),
                    Err(e) => window.push_notification(e.to_string(), cx),
                });
            }
        })
        .detach();
    }

    fn import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let mut errors = Vec::new();
                for p in paths {
                    match std::fs::read_to_string(&p).map_err(anyhow::Error::from).and_then(|s| Chain::from_toml(&s)).and_then(|c| chain::save(&c)) {
                        Ok(_) => {}
                        Err(e) => errors.push(format!("{}: {e:#}", p.display())),
                    }
                }
                let _ = this.update_in(cx, |p, window, cx| {
                    if errors.is_empty() {
                        window.push_notification(t("app.chains.imported").to_string(), cx);
                    } else {
                        window.push_notification(errors.join("\n"), cx);
                    }
                    p.reload(cx);
                });
            }
        })
        .detach();
    }

    fn pick_and_run(&mut self, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: !folder, directories: folder, multiple: !folder, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let files = cx.background_executor().spawn(async move { castor_core::files::expand(&paths, true) }).await;
                let _ = this.update(cx, |p, cx| p.start(files, cx));
            }
        })
        .detach();
    }

    fn start(&mut self, inputs: Vec<FileInfo>, cx: &mut Context<Self>) {
        let Some(c) = self.current_chain(cx) else { return };
        let reg = crate::model::registry(cx);
        let Ok(steps) = c.resolve(&reg) else { return };
        if steps.is_empty() {
            return;
        }
        let last_id = c.steps.last().map(|s| s.tool.clone()).unwrap_or_default();
        let s = crate::model::settings(cx);
        let req = RunRequest {
            steps: steps.into_iter().map(|s: Step| s).collect(),
            inputs,
            output: s.output_policy(&last_id),
            chain_name: Some(c.name.clone()),
            max_parallel: s.max_parallel,
            record: true,
        };
        let n = req.steps.len();
        let handle = runner::start(req);
        let rx = handle.events.clone();
        self.run = Some(ChainRun { step: 0, steps: n, jobs: vec![], record: None, handle: Some(handle) });
        cx.notify();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(50)).await;
                let mut evs = Vec::new();
                while let Ok(e) = rx.try_recv() {
                    evs.push(e);
                }
                if evs.is_empty() {
                    continue;
                }
                let done = this
                    .update(cx, |p, cx| {
                        let mut done = false;
                        if let Some(r) = &mut p.run {
                            for e in evs {
                                match e {
                                    RunEvent::StepStarted { step, jobs, .. } => {
                                        r.step = step;
                                        r.jobs = vec![0.0; jobs.len()];
                                    }
                                    RunEvent::JobProgress { job, frac, .. } => {
                                        if let Some(j) = r.jobs.get_mut(job) {
                                            *j = frac;
                                        }
                                    }
                                    RunEvent::JobDone { job, .. } | RunEvent::JobFailed { job, .. } | RunEvent::JobCancelled { job, .. } => {
                                        if let Some(j) = r.jobs.get_mut(job) {
                                            *j = 1.0;
                                        }
                                    }
                                    RunEvent::Finished(rec) => {
                                        r.record = Some((*rec).clone());
                                        r.handle = None;
                                        model(cx).update(cx, |m, cx| m.push_run(*rec, cx));
                                        done = true;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        cx.notify();
                        done
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
        })
        .detach();
    }

    fn render_list(&self, k: &Tokens, cx: &mut Context<Self>) -> impl IntoElement {
        let reg = crate::model::registry(cx);
        let entry = |id: SharedString, c: &Chain, sel: bool, missing: usize| {
            let names: Vec<String> = c.steps.iter().filter(|s| reg.get(&s.tool).is_some()).map(|s| castor_core::tr(&format!("{}.name", s.tool))).collect();
            div()
                .id(id)
                .px(px(10.0))
                .py(px(7.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .rounded(px(3.0))
                .cursor_pointer()
                .when(sel, |d| d.bg(k.raised).border_1().border_color(k.rule))
                .hover(|s| s.bg(k.surface))
                .child(ui::sans().font_weight(FontWeight::BOLD).text_color(k.ink).child(c.name.clone()))
                .child(ui::roman().text_size(px(13.0)).text_color(k.muted).truncate().child(if names.is_empty() { castor_core::tr("app.chains.no_steps") } else { names.join(" → ") }))
                .when(missing > 0, |d| d.child(ui::sans().text_size(px(12.5)).text_color(k.warn).child(ta("app.chains.missing", &[("count", &missing.to_string())]))))
        };
        let sel = self.selected.clone();
        div()
            .w(px(self.list_w))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(6.0))
                    .child(Button::new("new-chain").small().primary().icon(gpui_kit::component::IconName::Plus).label(t("app.chains.new")).on_click(cx.listener(|p, _, window, cx| p.new_chain(window, cx))))
                    .child(Button::new("import-chain").small().label(t("app.chains.import")).on_click(cx.listener(|p, _, window, cx| p.import(window, cx)))),
            )
            .child(
                div()
                    .id("chains-list")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(div().pt(px(8.0)).font_family(fonts().caps.clone()).text_size(px(13.0)).text_color(k.muted).child(ui::small_caps(&t("app.chains.mine"))))
                    .when(self.user.is_empty(), |d| d.child(ui::roman().text_size(px(13.5)).italic().text_color(k.faint).child(t("app.chains.mine_empty"))))
                    .children(self.user.iter().enumerate().map(|(i, (p, c))| {
                        let (c2, p2) = (c.clone(), p.clone());
                        let missing = c.steps.iter().filter(|s| reg.get(&s.tool).is_none()).count();
                        entry(SharedString::from(format!("user-chain-{i}")), c, sel.as_deref() == Some(&c.name), missing)
                            .on_click(cx.listener(move |pg, _, window, cx| pg.open(c2.clone(), Some(p2.clone()), window, cx)))
                    }))
                    .child(div().pt(px(12.0)).font_family(fonts().caps.clone()).text_size(px(13.0)).text_color(k.muted).child(ui::small_caps(&t("app.chains.templates"))))
                    .children(self.templates.iter().enumerate().map(|(i, c)| {
                        let c2 = c.clone();
                        let missing = c.steps.iter().filter(|s| reg.get(&s.tool).is_none()).count();
                        entry(SharedString::from(format!("tpl-chain-{i}")), c, sel.as_deref() == Some(&c.name), missing)
                            .on_click(cx.listener(move |pg, _, window, cx| pg.open(c2.clone(), None, window, cx)))
                    })),
            )
    }

    fn render_editor(&self, k: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let Some(ed) = &self.editor else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(ui::roman().italic().text_color(k.muted).child(t("app.chains.pick")))
                .into_any_element();
        };
        let reg = crate::model::registry(cx);
        let n = ed.steps.len();
        let running = self.run.as_ref().is_some_and(|r| r.record.is_none());
        // "Step 1 ◇ Resize → Step 2 ◇ Compress"
        let flow = div().flex().flex_row().flex_wrap().items_center().gap(px(8.0)).children(ed.steps.iter().enumerate().flat_map(|(i, s)| {
            let name = reg.get(&s.tool_id).map(|t| t.meta().name()).unwrap_or_default();
            let current = self.run.as_ref().is_some_and(|r| r.record.is_none() && r.step == i);
            let mut v: Vec<AnyElement> = Vec::new();
            if i > 0 {
                v.push(ui::sans().text_color(k.faint).child("→").into_any_element());
            }
            v.push(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(ui::sans().text_color(k.muted).child(ta("app.chains.step", &[("n", &(i + 1).to_string())])))
                    .child(if current { ui::diamond_filled(10.0, k.ink) } else { ui::diamond(10.0, k.ink) })
                    .child(ui::sans().font_weight(FontWeight::BOLD).child(name))
                    .into_any_element(),
            );
            v
        }));
        let steps = ed.steps.iter().enumerate().map(|(i, s)| {
            let tool = reg.get(&s.tool_id);
            let fam = tool.as_ref().map(|t| t.meta().family);
            div()
                .flex()
                .flex_col()
                .gap(px(10.0))
                .p(px(14.0))
                .border_1()
                .border_color(k.rule)
                .rounded(px(3.0))
                .bg(k.raised)
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(8.0))
                        .when_some(fam, |d, f| d.child(ui::family_swatch(k, f, 12.0)))
                        .child(ui::sans().font_weight(FontWeight::BOLD).flex_1().child(format!("{} · {}", ta("app.chains.step", &[("n", &(i + 1).to_string())]), tool.map(|t| t.meta().name()).unwrap_or_default())))
                        .child(Button::new(("step-up", i)).ghost().xsmall().disabled(i == 0 || running).tooltip(t("app.chains.up")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("arrow-up"))).on_click(cx.listener(move |p, _, _, cx| {
                            if let Some(ed) = &mut p.editor {
                                if i > 0 {
                                    ed.steps.swap(i, i - 1);
                                }
                            }
                            cx.notify();
                        })))
                        .child(Button::new(("step-down", i)).ghost().xsmall().disabled(i + 1 >= n || running).tooltip(t("app.chains.down")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("arrow-down"))).on_click(cx.listener(move |p, _, _, cx| {
                            if let Some(ed) = &mut p.editor {
                                if i + 1 < ed.steps.len() {
                                    ed.steps.swap(i, i + 1);
                                }
                            }
                            cx.notify();
                        })))
                        .child(Button::new(("step-rm", i)).ghost().xsmall().disabled(running).tooltip(t("app.chains.remove")).icon(gpui_kit::component::IconName::Close).on_click(cx.listener(move |p, _, _, cx| {
                            if let Some(ed) = &mut p.editor {
                                if i < ed.steps.len() {
                                    ed.steps.remove(i);
                                }
                            }
                            cx.notify();
                        }))),
                )
                .child(s.form.clone())
        });
        let run_line: Option<AnyElement> = self.run.as_ref().map(|r| match &r.record {
            None => {
                let frac = if r.jobs.is_empty() { 0.0 } else { r.jobs.iter().sum::<f32>() / r.jobs.len() as f32 };
                let total = (r.step as f32 + frac) / r.steps.max(1) as f32;
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(div().w(px(220.0)).child(Progress::new("chain-progress").value(total * 100.0)))
                    .child(ui::sans().text_color(k.muted).child(ta("app.chains.running", &[("n", &(r.step + 1).to_string()), ("of", &r.steps.to_string())])))
                    .child(Button::new("chain-cancel").small().label(t("common.cancel")).on_click(cx.listener(|p, _, _, cx| {
                        if let Some(h) = p.run.as_ref().and_then(|r| r.handle.as_ref()) {
                            h.cancel();
                        }
                        cx.notify();
                    })))
                    .into_any_element()
            }
            Some(rec) => {
                let first = rec.outputs.first().cloned();
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(ui::sans().font_weight(FontWeight::BOLD).text_color(crate::runs::status_color(k, rec.status)).child(crate::runs::status_label(rec.status)))
                    .child(ui::sans().child(crate::tool_screen::summary_line(rec)))
                    .when_some(first, |d, p| {
                        d.child(Button::new("chain-reveal").small().label(t("common.reveal")).on_click(move |_, _, _| {
                            let _ = castor_core::process::reveal(&p);
                        }))
                    })
                    .when(!rec.errors.is_empty(), |d| d.child(ui::sans().text_size(px(13.0)).text_color(k.bad).truncate().child(rec.errors[0].clone())))
                    .into_any_element()
            }
        });
        div()
            .id("chain-editor")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.0))
                    .pb(px(30.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .flex_wrap()
                            .child(div().flex_1().min_w(px(180.0)).child(Input::new(&ed.name)))
                            .child(Button::new("chain-save").small().label(if ed.builtin { t("app.chains.save_copy") } else { t("app.common.save") }).on_click(cx.listener(|p, _, window, cx| p.save(window, cx))))
                            .child(Button::new("chain-export").small().label(t("app.chains.export")).on_click(cx.listener(|p, _, window, cx| p.export(window, cx))))
                            .when(ed.path.is_some(), |d| d.child(Button::new("chain-delete").small().danger().label(t("app.common.delete")).on_click(cx.listener(|p, _, window, cx| p.delete(window, cx))))),
                    )
                    .when(n > 0, |d| d.child(flow))
                    .when(!ed.skipped.is_empty(), |d| {
                        d.child(ui::roman().text_size(px(13.5)).text_color(k.warn).child(ta("app.chains.skipped", &[("tools", &ed.skipped.join(", "))])))
                    })
                    .children(steps)
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(ui::sans().text_color(k.muted).child(t("app.chains.add_step")))
                            .child(div().w(px(320.0)).child(Select::new(&self.picker).small().placeholder(t("app.chains.pick_tool")))),
                    )
                    .child(ui::hrule(k.rule, 1.0))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(8.0))
                            .child(Button::new("chain-run-files").primary().disabled(n == 0 || running).label(t("app.chains.run_files")).on_click(cx.listener(|p, _, window, cx| p.pick_and_run(false, window, cx))))
                            .child(Button::new("chain-run-folder").disabled(n == 0 || running).label(t("app.chains.run_folder")).on_click(cx.listener(|p, _, window, cx| p.pick_and_run(true, window, cx)))),
                    )
                    .when_some(run_line, |d, l| d.child(l)),
            )
            .into_any_element()
    }
}

fn directories_home() -> PathBuf {
    std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from).unwrap_or_else(castor_core::paths::default_output_dir)
}

impl Render for ChainsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        self.list_w = if window.viewport_size().width.as_f32() < 1100.0 { 240.0 } else { 300.0 };
        let count = self.templates.len() + self.user.len();
        div()
            .id("chains-page")
            .size_full()
            .flex()
            .flex_col()
            .px(px(PAGE_PAD))
            .pb(px(16.0))
            .gap(px(18.0))
            .child(ui::page_header(cx, t("app.nav.chains"), Some(count.to_string()), Some(t("app.chains.subtitle").into_any_element())))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h(px(0.0))
                    .gap(px(28.0))
                    .child(self.render_list(&k, cx))
                    .child(self.render_editor(&k, cx)),
            )
    }
}
