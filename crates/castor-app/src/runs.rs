//! Runs page (DESIGN §4): "Table 1 — Runs" grouped by day, filters, details, run again, undo,
//! clear history.

use castor_core::history::{RunRecord, RunStatus};
use castor_core::units;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, WindowExt as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::home::PAGE_PAD;
use crate::model::{Nav, model, navigate};
use crate::theme::{Tokens, tokens};
use crate::tool_screen::summary_line;
use crate::ui::{self, t, ta};

const ROW_H: f32 = 34.0;
const DETAILS_W: f32 = 360.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Complete,
    Failed,
    Cancelled,
}

impl Filter {
    fn key(self) -> &'static str {
        match self {
            Filter::All => "all",
            Filter::Complete => "complete",
            Filter::Failed => "failed",
            Filter::Cancelled => "cancelled",
        }
    }
    pub fn accepts(self, s: RunStatus) -> bool {
        match self {
            Filter::All => true,
            Filter::Complete => matches!(s, RunStatus::Complete),
            Filter::Failed => matches!(s, RunStatus::Failed | RunStatus::Partial),
            Filter::Cancelled => matches!(s, RunStatus::Cancelled),
        }
    }
}

#[derive(Clone)]
enum Row {
    Day(String),
    Run(usize),
}

pub struct RunsPage {
    filter: Filter,
    /// Only the runs of the active workspace.
    ws_only: bool,
    selected: Option<String>,
    scroll: UniformListScrollHandle,
    busy: bool,
    _subs: Vec<Subscription>,
}

/// Local date key and label for a unix time.
fn day_of(ts: i64) -> (time::Date, String) {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let dt = time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH).to_offset(offset);
    let today = time::OffsetDateTime::now_utc().to_offset(offset).date();
    let d = dt.date();
    let label = if d == today {
        castor_core::tr("app.runs.today")
    } else if Some(d) == today.previous_day() {
        castor_core::tr("app.runs.yesterday")
    } else {
        format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
    };
    (d, label)
}

fn time_of(ts: i64) -> String {
    let offset = time::UtcOffset::current_local_offset().unwrap_or(time::UtcOffset::UTC);
    let dt = time::OffsetDateTime::from_unix_timestamp(ts).unwrap_or(time::OffsetDateTime::UNIX_EPOCH).to_offset(offset);
    format!("{:02}:{:02}", dt.hour(), dt.minute())
}

pub fn status_label(s: RunStatus) -> SharedString {
    t(match s {
        RunStatus::Complete => "app.status.complete",
        RunStatus::Partial => "app.status.partial",
        RunStatus::Failed => "app.status.failed",
        RunStatus::Cancelled => "app.status.cancelled",
        RunStatus::Running => "app.status.running",
    })
}

pub fn status_color(k: &Tokens, s: RunStatus) -> Hsla {
    match s {
        RunStatus::Complete => k.good,
        RunStatus::Partial => k.warn,
        RunStatus::Failed => k.bad,
        _ => k.muted,
    }
}

/// Display name of a run: the tool name, or the chain name.
pub fn run_title(r: &RunRecord) -> String {
    if let Some(c) = &r.chain_name {
        return c.clone();
    }
    castor_core::tr(&format!("{}.name", r.tool_id))
}

impl RunsPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let m = model(cx);
        let subs = vec![cx.observe(&m, |_, _, cx| cx.notify())];
        RunsPage { filter: Filter::All, ws_only: true, selected: None, scroll: UniformListScrollHandle::new(), busy: false, _subs: subs }
    }

    fn clear_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.open_alert_dialog(cx, |alert, _, _| {
            alert
                .title(t("app.runs.clear_title"))
                .description(t("app.runs.clear_body"))
                .show_cancel(true)
                .ok_text(t("app.runs.clear_ok"))
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(t("common.cancel"))
                .on_ok(|_, _, cx| {
                    let _ = castor_core::history::clear();
                    model(cx).update(cx, |m, cx| {
                        m.history.clear();
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn undo(&mut self, r: RunRecord, window: &mut Window, cx: &mut Context<Self>) {
        self.busy = true;
        cx.notify();
        // Registry tools and the studios' own tools (an overwrite from PDF Studio).
        let tools: std::collections::HashMap<String, std::sync::Arc<dyn castor_core::Tool>> =
            r.undo.iter().filter_map(|(id, _)| crate::studio::find_tool(id, cx).map(|t| (id.clone(), t))).collect();
        cx.spawn_in(window, async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let mut errors = Vec::new();
                    // Undo in reverse order.
                    for (tool_id, journal) in r.undo.iter().rev() {
                        match tools.get(tool_id) {
                            Some(tool) => {
                                if let Err(e) = tool.undo(journal) {
                                    errors.push(format!("{e:#}"));
                                }
                            }
                            None => errors.push(format!("unknown tool {tool_id}")),
                        }
                    }
                    let mut r = r;
                    if errors.is_empty() {
                        r.undone = true;
                        let _ = castor_core::history::update(&r);
                    }
                    (r, errors)
                })
                .await;
            let _ = this.update_in(cx, |p, window, cx| {
                p.busy = false;
                let (r, errors) = res;
                if errors.is_empty() {
                    window.push_notification(t("app.runs.undone").to_string(), cx);
                    model(cx).update(cx, |m, cx| {
                        if let Some(x) = m.history.iter_mut().find(|x| x.id == r.id) {
                            *x = r;
                        }
                        cx.notify();
                    });
                } else {
                    window.push_notification(format!("{} {}", t("app.runs.undo_failed"), errors.join("; ")), cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn run_again(&self, r: &RunRecord, cx: &mut App) {
        let files: Vec<std::path::PathBuf> = r.inputs.iter().filter(|p| p.exists()).cloned().collect();
        if let Some(c) = &r.chain_name {
            navigate(Nav::Chain(c.clone()), cx);
            return;
        }
        // A studio save: reopen the studio on the original file.
        if r.tool_id.starts_with("studio.") {
            let kind = if r.tool_id == "studio.pdf" { crate::studio::StudioKind::Pdf } else { crate::studio::StudioKind::Media };
            navigate(Nav::Studio(kind, files.into_iter().next()), cx);
            return;
        }
        navigate(Nav::Tool { id: r.tool_id.clone(), files, values: Some(r.values.clone()) }, cx);
    }

    fn render_details(&self, k: &Tokens, r: &RunRecord, cx: &mut Context<Self>) -> impl IntoElement {
        let reg = crate::model::registry(cx);
        let can_undo = !r.undo.is_empty() && !r.undone;
        let can_rerun = r.chain_name.is_some() || reg.get(&r.tool_id).is_some() || r.tool_id.starts_with("studio.");
        let (_, day) = day_of(r.started);
        let r2 = r.clone();
        let r3 = r.clone();
        div()
            .id("run-details")
            .w(px(DETAILS_W))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .pl(px(20.0))
            .border_l_1()
            .border_color(k.rule)
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(ui::sans().text_size(px(17.0)).font_weight(FontWeight::BOLD).child(run_title(r)))
            .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child(format!("{day} · {}", time_of(r.started))))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .child(ui::sans().font_weight(FontWeight::BOLD).text_color(status_color(k, r.status)).child(status_label(r.status)))
                    .when(r.undone, |d| d.child(ui::tag(k, t("app.runs.undone_tag"), k.surface))),
            )
            .child(ui::roman().text_color(k.ink).child(summary_line(r)))
            .when(r.steps.len() > 1, |d| {
                d.child(
                    ui::sans()
                        .text_size(px(13.0))
                        .text_color(k.muted)
                        .child(r.steps.iter().map(|s| castor_core::tr(&format!("{s}.name"))).collect::<Vec<_>>().join(" → ")),
                )
            })
            .when(!r.errors.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .p(px(8.0))
                        .border_1()
                        .border_color(k.bad)
                        .rounded(px(3.0))
                        .children(r.errors.iter().take(20).map(|e| ui::mono().text_size(px(12.5)).text_color(k.ink).child(e.clone()))),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .when(can_rerun, |d| {
                        d.child(Button::new("run-again").small().primary().label(t("app.runs.run_again")).on_click(cx.listener(move |this, _, _, cx| this.run_again(&r2, cx))))
                    })
                    .when(can_undo, |d| {
                        d.child(
                            Button::new("undo-run")
                                .small()
                                .label(t("app.runs.undo"))
                                .loading(self.busy)
                                .on_click(cx.listener(move |this, _, window, cx| this.undo(r3.clone(), window, cx))),
                        )
                    }),
            )
            .when(!r.outputs.is_empty(), |d| {
                d.child(ui::caption(k, format!("{} 2", castor_core::tr("app.caption.table")), "—", ta("app.runs.outputs", &[("count", &r.outputs.len().to_string())])))
                    .child(ui::rule_strong(k))
                    .children(r.outputs.iter().take(200).enumerate().map(|(i, p)| {
                        let (p1, p2) = (p.clone(), p.clone());
                        let exists = p.exists();
                        div()
                            .h(px(30.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(6.0))
                            .child(ui::sans().flex_1().min_w(px(0.0)).text_size(px(13.5)).text_color(if exists { k.ink } else { k.faint }).truncate().child(p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()))
                            .when(exists, |d| {
                                d.child(Button::new(("out-open", i)).ghost().xsmall().tooltip(t("common.open")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("external-link"))).on_click(move |_, _, _| {
                                    let _ = castor_core::process::open(&p1);
                                }))
                                .child(Button::new(("out-reveal", i)).ghost().xsmall().tooltip(t("common.reveal")).icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide("folder-open"))).on_click(move |_, _, _| {
                                    let _ = castor_core::process::reveal(&p2);
                                }))
                            })
                    }))
                    .child(ui::rule_strong(k))
            })
    }
}

impl Render for RunsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let m = model(cx).read(cx);
        let loaded = m.history_loaded;
        let runs: Vec<RunRecord> = m.history.clone();
        let total = runs.len();
        let filter = self.filter;
        let active_ws = crate::workspaces::active(&m.settings);
        let ws_runs: Option<std::collections::HashSet<String>> =
            active_ws.as_ref().filter(|_| self.ws_only).map(|w| w.runs.iter().cloned().collect());
        let mut rows: Vec<Row> = Vec::new();
        let mut last_day = None;
        for (i, r) in runs.iter().enumerate() {
            if !filter.accepts(r.status) || ws_runs.as_ref().is_some_and(|set| !set.contains(&r.id)) {
                continue;
            }
            let (d, label) = day_of(r.started);
            if last_day != Some(d) {
                last_day = Some(d);
                rows.push(Row::Day(label));
            }
            rows.push(Row::Run(i));
        }
        let selected = self.selected.clone().and_then(|id| runs.iter().find(|r| r.id == id).cloned());
        let sel_id = self.selected.clone();
        let this = cx.entity().downgrade();
        let reg = crate::model::registry(cx);
        let runs_rc = std::rc::Rc::new(runs);
        let rows_rc = std::rc::Rc::new(rows);
        let n = rows_rc.len();
        let (runs2, rows2) = (runs_rc.clone(), rows_rc.clone());
        let list = uniform_list("runs-rows", n, move |range, _, _cx| {
            range
                .map(|i| match &rows2[i] {
                    Row::Day(label) => div()
                        .id(("runs-day", i))
                        .w_full()
                        .h(px(ROW_H))
                        .flex()
                        .items_end()
                        .pb(px(4.0))
                        .child(ui::sans().font_weight(FontWeight::BOLD).text_color(k.ink).child(label.clone()))
                        .into_any_element(),
                    Row::Run(ri) => {
                        let r = &runs2[*ri];
                        let sel = sel_id.as_deref() == Some(r.id.as_str());
                        let id = r.id.clone();
                        let this = this.clone();
                        let family = if r.steps.len() == 1 { r.steps.first().and_then(|s| reg.get(s)).map(|t| t.meta().family) } else { None };
                        // A size change only means something when each file gave one file (a
                        // checksum list or a merge is not a "−100 %" saving).
                        let comparable = r.files_out == r.files_in.max(r.inputs.len()) && r.files_out > 0;
                        let change = if comparable && r.bytes_in > 0 && r.bytes_out > 0 { units::change(r.bytes_in, r.bytes_out) } else { String::new() };
                        div()
                            .id(("runs-row", i))
                            .w_full()
                            .h(px(ROW_H))
                            .px(px(6.0))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.0))
                            .cursor_pointer()
                            .when(sel, |d| d.bg(k.surface))
                            .hover(|s| s.bg(k.surface))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                let _ = this.update(cx, |p, cx| {
                                    p.selected = Some(id);
                                    cx.notify();
                                });
                            })
                            .child(ui::sans().w(px(44.0)).text_color(k.muted).child(time_of(r.started)))
                            .child(match family {
                                Some(f) => ui::family_swatch(&k, f, 10.0).into_any_element(),
                                None => ui::icon("link", k.muted).size(px(12.0)).into_any_element(),
                            })
                            .child(ui::sans().flex_1().min_w(px(0.0)).text_color(k.ink).truncate().child(run_title(r)))
                            .child(ui::sans().w(px(70.0)).text_right().text_color(k.muted).child(r.files_in.max(r.inputs.len()).to_string()))
                            .child(ui::sans().w(px(80.0)).text_right().text_color(if r.bytes_out < r.bytes_in { k.good } else { k.muted }).child(change))
                            .child(ui::sans().w(px(70.0)).text_right().text_color(k.muted).child(units::duration(r.duration_secs as f64)))
                            .child(ui::sans().w(px(110.0)).pl(px(10.0)).text_color(status_color(&k, r.status)).child(status_label(r.status)))
                            .into_any_element()
                    }
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h(px(0.0));

        let this2 = cx.entity().downgrade();
        let filters = ui::segmented(
            "runs-filter",
            &k,
            [Filter::All, Filter::Complete, Filter::Failed, Filter::Cancelled]
                .iter()
                .map(|f| (f.key().to_string(), t(&format!("app.runs.filter.{}", f.key()))))
                .collect(),
            filter.key(),
            std::rc::Rc::new(move |v, _, cx| {
                let f = match v {
                    "complete" => Filter::Complete,
                    "failed" => Filter::Failed,
                    "cancelled" => Filter::Cancelled,
                    _ => Filter::All,
                };
                let _ = this2.update(cx, |p, cx| {
                    p.filter = f;
                    cx.notify();
                });
            }),
        );

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(30.0))
            .px(px(6.0))
            .gap(px(12.0))
            .child(ui::sans().w(px(44.0)).font_weight(FontWeight::BOLD).child(t("app.runs.col.time")))
            .child(div().w(px(10.0)))
            .child(ui::sans().flex_1().font_weight(FontWeight::BOLD).child(t("app.runs.col.tool")))
            .child(ui::sans().w(px(70.0)).text_right().font_weight(FontWeight::BOLD).child(t("app.runs.col.files")))
            .child(ui::sans().w(px(80.0)).text_right().font_weight(FontWeight::BOLD).child(t("app.runs.col.size")))
            .child(ui::sans().w(px(70.0)).text_right().font_weight(FontWeight::BOLD).child(t("app.runs.col.duration")))
            .child(ui::sans().w(px(110.0)).pl(px(10.0)).font_weight(FontWeight::BOLD).child(t("app.table.status")));

        let body: AnyElement = if total == 0 {
            ui::roman()
                .py(px(30.0))
                .italic()
                .text_color(k.muted)
                .child(if loaded { t("app.runs.empty") } else { t("app.runs.loading") })
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h(px(0.0))
                .gap(px(20.0))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .child(ui::caption(&k, format!("{} 1", castor_core::tr("app.caption.table")), "—", t("app.runs.caption")))
                        .child(div().h(px(6.0)))
                        .child(ui::rule_strong(&k))
                        .child(header)
                        .child(ui::hrule(k.rule, 1.0))
                        .child(list)
                        .child(ui::rule_strong(&k)),
                )
                .when_some(selected, |d, r| d.child(self.render_details(&k, &r, cx)))
                .into_any_element()
        };

        div()
            .id("runs-page")
            .size_full()
            .flex()
            .flex_col()
            .px(px(PAGE_PAD))
            .pb(px(20.0))
            .gap(px(18.0))
            .child(ui::page_header(cx, t("app.nav.runs"), Some(total.to_string()), Some(t("app.runs.subtitle").into_any_element())))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .child(filters)
                    .when_some(active_ws.clone(), |d, w| {
                        let this3 = cx.entity().downgrade();
                        d.child(ui::segmented(
                            "runs-ws",
                            &k,
                            vec![("ws".into(), SharedString::from(w.name.clone())), ("all".into(), t("app.runs.all_workspaces"))],
                            if self.ws_only { "ws" } else { "all" },
                            std::rc::Rc::new(move |v, _, cx| {
                                let only = v == "ws";
                                let _ = this3.update(cx, |p, cx| {
                                    p.ws_only = only;
                                    cx.notify();
                                });
                            }),
                        ))
                    })
                    .child(div().flex_1())
                    .when(total > 0, |d| {
                        d.child(Button::new("clear-history").small().danger().label(t("app.runs.clear")).on_click(cx.listener(|this, _, window, cx| this.clear_history(window, cx))))
                    }),
            )
            .child(body)
    }
}
