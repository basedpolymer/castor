//! Settings → Workspaces: the list, and an editor for one workspace (name, brand colours,
//! logo, signature, output folder).

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::model::model;
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t, ta};
use crate::workspaces::{self as ws, Workspace};

#[derive(Clone, Copy, PartialEq)]
enum PathField {
    Logo,
    Signature,
    Output,
}

pub struct WorkspaceEditor {
    editing: Option<String>,
    name: Entity<InputState>,
    color: Entity<InputState>,
    accent: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl WorkspaceEditor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.ws.name_ph")));
        let color = cx.new(|cx| InputState::new(window, cx).placeholder("#1b1616"));
        let accent = cx.new(|cx| InputState::new(window, cx).placeholder("#ffeca4"));
        let mut subs = Vec::new();
        for (inp, field) in [(&name, 0u8), (&color, 1), (&accent, 2)] {
            subs.push(cx.subscribe(inp, move |this, st, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = st.read(cx).value().to_string();
                    this.update_current(cx, |w| match field {
                        0 => {
                            if !v.trim().is_empty() {
                                w.name = v.trim().to_string()
                            }
                        }
                        1 => w.color = v.trim().to_string(),
                        _ => w.accent = v.trim().to_string(),
                    });
                }
            }));
        }
        subs.push(cx.observe(&model(cx), |_, _, cx| cx.notify()));
        let mut e = WorkspaceEditor { editing: None, name, color, accent, _subs: subs };
        // Show the active workspace's details right away.
        let active = crate::model::settings(cx).active_workspace.clone();
        if !active.is_empty() {
            e.edit(&active, window, cx);
        }
        e
    }

    fn list(cx: &App) -> Vec<Workspace> {
        ws::all(crate::model::settings(cx))
    }

    /// Change the workspace being edited (renames follow the active workspace).
    fn update_current(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Workspace)) {
        let Some(cur) = self.editing.clone() else { return };
        let mut list = Self::list(cx);
        let Some(w) = list.iter_mut().find(|w| w.name == cur) else { return };
        f(w);
        let new_name = w.name.clone();
        if new_name != cur && list.iter().filter(|x| x.name == new_name).count() > 1 {
            return; // Name taken: keep the old one until it is unique.
        }
        self.editing = Some(new_name.clone());
        model(cx).update(cx, |m, cx| {
            m.update_settings(cx, move |s| {
                if s.active_workspace == cur {
                    s.active_workspace = new_name.clone();
                }
                ws::store(s, &list);
            })
        });
    }

    fn edit(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(w) = Self::list(cx).into_iter().find(|w| w.name == name) else { return };
        self.editing = Some(w.name.clone());
        self.name.update(cx, |i, cx| i.set_value(w.name.clone(), window, cx));
        self.color.update(cx, |i, cx| i.set_value(w.color.clone(), window, cx));
        self.accent.update(cx, |i, cx| i.set_value(w.accent.clone(), window, cx));
        cx.notify();
    }

    fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut list = Self::list(cx);
        let name = ws::new_name(&list, &t("app.ws.new_name"));
        list.push(Workspace { name: name.clone(), color: "#1b1616".into(), accent: "#ffeca4".into(), ..Default::default() });
        model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| ws::store(s, &list)));
        self.edit(&name, window, cx);
    }

    fn delete(&mut self, name: &str, cx: &mut Context<Self>) {
        let list: Vec<Workspace> = Self::list(cx).into_iter().filter(|w| w.name != name).collect();
        if self.editing.as_deref() == Some(name) {
            self.editing = None;
        }
        model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| ws::store(s, &list)));
    }

    fn choose(&mut self, field: PathField, window: &mut Window, cx: &mut Context<Self>) {
        let folder = field == PathField::Output;
        let rx = cx.prompt_for_paths(PathPromptOptions { files: !folder, directories: folder, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update(cx, |s, cx| s.set_path(field, Some(p), cx));
                }
            }
        })
        .detach();
    }

    fn set_path(&mut self, field: PathField, p: Option<PathBuf>, cx: &mut Context<Self>) {
        self.update_current(cx, |w| match field {
            PathField::Logo => w.logo = p,
            PathField::Signature => w.signature = p,
            PathField::Output => w.output_folder = p,
        });
    }

    fn path_row(&self, k: &Tokens, id: &'static str, label: SharedString, value: Option<PathBuf>, field: PathField, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.0))
            .child(ui::sans().w(px(120.0)).flex_none().text_color(k.muted).child(label))
            .child(match &value {
                Some(p) => ui::mono().flex_1().min_w(px(0.0)).text_ellipsis_start().whitespace_nowrap().overflow_hidden().child(p.display().to_string()).into_any_element(),
                None => ui::roman().flex_1().italic().text_color(k.faint).child(t("app.ws.none")).into_any_element(),
            })
            .child(Button::new(SharedString::from(format!("{id}-choose"))).small().label(t("app.common.choose")).on_click(cx.listener(move |s, _, w, cx| s.choose(field, w, cx))))
            .when(value.is_some(), |d| {
                d.child(Button::new(SharedString::from(format!("{id}-clear"))).small().ghost().label(t("app.common.clear")).on_click(cx.listener(move |s, _, _, cx| s.set_path(field, None, cx))))
            })
    }
}

/// A brand swatch: the logo if there is one, else a diamond in the brand colour.
pub fn badge(k: &Tokens, w: Option<&Workspace>, side: f32) -> AnyElement {
    match w {
        Some(w) if w.logo.as_ref().is_some_and(|p| p.is_file()) => img(w.logo.clone().unwrap()).size(px(side)).flex_none().object_fit(ObjectFit::Contain).into_any_element(),
        Some(w) => match w.swatch() {
            Some(c) => div().relative().size(px(side)).flex_none().child(ui::diamond_filled(side, c).absolute()).child(ui::diamond(side, k.ink_a(0.6)).absolute()).into_any_element(),
            None => ui::diamond(side, k.muted).into_any_element(),
        },
        None => ui::diamond(side, k.muted).into_any_element(),
    }
}

impl Render for WorkspaceEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let list = Self::list(cx);
        let active = crate::model::settings(cx).active_workspace.clone();
        let editing = self.editing.clone().and_then(|n| list.iter().find(|w| w.name == n).cloned());
        let rows = list.iter().enumerate().map(|(i, w)| {
            let (n1, n2, n3) = (w.name.clone(), w.name.clone(), w.name.clone());
            let is_active = w.name == active;
            div()
                .id(("ws-row", i))
                .h(px(36.0))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .when(self.editing.as_deref() == Some(w.name.as_str()), |d| d.bg(k.surface))
                .child(badge(&k, Some(w), 16.0))
                .child(ui::sans().flex_1().min_w(px(0.0)).truncate().text_color(k.ink).child(w.name.clone()))
                .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(ta("app.ws.runs", &[("count", &w.runs.len().to_string())])))
                .child(if is_active {
                    ui::tag(&k, t("app.ws.active"), k.tint(castor_core::Family::Create)).into_any_element()
                } else {
                    Button::new(("ws-use", i))
                        .xsmall()
                        .label(t("app.ws.use"))
                        .on_click(move |_, _, cx| {
                            let n = n1.clone();
                            model(cx).update(cx, |m, cx| m.update_settings(cx, move |s| s.active_workspace = n));
                        })
                        .into_any_element()
                })
                .child(Button::new(("ws-edit", i)).xsmall().ghost().label(t("app.ws.edit")).on_click(cx.listener(move |s, _, w, cx| s.edit(&n2, w, cx))))
                .child(Button::new(("ws-del", i)).xsmall().ghost().danger().label(t("app.common.delete")).on_click(cx.listener(move |s, _, _, cx| s.delete(&n3, cx))))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(ui::roman().max_w(px(700.0)).text_size(px(14.0)).text_color(k.muted).child(t("app.ws.help")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .max_w(px(700.0))
                    .child(ui::rule_strong(&k))
                    .child(
                        div()
                            .h(px(36.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(10.0))
                            .child(ui::diamond(16.0, k.muted))
                            .child(ui::sans().flex_1().text_color(k.ink).child(t("app.ws.default")))
                            .child(if active.is_empty() {
                                ui::tag(&k, t("app.ws.active"), k.tint(castor_core::Family::Create)).into_any_element()
                            } else {
                                Button::new("ws-use-default")
                                    .xsmall()
                                    .label(t("app.ws.use"))
                                    .on_click(|_, _, cx| model(cx).update(cx, |m, cx| m.update_settings(cx, |s| s.active_workspace.clear())))
                                    .into_any_element()
                            }),
                    )
                    .children(rows)
                    .child(ui::rule_strong(&k)),
            )
            .child(div().flex().flex_row().child(Button::new("ws-add").small().icon(gpui_kit::component::IconName::Plus).label(t("app.ws.add")).on_click(cx.listener(|s, _, w, cx| s.add(w, cx)))))
            .when_some(editing, |d, w| {
                d.child(
                    div()
                        .max_w(px(700.0))
                        .p(px(14.0))
                        .flex()
                        .flex_col()
                        .gap(px(10.0))
                        .border_1()
                        .border_color(k.rule)
                        .rounded(px(3.0))
                        .bg(k.raised)
                        .child(div().flex().flex_row().items_center().gap(px(8.0)).child(ui::sans().w(px(120.0)).flex_none().text_color(k.muted).child(t("app.ws.name"))).child(div().flex_1().child(Input::new(&self.name).small())))
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(ui::sans().w(px(120.0)).flex_none().text_color(k.muted).child(t("app.ws.colours")))
                                .child(div().size(px(20.0)).rounded(px(3.0)).border_1().border_color(k.rule).when_some(w.swatch(), |d, c| d.bg(c)))
                                .child(div().w(px(110.0)).child(Input::new(&self.color).small()))
                                .child(div().size(px(20.0)).rounded(px(3.0)).border_1().border_color(k.rule).when_some(ws::parse_hex(&w.accent), |d, [r, g, b]| d.bg(Hsla::from(gpui_kit::Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }))))
                                .child(div().w(px(110.0)).child(Input::new(&self.accent).small())),
                        )
                        .child(self.path_row(&k, "ws-logo", t("app.ws.logo"), w.logo.clone(), PathField::Logo, cx))
                        .child(self.path_row(&k, "ws-sig", t("app.ws.signature"), w.signature.clone(), PathField::Signature, cx))
                        .child(self.path_row(&k, "ws-out", t("app.ws.output"), w.output_folder.clone(), PathField::Output, cx))
                        .child(ui::roman().text_size(px(13.0)).text_color(k.muted).child(t("app.ws.prefill_help"))),
                )
            })
    }
}
