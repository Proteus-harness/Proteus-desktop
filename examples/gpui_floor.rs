//! Synthetic renderer baseline; no backend, extensions or application state.
//! cargo run --release --example gpui_floor [-- --assets]
use gpui_kit::*;
#[path = "../src/graphics.rs"]
mod graphics;

struct EmptyWindow;
impl Render for EmptyWindow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x181818))
            .text_color(rgb(0xdddddd))
            .child("GPUI renderer baseline")
    }
}

fn main() {
    graphics::prepare_environment();
    let mut app = gpui_kit::application();
    if std::env::args().any(|arg| arg == "--assets") {
        app = app.with_assets(gpui_kit::assets::AllAssets);
    }
    app.run(|cx| {
        gpui_kit::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        gpui_kit::open_window(
            WindowOptions {
                app_id: Some("dev.proteus.gpui-floor".into()),
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(820.)), cx)),
                window_decorations: Some(WindowDecorations::Server),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| EmptyWindow),
        )
        .expect("open baseline window");
    });
}
