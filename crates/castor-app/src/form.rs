//! The generated options form (DESIGN §6): one control per `OptionSpec`, `visible_if`,
//! "More options" for `advanced`. Used by the tool screen and by every chain step.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use castor_core::options::{Anchor, Choice, OptionKind, OptionSpec, Value, Values};
use castor_core::Tool;
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::color_picker::{ColorPickerEvent, ColorPickerState, ColorSelect};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::searchable_list::{SearchableListItem, SearchableVec};
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{IndexPath, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::component::Disableable as _;
use gpui_kit::*;

use crate::theme::{Tokens, fonts, tokens};
use crate::ui::{self, t};

/// An item of a generated select.
#[derive(Clone, Debug)]
pub struct Opt {
    pub label: SharedString,
    pub value: String,
    /// Render the label in this font (font pickers).
    pub font: Option<SharedString>,
}

impl SearchableListItem for Opt {
    type Value = String;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &String {
        &self.value
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div().when_some(self.font.clone(), |d, f| d.font_family(f)).child(self.label.clone())
    }
    fn matches(&self, query: &str) -> bool {
        let q = castor_core::registry::fold(query);
        castor_core::registry::fold(&self.label).contains(&q) || self.value.to_lowercase().contains(&q)
    }
}

type OptSelect = SelectState<SearchableVec<Opt>>;

enum Control {
    /// Rendered straight from the value (segmented choice, toggle, anchor, path).
    Plain,
    Input(Entity<InputState>),
    Textarea(Entity<TextareaState>),
    Select(Entity<OptSelect>),
    Slider(Entity<SliderState>, Entity<InputState>),
    Color(Entity<ColorPickerState>),
    Size(Entity<InputState>, Entity<InputState>),
}

pub enum FormEvent {
    Changed,
}

pub struct OptionsForm {
    #[allow(dead_code)]
    tool: Arc<dyn Tool>,
    id: String,
    specs: Vec<OptionSpec>,
    values: Values,
    controls: HashMap<&'static str, Control>,
    show_advanced: bool,
    locked: HashMap<&'static str, bool>,
    disabled: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<FormEvent> for OptionsForm {}

/// Display label of an option.
pub fn label(spec: &OptionSpec, tool_id: &str) -> String {
    castor_core::tr(&spec.label_key(tool_id))
}

/// Help text under the control, if the locale has one.
pub fn help(spec: &OptionSpec, tool_id: &str) -> Option<String> {
    let k = format!("{}.help", spec.label_key(tool_id));
    castor_core::i18n::has(&k).then(|| castor_core::tr(&k))
}

fn num_text(v: &Value) -> String {
    match v {
        Value::Int(n) => n.to_string(),
        Value::Float(f) => {
            let s = format!("{f:.3}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Value::Text(s) => s.clone(),
        _ => String::new(),
    }
}

pub fn color_to_hex(c: Hsla) -> String {
    let r = Rgba::from(c);
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}{:02x}", b(r.r), b(r.g), b(r.b), b(r.a))
}

pub fn hex_to_color(s: &str) -> Option<Hsla> {
    let [r, g, b, a] = castor_core::options::parse_hex_color(s)?;
    Some(Rgba { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }.into())
}

impl OptionsForm {
    pub fn new(tool: Arc<dyn Tool>, values: Values, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let specs = tool.options();
        let id = tool.meta().id.to_string();
        let mut f = OptionsForm {
            tool,
            id,
            specs: specs.clone(),
            values,
            controls: HashMap::new(),
            show_advanced: false,
            locked: HashMap::new(),
            disabled: false,
            _subs: Vec::new(),
        };
        for spec in &specs {
            if !f.values.0.contains_key(spec.key) {
                f.values.set(spec.key, spec.default.clone());
            }
            let c = f.build_control(spec, window, cx);
            f.controls.insert(spec.key, c);
        }
        f
    }

    pub fn values(&self) -> &Values {
        &self.values
    }

    pub fn set_disabled(&mut self, d: bool, cx: &mut Context<Self>) {
        self.disabled = d;
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(FormEvent::Changed);
        cx.notify();
    }

    fn set(&mut self, key: &str, v: Value, cx: &mut Context<Self>) {
        self.values.set(key, v);
        self.changed(cx);
    }

    fn input(&mut self, text: String, placeholder: Option<SharedString>, masked: bool, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        cx.new(|cx| {
            let mut s = InputState::new(window, cx).default_value(text).masked(masked);
            if let Some(p) = placeholder {
                s = s.placeholder(p);
            }
            s
        })
    }

    fn build_control(&mut self, spec: &OptionSpec, window: &mut Window, cx: &mut Context<Self>) -> Control {
        let key = spec.key;
        let cur = self.values.get(key).cloned().unwrap_or_else(|| spec.default.clone());
        match &spec.kind {
            OptionKind::Choice(cs) if cs.len() > 4 => {
                let items: Vec<Opt> = cs
                    .iter()
                    .map(|c| Opt { label: spec.choice_label(&self.id, c).into(), value: c.value.to_string(), font: None })
                    .collect();
                let sel = items.iter().position(|o| Some(o.value.as_str()) == cur.as_str()).map(IndexPath::new);
                let searchable = items.len() > 10;
                let state = cx.new(|cx| SelectState::new(SearchableVec::from(items), sel, window, cx).searchable(searchable));
                self._subs.push(cx.subscribe(&state, move |this, _, ev: &SelectEvent<SearchableVec<Opt>>, cx| {
                    if let SelectEvent::Confirm(Some(v)) = ev {
                        this.set(key, Value::Text(v.clone()), cx);
                    }
                }));
                Control::Select(state)
            }
            OptionKind::Font => {
                let mut names = cx.text_system().all_font_names();
                names.sort();
                names.dedup();
                let items: Vec<Opt> = names
                    .into_iter()
                    .filter(|n| !n.starts_with('.') && !n.starts_with('@'))
                    .map(|n| Opt { label: n.clone().into(), value: n.clone(), font: Some(n.into()) })
                    .collect();
                let sel = items.iter().position(|o| Some(o.value.as_str()) == cur.as_str()).map(IndexPath::new);
                let state = cx.new(|cx| SelectState::new(SearchableVec::from(items), sel, window, cx).searchable(true));
                self._subs.push(cx.subscribe(&state, move |this, _, ev: &SelectEvent<SearchableVec<Opt>>, cx| {
                    if let SelectEvent::Confirm(Some(v)) = ev {
                        this.set(key, Value::Text(v.clone()), cx);
                    }
                }));
                Control::Select(state)
            }
            OptionKind::Language { list, multi } => {
                let items: Vec<Opt> =
                    list.iter().map(|(code, name)| Opt { label: (*name).into(), value: code.to_string(), font: None }).collect();
                let first = cur.as_str().unwrap_or("").split('+').next().unwrap_or("").to_string();
                let sel = if *multi { None } else { items.iter().position(|o| o.value == first).map(IndexPath::new) };
                let multi = *multi;
                let state = cx.new(|cx| SelectState::new(SearchableVec::from(items), sel, window, cx).searchable(true));
                self._subs.push(cx.subscribe(&state, move |this, _, ev: &SelectEvent<SearchableVec<Opt>>, cx| {
                    if let SelectEvent::Confirm(Some(v)) = ev {
                        if multi {
                            let mut codes: Vec<String> =
                                this.values.str(key).split('+').filter(|s| !s.is_empty()).map(String::from).collect();
                            if !codes.contains(v) {
                                codes.push(v.clone());
                            }
                            this.set(key, Value::Text(codes.join("+")), cx);
                        } else {
                            this.set(key, Value::Text(v.clone()), cx);
                        }
                    }
                }));
                Control::Select(state)
            }
            OptionKind::Int { min, max, step, slider, .. } => {
                let text = num_text(&cur);
                let input = self.input(text, None, false, window, cx);
                let (min, max) = (*min, *max);
                if *slider {
                    let v = match cur {
                        Value::Int(n) => n as f32,
                        _ => min as f32,
                    };
                    let s = cx.new(|_| SliderState::new().min(min as f32).max(max as f32).step(*step as f32).default_value(v));
                    let inp = input.clone();
                    self._subs.push(cx.subscribe_in(&s, window, move |this, _, ev: &SliderEvent, window, cx| {
                        let SliderEvent::Change(v) = ev else { return };
                        let n = (v.start().round() as i64).clamp(min, max);
                        inp.update(cx, |i, cx| i.set_value(n.to_string(), window, cx));
                        this.set(key, Value::Int(n), cx);
                    }));
                    let s2 = s.clone();
                    self._subs.push(cx.subscribe_in(&input, window, move |this, st, ev: &InputEvent, window, cx| {
                        if let InputEvent::Change = ev {
                            if let Ok(n) = st.read(cx).value().trim().parse::<i64>() {
                                let n = n.clamp(min, max);
                                s2.update(cx, |s, cx| s.set_value(n as f32, window, cx));
                                this.set(key, Value::Int(n), cx);
                            }
                        }
                    }));
                    Control::Slider(s, input)
                } else {
                    self._subs.push(cx.subscribe(&input, move |this, st, ev: &InputEvent, cx| {
                        if let InputEvent::Change = ev {
                            if let Ok(n) = st.read(cx).value().trim().parse::<i64>() {
                                this.set(key, Value::Int(n.clamp(min, max)), cx);
                            }
                        }
                    }));
                    Control::Input(input)
                }
            }
            OptionKind::Float { min, max, step, slider, .. } => {
                let text = num_text(&cur);
                let input = self.input(text, None, false, window, cx);
                let (min, max) = (*min, *max);
                let parse = |s: &str| s.trim().replace(',', ".").parse::<f64>().ok();
                if *slider {
                    let v = match cur {
                        Value::Float(f) => f as f32,
                        Value::Int(n) => n as f32,
                        _ => min as f32,
                    };
                    let s = cx.new(|_| SliderState::new().min(min as f32).max(max as f32).step(*step as f32).default_value(v));
                    let inp = input.clone();
                    self._subs.push(cx.subscribe_in(&s, window, move |this, _, ev: &SliderEvent, window, cx| {
                        let SliderEvent::Change(v) = ev else { return };
                        let f = (v.start() as f64).clamp(min, max);
                        let f = (f * 1000.0).round() / 1000.0;
                        inp.update(cx, |i, cx| i.set_value(num_text(&Value::Float(f)), window, cx));
                        this.set(key, Value::Float(f), cx);
                    }));
                    let s2 = s.clone();
                    self._subs.push(cx.subscribe_in(&input, window, move |this, st, ev: &InputEvent, window, cx| {
                        if let InputEvent::Change = ev {
                            if let Some(f) = parse(&st.read(cx).value()) {
                                let f = f.clamp(min, max);
                                s2.update(cx, |s, cx| s.set_value(f as f32, window, cx));
                                this.set(key, Value::Float(f), cx);
                            }
                        }
                    }));
                    Control::Slider(s, input)
                } else {
                    self._subs.push(cx.subscribe(&input, move |this, st, ev: &InputEvent, cx| {
                        if let InputEvent::Change = ev {
                            if let Some(f) = parse(&st.read(cx).value()) {
                                this.set(key, Value::Float(f.clamp(min, max)), cx);
                            }
                        }
                    }));
                    Control::Input(input)
                }
            }
            OptionKind::Text { multiline: true, placeholder_key } => {
                let text = cur.as_str().unwrap_or("").to_string();
                let ph = placeholder_key.map(|k| SharedString::from(castor_core::tr(k)));
                let state = cx.new(|cx| {
                    let mut s = TextareaState::new(window, cx).auto_grow(3, 10).default_value(text);
                    if let Some(p) = ph {
                        s = s.placeholder(p);
                    }
                    s
                });
                self._subs.push(cx.subscribe(&state, move |this, st, ev: &InputEvent, cx| {
                    if let InputEvent::Change = ev {
                        let v = st.read(cx).value().to_string();
                        this.set(key, Value::Text(v), cx);
                    }
                }));
                Control::Textarea(state)
            }
            OptionKind::Text { multiline: false, placeholder_key } => {
                let ph = placeholder_key.map(|k| SharedString::from(castor_core::tr(k)));
                let input = self.input(cur.as_str().unwrap_or("").to_string(), ph, false, window, cx);
                self.text_sub(&input, key, cx);
                Control::Input(input)
            }
            OptionKind::Password => {
                let input = self.input(String::new(), None, true, window, cx);
                self.text_sub(&input, key, cx);
                Control::Input(input)
            }
            OptionKind::PageRange => {
                let input = self.input(cur.as_str().unwrap_or("").to_string(), Some(t("app.form.pages_placeholder")), false, window, cx);
                self.text_sub(&input, key, cx);
                Control::Input(input)
            }
            OptionKind::Color => {
                let c = hex_to_color(cur.as_str().unwrap_or("#000000ff")).unwrap_or(gpui_kit::black());
                let state = cx.new(|cx| ColorPickerState::new(window, cx).default_value(c));
                self._subs.push(cx.subscribe(&state, move |this, _, ev: &ColorPickerEvent, cx| {
                    let ColorPickerEvent::Change(Some(c)) = ev else { return };
                    this.set(key, Value::Text(color_to_hex(*c)), cx);
                }));
                Control::Color(state)
            }
            OptionKind::Size => {
                let (w, h) = match cur {
                    Value::Size(w, h) => (w, h),
                    _ => (0, 0),
                };
                let wi = self.input(w.to_string(), Some("W".into()), false, window, cx);
                let hi = self.input(h.to_string(), Some("H".into()), false, window, cx);
                let (wi2, hi2) = (wi.clone(), hi.clone());
                self._subs.push(cx.subscribe_in(&wi, window, move |this, st, ev: &InputEvent, window, cx| {
                    let InputEvent::Change = ev else { return };
                    let Ok(nw) = st.read(cx).value().trim().parse::<u32>() else { return };
                    let (ow, oh) = this.values.size(key);
                    let nh = if this.locked.get(key).copied().unwrap_or(false) && ow > 0 {
                        let nh = ((nw as f64) * oh as f64 / ow as f64).round() as u32;
                        hi2.update(cx, |i, cx| i.set_value(nh.to_string(), window, cx));
                        nh
                    } else {
                        oh
                    };
                    this.set(key, Value::Size(nw, nh), cx);
                }));
                self._subs.push(cx.subscribe_in(&hi, window, move |this, st, ev: &InputEvent, window, cx| {
                    let InputEvent::Change = ev else { return };
                    let Ok(nh) = st.read(cx).value().trim().parse::<u32>() else { return };
                    let (ow, oh) = this.values.size(key);
                    let nw = if this.locked.get(key).copied().unwrap_or(false) && oh > 0 {
                        let nw = ((nh as f64) * ow as f64 / oh as f64).round() as u32;
                        wi2.update(cx, |i, cx| i.set_value(nw.to_string(), window, cx));
                        nw
                    } else {
                        ow
                    };
                    this.set(key, Value::Size(nw, nh), cx);
                }));
                Control::Size(wi, hi)
            }
            OptionKind::Choice(_) | OptionKind::Toggle | OptionKind::Anchor | OptionKind::Path { .. } => Control::Plain,
        }
    }

    fn text_sub(&mut self, input: &Entity<InputState>, key: &'static str, cx: &mut Context<Self>) {
        self._subs.push(cx.subscribe(input, move |this, st, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let v = st.read(cx).value().to_string();
                this.set(key, Value::Text(v), cx);
            }
        }));
    }

    fn choose_path(&mut self, key: &'static str, folder: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: !folder, directories: folder, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                if let Some(p) = paths.pop() {
                    let _ = this.update(cx, |f, cx| f.set(key, Value::Path(Some(p)), cx));
                }
            }
        })
        .detach();
    }

    fn render_control(&self, spec: &OptionSpec, k: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let key = spec.key;
        let id = &self.id;
        let unit = |u: &Option<&'static str>| u.map(|u| ui::sans().text_size(px(13.0)).text_color(k.muted).child(u));
        match (&spec.kind, self.controls.get(key)) {
            (OptionKind::Choice(cs), _) if cs.len() <= 4 => {
                let items: Vec<(String, SharedString)> =
                    cs.iter().map(|c: &Choice| (c.value.to_string(), spec.choice_label(id, c).into())).collect();
                let cur = self.values.str(key).to_string();
                let this = cx.entity().downgrade();
                ui::segmented(
                    format!("seg-{id}-{key}"),
                    k,
                    items,
                    &cur,
                    std::rc::Rc::new(move |v, _, cx| {
                        let v = v.to_string();
                        let _ = this.update(cx, |f, cx| f.set(key, Value::Text(v), cx));
                    }),
                )
                .into_any_element()
            }
            (OptionKind::Toggle, _) => Checkbox::new(SharedString::from(format!("cb-{id}-{key}")))
                .label(label(spec, id))
                .checked(self.values.bool(key))
                .disabled(self.disabled)
                .on_change(cx.listener(move |f, v: &bool, _, cx| f.set(key, Value::Bool(*v), cx)))
                .into_any_element(),
            (OptionKind::Anchor, _) => {
                let cur = self.values.anchor(key);
                div()
                    .grid()
                    .grid_cols(3)
                    .w(px(84.0))
                    .gap(px(2.0))
                    .p(px(4.0))
                    .border_1()
                    .border_color(k.rule)
                    .rounded(px(3.0))
                    .bg(k.raised)
                    .children(Anchor::ALL.iter().map(|a| {
                        let sel = *a == cur;
                        let a2 = *a;
                        div()
                            .id(SharedString::from(format!("anchor-{id}-{key}-{}", a.as_str())))
                            .tab_index(0)
                            .size(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(2.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(k.surface))
                            .focus_visible(|s| s.border_1().border_color(k.link))
                            .on_click(cx.listener(move |f, _, _, cx| f.set(key, Value::Text(a2.as_str().into()), cx)))
                            .child(if sel { ui::diamond_filled(10.0, k.ink) } else { ui::diamond(10.0, k.faint) })
                            .test_support()
                    }))
                    .into_any_element()
            }
            (OptionKind::Path { folder, .. }, _) => {
                let folder = *folder;
                let p = self.values.path(key).map(|p| p.display().to_string());
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        ui::mono()
                            .flex_1()
                            .min_w(px(0.0))
                            .px(px(8.0))
                            .py(px(4.0))
                            .border_1()
                            .border_color(k.rule)
                            .rounded(px(3.0))
                            .bg(k.surface)
                            .text_color(if p.is_some() { k.ink } else { k.faint })
                            .text_ellipsis_start()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .child(p.clone().unwrap_or_else(|| castor_core::tr("app.form.no_file"))),
                    )
                    .when(p.is_some(), |d| {
                        d.child(
                            Button::new(SharedString::from(format!("clr-{id}-{key}")))
                                .ghost()
                                .small()
                                .icon(gpui_kit::component::IconName::Close)
                                .on_click(cx.listener(move |f, _, _, cx| f.set(key, Value::Path(None), cx))),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("choose-{id}-{key}")))
                            .small()
                            .label(t("app.common.choose"))
                            .on_click(cx.listener(move |f, _, window, cx| f.choose_path(key, folder, window, cx))),
                    )
                    .into_any_element()
            }
            (OptionKind::Int { unit: u, .. } | OptionKind::Float { unit: u, .. }, Some(Control::Slider(s, inp))) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(div().flex_1().child(Slider::new(s).disabled(self.disabled)))
                .child(div().w(px(64.0)).child(Input::new(inp).small().disabled(self.disabled)))
                .when_some(unit(u), |d, u| d.child(u))
                .into_any_element(),
            (OptionKind::Int { unit: u, .. } | OptionKind::Float { unit: u, .. }, Some(Control::Input(inp))) => div()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(div().w(px(110.0)).child(Input::new(inp).small().disabled(self.disabled)))
                .when_some(unit(u), |d, u| d.child(u))
                .into_any_element(),
            (OptionKind::Password, Some(Control::Input(inp))) => {
                Input::new(inp).small().mask_toggle().disabled(self.disabled).into_any_element()
            }
            (_, Some(Control::Input(inp))) => Input::new(inp).small().disabled(self.disabled).into_any_element(),
            (_, Some(Control::Textarea(ta))) => Textarea::new(ta).disabled(self.disabled).into_any_element(),
            (OptionKind::Language { multi: true, .. }, Some(Control::Select(sel))) => {
                let codes: Vec<String> = self.values.str(key).split('+').filter(|s| !s.is_empty()).map(String::from).collect();
                let list = match &spec.kind {
                    OptionKind::Language { list, .. } => *list,
                    _ => &[],
                };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(div().flex().flex_row().flex_wrap().gap(px(4.0)).children(codes.iter().map(|c| {
                        let name = list.iter().find(|(code, _)| code == c).map(|(_, n)| *n).unwrap_or(c.as_str()).to_string();
                        let c2 = c.clone();
                        div()
                            .id(SharedString::from(format!("chip-{id}-{key}-{c}")))
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(4.0))
                            .px(px(6.0))
                            .py(px(1.0))
                            .rounded(px(3.0))
                            .border_1()
                            .border_color(k.rule)
                            .bg(k.surface)
                            .child(ui::sans().text_size(px(13.0)).child(name))
                            .child(
                                div()
                                    .id(SharedString::from(format!("chipx-{id}-{key}-{c}")))
                                    .cursor_pointer()
                                    .on_click(cx.listener(move |f, _, _, cx| {
                                        let rest: Vec<String> =
                                            f.values.str(key).split('+').filter(|s| !s.is_empty() && *s != c2).map(String::from).collect();
                                        f.set(key, Value::Text(rest.join("+")), cx);
                                    }))
                                    .child(ui::icon("x", k.muted).size(px(12.0))),
                            )
                    })))
                    .child(Select::new(sel).small().placeholder(t("app.form.add_language")).disabled(self.disabled))
                    .into_any_element()
            }
            (_, Some(Control::Select(sel))) => Select::new(sel).small().disabled(self.disabled).into_any_element(),
            (_, Some(Control::Color(c))) => ColorSelect::new(c).small().into_any_element(),
            (_, Some(Control::Size(w, h))) => {
                let locked = self.locked.get(key).copied().unwrap_or(false);
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().w(px(80.0)).child(Input::new(w).small().disabled(self.disabled)))
                    .child(ui::sans().text_color(k.muted).child("×"))
                    .child(div().w(px(80.0)).child(Input::new(h).small().disabled(self.disabled)))
                    .child(ui::sans().text_size(px(13.0)).text_color(k.muted).child("px"))
                    .child(
                        Button::new(SharedString::from(format!("lock-{id}-{key}")))
                            .ghost()
                            .small()
                            .tooltip(t("app.form.lock_ratio"))
                            .icon(gpui_kit::component::Icon::empty().path(crate::assets::lucide(if locked { "lock" } else { "lock-open" })))
                            .on_click(cx.listener(move |f, _, _, cx| {
                                let l = f.locked.entry(key).or_insert(false);
                                *l = !*l;
                                cx.notify();
                            })),
                    )
                    .into_any_element()
            }
            _ => div().into_any_element(),
        }
    }

    fn render_field(&self, spec: &OptionSpec, k: &Tokens, cx: &mut Context<Self>) -> AnyElement {
        let is_toggle = matches!(spec.kind, OptionKind::Toggle);
        div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .when(!is_toggle, |d| {
                d.child(ui::sans().text_size(px(13.5)).text_color(k.ink).font_weight(FontWeight::BOLD).child(label(spec, &self.id)))
            })
            .child(self.render_control(spec, k, cx))
            .when_some(help(spec, &self.id), |d, h| {
                d.child(ui::roman().text_size(px(13.0)).line_height(px(16.5)).text_color(k.muted).child(h))
            })
            .into_any_element()
    }
}

impl Render for OptionsForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let visible: Vec<OptionSpec> =
            self.specs.iter().filter(|s| s.visible_if.as_ref().map(|c| c.eval(&self.values)).unwrap_or(true)).cloned().collect();
        let basic: Vec<AnyElement> = visible.iter().filter(|s| !s.advanced).map(|s| self.render_field(s, &k, cx)).collect();
        let adv: Vec<&OptionSpec> = visible.iter().filter(|s| s.advanced).collect();
        let n_adv = adv.len();
        let adv_fields: Vec<AnyElement> =
            if self.show_advanced { adv.into_iter().map(|s| self.render_field(s, &k, cx)).collect() } else { Vec::new() };
        let open = self.show_advanced;
        div()
            .flex()
            .flex_col()
            .gap(px(16.0))
            .when(basic.is_empty() && n_adv == 0, |d| {
                d.child(ui::roman().text_size(px(13.5)).text_color(k.muted).child(t("app.form.no_options")))
            })
            .children(basic)
            .when(n_adv > 0, |d| {
                d.child(
                    div()
                        .id(SharedString::from(format!("more-{}", self.id)))
                        .tab_index(0)
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .cursor_pointer()
                        .text_color(k.ink)
                        .focus_visible(|s| s.underline())
                        .on_click(cx.listener(|f, _, _, cx| {
                            f.show_advanced = !f.show_advanced;
                            cx.notify();
                        }))
                        .child(ui::icon(if open { "chevron-down" } else { "chevron-right" }, k.muted).size(px(14.0)))
                        .child(ui::sans().text_size(px(13.5)).child(t("app.form.more_options")))
                        .child(ui::sans().text_size(px(12.5)).text_color(k.muted).child(format!("({n_adv})"))),
                )
                .children(adv_fields)
            })
            .font_family(fonts().sans.clone())
    }
}

/// Values to persist for a tool: defaults merged with the edits, secrets removed.
pub fn persistable(tool: &dyn Tool, v: &Values) -> Values {
    v.without_secrets(&tool.options())
}

/// Path helper for tests.
#[allow(dead_code)]
pub fn path_value(p: &str) -> Value {
    Value::Path(Some(PathBuf::from(p)))
}
