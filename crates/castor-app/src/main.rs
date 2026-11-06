use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;
use std::time::Duration;

pub struct Example { clicks: usize, tray_events: usize }
impl Render for Example {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().v_flex().gap_2().size_full().items_center().justify_center()
            .child(format!("Hello from spike — clicks {} — tray events {}", self.clicks, self.tray_events))
            .child(Button::new("ok").primary().label("Click").on_click(cx.listener(|this, _, _, cx| { this.clicks += 1; cx.notify(); })))
    }
}

fn icon() -> tray_icon::Icon {
    let (w, h) = (32u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h { for x in 0..w {
        let (dx, dy) = ((x as i32 - 16).abs(), (y as i32 - 16).abs());
        if dx + dy <= 14 && dx + dy >= 10 { let i = ((y * w + x) * 4) as usize; rgba[i] = 30; rgba[i+1]=30; rgba[i+2]=30; rgba[i+3]=255; }
    }}
    tray_icon::Icon::from_rgba(rgba, w, h).unwrap()
}

fn main() {
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let tray = tray_icon::TrayIconBuilder::new().with_tooltip("spike").with_icon(icon()).build().expect("tray");
        std::mem::forget(tray);
        let view = gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| Example { clicks: 0, tray_events: 0 })).expect("window");
        let _ = view;
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                while let Ok(ev) = tray_icon::TrayIconEvent::receiver().try_recv() {
                    println!("tray event: {:?}", ev);
                }
            }
        }).detach();
    });
}
