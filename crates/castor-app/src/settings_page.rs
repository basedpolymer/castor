//! Settings (DESIGN §4): a document — 1. General, 2. Menu bar / System tray, 3. Models &
//! helpers, 4. Updates, 5. Privacy, 6. About.

use castor_core::download::Asset;
use castor_core::settings::{Appearance, OutputLocation};
use castor_core::units;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::searchable_list::SearchableVec;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{IndexPath, Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::form::Opt;
use crate::home::PAGE_PAD;
use crate::model::{UpdateState, model};
use crate::theme::{Tokens, tokens};
use crate::ui::{self, t, ta};

/// `--scroll` for `--page settings` (screenshots).
pub struct StartScroll(pub Option<f32>);
impl Global for StartScroll {}

pub struct SettingsPage {
    scroll: ScrollHandle,
    workspaces: Entity<crate::workspace_editor::WorkspaceEditor>,
    integration: Entity<crate::integration::IntegrationSection>,
    language: Entity<SelectState<SearchableVec<Opt>>>,
    ai_key: Entity<InputState>,
    recording: bool,
    record_focus: FocusHandle,
    assets: Vec<(Asset, bool)>,
    login_error: Option<String>,
    _subs: Vec<Subscription>,
}

fn all_assets(cx: &App) -> Vec<(Asset, bool)> {
    let reg = crate::model::registry(cx);
    let mut out: Vec<(Asset, bool)> = Vec::new();
    for tool in reg.all() {
        let v = castor_core::Values::defaults(&tool.options());
        for a in tool.downloads(&v) {
            if !out.iter().any(|(x, _)| x.id == a.id) {
                let present = a.is_present();
                out.push((a, present));
            }
        }
    }
    out.sort_by(|a, b| a.0.id.cmp(b.0.id));
    out
}

fn auto_launch() -> anyhow::Result<auto_launch::AutoLaunch> {
    let exe = std::env::current_exe()?;
    let mut b = auto_launch::AutoLaunchBuilder::new();
    b.set_app_name("Castor").set_app_path(&exe.to_string_lossy()).set_args(&["--background"]);
    Ok(b.build()?)
}

/// Turn launch at login on or off.
pub fn set_launch_at_login(on: bool) -> anyhow::Result<()> {
    let al = auto_launch()?;
    if on { al.enable()? } else if al.is_enabled().unwrap_or(false) { al.disable()? }
    Ok(())
}

/// Human form of a GPUI keystroke string ("ctrl-alt-space" → "Ctrl Alt Space").
pub fn hotkey_label(s: &str) -> String {
    if s.is_empty() {
        return castor_core::tr("app.settings.hotkey_off");
    }
    s.split('-')
        .map(|p| match p {
            "cmd" | "platform" => if cfg!(target_os = "macos") { "⌘".to_string() } else { "Win".to_string() },
            "ctrl" | "control" => if cfg!(target_os = "macos") { "⌃".to_string() } else { "Ctrl".to_string() },
            "alt" => if cfg!(target_os = "macos") { "⌥".to_string() } else { "Alt".to_string() },
            "shift" => if cfg!(target_os = "macos") { "⇧".to_string() } else { "Shift".to_string() },
            k => {
                let mut c = k.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join(if cfg!(target_os = "macos") { "" } else { " " })
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let s = crate::model::settings(cx).clone();
        let mut items = vec![Opt { label: t("app.settings.language_system"), value: String::new(), font: None }];
        items.extend(castor_core::i18n::LANGUAGES.iter().map(|(c, n)| Opt { label: (*n).into(), value: c.to_string(), font: None }));
        let sel = items.iter().position(|o| o.value == s.language).map(IndexPath::new);
        let language = cx.new(|cx| SelectState::new(SearchableVec::from(items), sel, window, cx));
        let ai_key = cx.new(|cx| {
            InputState::new(window, cx).masked(true).placeholder(t("app.settings.ai_key_placeholder")).default_value(s.ai_api_key.clone().unwrap_or_default())
        });
        let m = model(cx);
        let subs = vec![
            cx.subscribe_in(&language, window, |_, _, ev: &SelectEvent<SearchableVec<Opt>>, window, cx| {
                if let SelectEvent::Confirm(Some(code)) = ev {
                    let code = code.clone();
                    crate::app::set_language(&code);
                    model(cx).update(cx, |m, cx| m.update_settings(cx, |s| s.language = code));
                    crate::tray::rebuild_menu(cx);
                    window.refresh();
                    cx.refresh_windows();
                }
            }),
            cx.observe(&m, |_, _, cx| cx.notify()),
        ];
        let workspaces = cx.new(|cx| crate::workspace_editor::WorkspaceEditor::new(window, cx));
        let integration = cx.new(|cx| crate::integration::IntegrationSection::new(window, cx));
        // `--scroll` (screenshots) applies to the page opened at launch.
        let scroll = ScrollHandle::new();
        if let Some(y) = cx.try_global::<StartScroll>().and_then(|h| h.0) {
            scroll.set_offset(point(px(0.0), px(-y)));
        }
        SettingsPage {
            scroll,
            workspaces,
            integration,
            language,
            ai_key,
            recording: false,
            record_focus: cx.focus_handle(),
            assets: all_assets(cx),
            login_error: None,
            _subs: subs,
        }
    }

    fn set(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut castor_core::settings::Settings)) {
        model(cx).update(cx, |m, cx| m.update_settings(cx, f));
    }

    fn row(&self, k: &Tokens, label: SharedString, help: Option<SharedString>, control: impl IntoElement) -> Div {
        div()
            .flex()
            .flex_row()
            .items_start()
            .gap(px(24.0))
            .py(px(10.0))
            .child(
                div()
                    .w(px(280.0))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(ui::sans().text_color(k.ink).child(label))
                    .when_some(help, |d, h| d.child(ui::roman().text_size(px(13.0)).line_height(px(16.5)).text_color(k.muted).child(h))),
            )
            .child(div().flex_1().min_w(px(0.0)).child(control))
    }

    fn choose_output_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut p))) = rx.await {
                if let Some(p) = p.pop() {
                    let _ = this.update(cx, |s, cx| {
                        s.set(cx, |st| {
                            st.output_folder = Some(p);
                            st.output_location = OutputLocation::Folder;
                        })
                    });
                }
            }
        })
        .detach();
    }

    fn on_record_key(&mut self, ev: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.recording {
            return;
        }
        let ks = &ev.keystroke;
        cx.stop_propagation();
        if ks.key == "escape" {
            self.recording = false;
            cx.notify();
            return;
        }
        let m = &ks.modifiers;
        if !(m.control || m.alt || m.platform) || ks.key.is_empty() || matches!(ks.key.as_str(), "shift" | "control" | "alt" | "platform" | "cmd" | "ctrl") {
            return;
        }
        let mut parts = Vec::new();
        if m.control {
            parts.push("ctrl");
        }
        if m.alt {
            parts.push("alt");
        }
        if m.shift {
            parts.push("shift");
        }
        if m.platform {
            parts.push("cmd");
        }
        let combo = format!("{}-{}", parts.join("-"), ks.key);
        self.recording = false;
        self.set(cx, |s| s.global_hotkey = combo);
        crate::tray::register_hotkey(cx);
        cx.notify();
    }

    fn delete_asset(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((a, _)) = self.assets.get(ix) {
            let p = a.path();
            if let Err(e) = std::fs::remove_file(&p) {
                window.push_notification(format!("{e}"), cx);
            }
        }
        self.assets = all_assets(cx);
        cx.notify();
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let m = model(cx).read(cx);
        let s = m.settings.clone();
        let update = m.update.clone();
        let version = m.version;
        let mac = cfg!(target_os = "macos");

        // 1. General
        let appearance = ui::segmented(
            "appearance",
            &k,
            vec![
                ("system".into(), t("app.settings.appearance_system")),
                ("light".into(), t("app.settings.appearance_light")),
                ("dark".into(), t("app.settings.appearance_dark")),
            ],
            match s.appearance {
                Appearance::System => "system",
                Appearance::Light => "light",
                Appearance::Dark => "dark",
            },
            {
                let this = cx.entity().downgrade();
                std::rc::Rc::new(move |v, window, cx| {
                    let a = match v {
                        "light" => Appearance::Light,
                        "dark" => Appearance::Dark,
                        _ => Appearance::System,
                    };
                    let _ = this.update(cx, |p, cx| p.set(cx, |s| s.appearance = a));
                    crate::theme::apply(a, Some(window), cx);
                })
            },
        );
        let out_seg = ui::segmented(
            "out-loc",
            &k,
            vec![("next".into(), t("app.tool.output_next_to")), ("folder".into(), t("app.settings.output_folder"))],
            if s.output_location == OutputLocation::NextToOriginal { "next" } else { "folder" },
            {
                let this = cx.entity().downgrade();
                std::rc::Rc::new(move |v, _, cx| {
                    let loc = if v == "folder" { OutputLocation::Folder } else { OutputLocation::NextToOriginal };
                    let _ = this.update(cx, |p, cx| p.set(cx, |s| s.output_location = loc));
                })
            },
        );
        let folder = s.output_folder.clone().unwrap_or_else(castor_core::paths::default_output_dir);
        let general = div()
            .flex()
            .flex_col()
            .child(self.row(&k, t("app.settings.language"), None, div().w(px(260.0)).child(Select::new(&self.language).small())))
            .child(self.row(&k, t("app.settings.appearance"), None, appearance))
            .child(self.row(
                &k,
                t("app.settings.output"),
                Some(t("app.settings.output_help")),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(out_seg)
                    .when(s.output_location == OutputLocation::Folder, |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(ui::mono().flex_1().min_w(px(0.0)).text_ellipsis_start().whitespace_nowrap().overflow_hidden().child(folder.display().to_string()))
                                .child(Button::new("out-folder").small().label(t("app.common.choose")).on_click(cx.listener(|p, _, window, cx| p.choose_output_folder(window, cx)))),
                        )
                    }),
            ))
            .child(self.row(
                &k,
                t("app.settings.reveal"),
                None,
                Switch::new("reveal").checked(s.reveal_when_done).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    p.set(cx, |s| s.reveal_when_done = v)
                })),
            ));

        // 2. Menu bar / tray
        let hotkey = if self.recording { t("app.settings.hotkey_press") } else { hotkey_label(&s.global_hotkey).into() };
        let tray = div()
            .flex()
            .flex_col()
            .child(self.row(
                &k,
                if mac { t("app.settings.tray_mac") } else { t("app.settings.tray_win") },
                Some(t("app.settings.tray_help")),
                Switch::new("tray").checked(s.tray_icon).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    p.set(cx, |s| s.tray_icon = v);
                    crate::tray::sync(cx);
                })),
            ))
            .child(self.row(
                &k,
                t("app.settings.keep_running"),
                Some(t("app.settings.keep_running_help")),
                Switch::new("keep-running").checked(s.keep_running_in_tray).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    p.set(cx, |s| s.keep_running_in_tray = v)
                })),
            ))
            .child(self.row(
                &k,
                t("app.settings.hotkey"),
                Some(t("app.settings.hotkey_help")),
                div()
                    .id("hotkey-recorder")
                    .track_focus(&self.record_focus)
                    .on_key_down(cx.listener(Self::on_record_key))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        ui::sans()
                            .min_w(px(150.0))
                            .px(px(10.0))
                            .py(px(3.0))
                            .border_1()
                            .rounded(px(3.0))
                            .border_color(if self.recording { k.link } else { k.rule })
                            .bg(k.raised)
                            .text_color(if self.recording { k.link } else { k.ink })
                            .child(hotkey),
                    )
                    .child(Button::new("record-hotkey").small().label(t("app.settings.hotkey_record")).on_click(cx.listener(|p, _, window, cx| {
                        p.recording = true;
                        p.record_focus.focus(window, cx);
                        cx.notify();
                    })))
                    .when(!s.global_hotkey.is_empty(), |d| {
                        d.child(Button::new("clear-hotkey").small().ghost().label(t("app.settings.hotkey_clear")).on_click(cx.listener(|p, _, _, cx| {
                            p.set(cx, |s| s.global_hotkey.clear());
                            crate::tray::register_hotkey(cx);
                        })))
                    }),
            ))
            .child(self.row(
                &k,
                t("app.settings.login"),
                self.login_error.clone().map(SharedString::from),
                Switch::new("login").checked(s.launch_at_login).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    match set_launch_at_login(v) {
                        Ok(()) => {
                            p.login_error = None;
                            p.set(cx, |s| s.launch_at_login = v);
                        }
                        Err(e) => {
                            p.login_error = Some(format!("{e:#}"));
                            cx.notify();
                        }
                    }
                })),
            ));

        // 3. Models & helpers
        let models: AnyElement = if self.assets.is_empty() {
            ui::roman().text_color(k.muted).italic().child(t("app.settings.models_none")).into_any_element()
        } else {
            let total: u64 = self.assets.iter().filter(|(_, p)| *p).map(|(a, _)| a.size).sum();
            div()
                .flex()
                .flex_col()
                .child(ui::caption(&k, format!("{} 1", castor_core::tr("app.caption.table")), "—", ta("app.settings.models_caption", &[("size", &units::bytes(total))])))
                .child(div().h(px(6.0)))
                .child(ui::rule_strong(&k))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .h(px(30.0))
                        .items_center()
                        .gap(px(12.0))
                        .child(ui::sans().flex_1().font_weight(FontWeight::BOLD).child(t("app.table.name")))
                        .child(ui::sans().w(px(110.0)).font_weight(FontWeight::BOLD).child(t("app.settings.licence")))
                        .child(ui::sans().w(px(80.0)).text_right().font_weight(FontWeight::BOLD).child(t("app.table.size")))
                        .child(ui::sans().w(px(160.0)).font_weight(FontWeight::BOLD).pl(px(10.0)).child(t("app.table.status"))),
                )
                .child(ui::hrule(k.rule, 1.0))
                .children(self.assets.iter().enumerate().map(|(i, (a, present))| {
                    div()
                        .flex()
                        .flex_row()
                        .h(px(34.0))
                        .items_center()
                        .gap(px(12.0))
                        .child(ui::mono().flex_1().min_w(px(0.0)).truncate().child(a.id.to_string()))
                        .child(ui::sans().w(px(110.0)).text_color(k.muted).child(a.license.to_string()))
                        .child(ui::sans().w(px(80.0)).text_right().child(if a.size > 0 { units::bytes(a.size) } else { "—".into() }))
                        .child(
                            div()
                                .w(px(160.0))
                                .pl(px(10.0))
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap(px(8.0))
                                .child(ui::sans().text_color(if *present { k.good } else { k.muted }).child(if *present { t("app.settings.present") } else { t("app.settings.absent") }))
                                .when(*present, |d| {
                                    d.child(Button::new(("del-asset", i)).xsmall().danger().label(t("app.common.delete")).on_click(cx.listener(move |p, _, window, cx| p.delete_asset(i, window, cx))))
                                }),
                        )
                }))
                .child(ui::rule_strong(&k))
                .into_any_element()
        };

        // 4. Updates
        let status: SharedString = match &update {
            UpdateState::Disabled => t("app.update.disabled"),
            UpdateState::Idle => t("app.update.idle"),
            UpdateState::Checking => t("app.update.checking"),
            UpdateState::UpToDate => t("app.update.up_to_date"),
            UpdateState::Available { version, .. } => ta("app.update.available", &[("version", version)]),
            UpdateState::Downloading { version, frac } => {
                ta("app.update.downloading", &[("version", version), ("pct", &format!("{:.0}", frac * 100.0))])
            }
            UpdateState::Ready { version } => ta("app.update.ready", &[("version", version)]),
            UpdateState::Failed(e) => ta("app.update.failed", &[("error", e)]),
        };
        let updates = div()
            .flex()
            .flex_col()
            .child(self.row(&k, t("app.settings.version"), None, ui::sans().child(format!("Castor {version}"))))
            .child(self.row(
                &k,
                t("app.settings.check_updates"),
                Some(t("app.settings.check_updates_help")),
                Switch::new("auto-check").checked(s.check_updates).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    p.set(cx, |s| s.check_updates = v)
                })),
            ))
            .child(self.row(
                &k,
                t("app.settings.auto_install"),
                Some(t("app.settings.auto_install_help")),
                Switch::new("auto-install").checked(s.auto_install_updates).on_change(cx.listener(|p, v: &bool, _, cx| {
                    let v = *v;
                    p.set(cx, |s| s.auto_install_updates = v)
                })),
            ))
            .child(self.row(
                &k,
                t("app.settings.update_status"),
                None,
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(10.0))
                    .child(ui::roman().text_color(k.ink).child(status))
                    .when(!matches!(update, UpdateState::Disabled | UpdateState::Checking | UpdateState::Downloading { .. }), |d| {
                        d.child(Button::new("check-now").small().label(t("app.settings.check_now")).on_click(|_, _, cx| crate::updates::check_now(cx)))
                    })
                    .when(matches!(update, UpdateState::Available { .. }), |d| {
                        d.child(Button::new("download-update").small().primary().label(t("app.update.download")).on_click(|_, _, cx| crate::updates::download(cx)))
                    })
                    .when(matches!(update, UpdateState::Ready { .. }), |d| {
                        d.child(Button::new("restart-update").small().primary().label(t("app.update.restart")).on_click(|_, _, cx| crate::updates::restart(cx)))
                    }),
            ));

        // 5. Privacy
        let has_key = s.ai_api_key.as_deref().is_some_and(|k| !k.is_empty());
        let ai_key = self.ai_key.clone();
        let privacy = div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(ui::roman().max_w(px(680.0)).text_color(k.ink).child(t("app.settings.privacy_body")))
            .child(self.row(
                &k,
                t("app.settings.ai_key"),
                Some(t("app.settings.ai_key_help")),
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().w(px(280.0)).child(Input::new(&self.ai_key).small().mask_toggle()))
                    .child(Button::new("save-key").small().label(t("app.common.save")).on_click(cx.listener(move |p, _, window, cx| {
                        let v = ai_key.read(cx).value().trim().to_string();
                        p.set(cx, |s| s.ai_api_key = if v.is_empty() { None } else { Some(v) });
                        window.push_notification(t("app.settings.ai_key_saved").to_string(), cx);
                    })))
                    .when(has_key, |d| {
                        d.child(Button::new("forget-key").small().danger().label(t("app.settings.ai_key_forget")).on_click(cx.listener(|p, _, window, cx| {
                            p.set(cx, |s| s.ai_api_key = None);
                            p.ai_key.update(cx, |i, cx| i.set_value("", window, cx));
                        })))
                    }),
            ));

        // 6. About
        let notices = castor_core::paths::resources_dir().join("THIRD_PARTY.md");
        let about = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(ui::roman().text_color(k.ink).child(ta("app.settings.about_line", &[("version", version)])))
            .child(ui::roman().text_color(k.ink).child(t("app.settings.about_licence")))
            .child(ui::roman().italic().text_color(k.muted).child(t("app.settings.about_joke")))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(16.0))
                    .pt(px(6.0))
                    .child(ui::link("about-site", &k, "basedpolymer.eu/castor", |_, _, cx| cx.open_url("https://basedpolymer.eu/castor")))
                    .child(ui::link("about-gh", &k, "GitHub", |_, _, cx| cx.open_url("https://github.com/basedpolymer/castor")))
                    .when(notices.is_file(), |d| {
                        let p = notices.clone();
                        d.child(ui::link("about-notices", &k, t("app.settings.notices"), move |_, _, _| {
                            let _ = castor_core::process::open(&p);
                        }))
                    })
                    .child(ui::link("about-logs", &k, t("app.settings.logs"), |_, _, _| {
                        let _ = castor_core::process::open(&castor_core::paths::data_dir().join("logs"));
                    })),
            );

        let section = |n: usize, title: SharedString, body: AnyElement| {
            div().flex().flex_col().gap(px(8.0)).child(ui::section_title(&k, n, title)).child(div().pl(px(2.0)).child(body))
        };
        let _ = window;

        div().id("settings-page").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(
            div()
                .px(px(PAGE_PAD))
                .pb(px(60.0))
                .max_w(px(1100.0))
                .flex()
                .flex_col()
                .gap(px(30.0))
                .child(ui::page_header(cx, t("app.nav.settings"), None, Some(t("app.settings.subtitle").into_any_element())))
                .child(section(1, t("app.settings.general"), general.into_any_element()))
                .child(section(2, t("app.settings.workspaces"), self.workspaces.clone().into_any_element()))
                .child(section(3, if mac { t("app.settings.menu_bar") } else { t("app.settings.system_tray") }, tray.into_any_element()))
                .child(section(4, t("app.settings.integration"), self.integration.clone().into_any_element()))
                .child(section(5, t("app.settings.models"), div().flex().flex_col().gap(px(8.0)).child(ui::roman().text_color(k.muted).text_size(px(14.0)).child(t("app.settings.models_help"))).child(models).into_any_element()))
                .child(section(6, t("app.settings.updates"), updates.into_any_element()))
                .child(section(7, t("app.settings.privacy"), privacy.into_any_element()))
                .child(section(8, t("app.settings.about"), about.into_any_element())),
        )
    }
}
