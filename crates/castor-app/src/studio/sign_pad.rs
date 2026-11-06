//! "New signature" dialog body: draw it with the mouse or trackpad, type it (rendered in TeX
//! Gyre Chorus), or import an image. Saved signatures are shared with the Sign PDF tool.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::Sizable as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::pdf_engine as eng;
use crate::theme::{fonts, tokens};
use crate::ui::{self, t};

const PAD_W: f32 = 500.0;
const PAD_H: f32 = 170.0;
const INK: eng::Rgb = [0.106, 0.165, 0.42];

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Draw,
    Type,
    Import,
}

pub struct SignPad {
    kind: Kind,
    strokes: Vec<Vec<[f32; 2]>>,
    drawing: bool,
    origin: Rc<Cell<Point<Pixels>>>,
    name: Entity<InputState>,
    image: Option<PathBuf>,
    _subs: Vec<Subscription>,
}

impl SignPad {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(t("app.pdf.sign.type_ph")));
        let subs = vec![cx.observe(&name, |_, _, cx| cx.notify())];
        SignPad { kind: Kind::Draw, strokes: Vec::new(), drawing: false, origin: Rc::new(Cell::new(Point::default())), name, image: None, _subs: subs }
    }

    /// Save the signature into the signatures folder.
    pub fn save(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let res = match self.kind {
            Kind::Draw => {
                if self.strokes.iter().all(|s| s.len() < 2) {
                    return Err(t("app.pdf.sign.err.empty").to_string());
                }
                eng::save_drawn_signature(&self.strokes, PAD_W, PAD_H, INK)
            }
            Kind::Type => {
                let n = self.name.read(cx).value().trim().to_string();
                if n.is_empty() {
                    return Err(t("app.pdf.sign.err.name").to_string());
                }
                eng::save_typed_signature(&n)
            }
            Kind::Import => match &self.image {
                Some(p) => eng::import_signature(p),
                None => return Err(t("app.pdf.sign.err.image").to_string()),
            },
        };
        res.map(|_| ()).map_err(|e| format!("{e:#}"))
    }

    fn local(&self, p: Point<Pixels>) -> [f32; 2] {
        let o = self.origin.get();
        [(p.x - o.x).as_f32().clamp(0.0, PAD_W), (p.y - o.y).as_f32().clamp(0.0, PAD_H)]
    }

    fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = rx.await {
                let _ = this.update(cx, |s, cx| {
                    s.image = paths.pop();
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for SignPad {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let k = tokens(cx);
        let this = cx.entity().downgrade();
        let kind = self.kind;
        let origin = self.origin.clone();
        let strokes = self.strokes.clone();
        let area = div()
            .id("sign-area")
            .relative()
            .w(px(PAD_W))
            .h(px(PAD_H))
            .bg(gpui_kit::white())
            .border_1()
            .border_color(k.rule)
            .rounded(px(3.0))
            .overflow_hidden();
        let body: AnyElement = match kind {
            Kind::Draw => area
                .cursor(CursorStyle::Crosshair)
                .child(div().absolute().left(px(24.0)).right(px(24.0)).bottom(px(38.0)).h(px(1.0)).bg(k.rule))
                .child(
                    canvas(
                        move |b, _, _| origin.set(b.origin),
                        move |b, _, window, _| {
                            for s in &strokes {
                                if s.len() < 2 {
                                    continue;
                                }
                                let mut pb = PathBuilder::stroke(px(2.6));
                                pb.move_to(b.origin + point(px(s[0][0]), px(s[0][1])));
                                for p in &s[1..] {
                                    pb.line_to(b.origin + point(px(p[0]), px(p[1])));
                                }
                                if let Ok(path) = pb.build() {
                                    window.paint_path(path, Hsla::from(gpui_kit::Rgba { r: INK[0], g: INK[1], b: INK[2], a: 1.0 }));
                                }
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .on_mouse_down(MouseButton::Left, cx.listener(|s, ev: &MouseDownEvent, _, cx| {
                    s.drawing = true;
                    let p = s.local(ev.position);
                    s.strokes.push(vec![p]);
                    cx.notify();
                }))
                .on_mouse_move(cx.listener(|s, ev: &MouseMoveEvent, _, cx| {
                    if s.drawing && ev.pressed_button == Some(MouseButton::Left) {
                        let p = s.local(ev.position);
                        if let Some(st) = s.strokes.last_mut() {
                            st.push(p);
                        }
                        cx.notify();
                    }
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|s, _, _, _| s.drawing = false))
                .into_any_element(),
            Kind::Type => {
                let name = self.name.read(cx).value().to_string();
                area.flex()
                    .items_center()
                    .justify_center()
                    .font_family(fonts().chorus.clone())
                    .text_size(px(52.0))
                    .text_color(Hsla::from(gpui_kit::Rgba { r: INK[0], g: INK[1], b: INK[2], a: 1.0 }))
                    .child(if name.trim().is_empty() { ui::roman().text_color(k.faint).child(t("app.pdf.sign.type_preview")).into_any_element() } else { div().child(name).into_any_element() })
                    .into_any_element()
            }
            Kind::Import => area
                .flex()
                .items_center()
                .justify_center()
                .map(|d| match &self.image {
                    Some(p) => d.child(img(p.clone()).max_w(px(PAD_W - 20.0)).max_h(px(PAD_H - 20.0)).object_fit(ObjectFit::Contain)),
                    None => d.child(ui::roman().italic().text_color(k.faint).child(t("app.pdf.sign.import_help"))),
                })
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .child(ui::segmented(
                "sign-kind",
                &k,
                vec![("draw".into(), t("app.pdf.sign.draw")), ("type".into(), t("app.pdf.sign.type")), ("import".into(), t("app.pdf.sign.import"))],
                match kind {
                    Kind::Draw => "draw",
                    Kind::Type => "type",
                    Kind::Import => "import",
                },
                Rc::new(move |v, _, cx| {
                    let v = v.to_string();
                    let _ = this.update(cx, |s, cx| {
                        s.kind = match v.as_str() {
                            "type" => Kind::Type,
                            "import" => Kind::Import,
                            _ => Kind::Draw,
                        };
                        cx.notify();
                    });
                }),
            ))
            .when(kind == Kind::Type, |d| d.child(Input::new(&self.name)))
            .child(body)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap(px(8.0))
                    .items_center()
                    .when(kind == Kind::Draw, |d| {
                        d.child(Button::new("sign-clear").small().label(t("app.common.clear")).on_click(cx.listener(|s, _, _, cx| {
                            s.strokes.clear();
                            cx.notify();
                        })))
                        .child(ui::roman().text_size(px(13.5)).text_color(k.muted).child(t("app.pdf.sign.draw_help")))
                    })
                    .when(kind == Kind::Import, |d| {
                        d.child(Button::new("sign-choose").small().label(t("app.home.choose")).on_click(cx.listener(|s, _, w, cx| s.choose_image(w, cx))))
                    }),
            )
    }
}
