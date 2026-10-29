//! Native isolation probe for the real shimmer and shared ticker. After the
//! first settled frame, child ticks must not render its containing thread.
//! cargo run --release --example feedback_caption -- --verify REPORT.json
//! Add --reduced to check the static caption with no ticking animation.
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use gpui_kit::{component::*, *};

#[path = "feedback_caption/ui.rs"]
mod ui;

static MOVES: AtomicBool = AtomicBool::new(true);
fn moves() -> bool {
    MOVES.load(Ordering::Relaxed)
}

struct Fixture {
    renders: usize,
    caption: AnyView,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        div()
            .size_full()
            .bg(cx.theme().background)
            .p_8()
            .text_size(px(28.))
            .child(self.caption.clone())
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let report = args
        .iter()
        .position(|arg| arg == "--verify")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from);
    let reduced = args.iter().any(|arg| arg == "--reduced");
    MOVES.store(!reduced, Ordering::Relaxed);
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(move |cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions {
            app_id: Some("dev.proteus.feedback-caption-fixture".into()),
            window_bounds: Some(WindowBounds::centered(size(px(600.), px(250.)), cx)),
            inactive_frame_interval: None,
            ..Default::default()
        }, cx, move |_, cx| cx.new(|cx: &mut Context<Fixture>| {
            if let Some(report) = report {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    let before = this.read_with(cx, |this, _| this.renders).unwrap();
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    let after = this.read_with(cx, |this, _| this.renders).unwrap();
                    assert_eq!(before, after, "caption ticks rendered the parent thread");
                    if let Some(parent) = report.parent() { std::fs::create_dir_all(parent).unwrap(); }
                    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
                        "status": "passed", "scope": "native retained shimmer parent isolation",
                        "reduced": reduced, "parent_renders_before": before,
                        "parent_renders_after": after, "observation_seconds": 3,
                    })).unwrap()).unwrap();
                    eprintln!("PASS: retained shimmer; {}", report.display());
                    this.update(cx, |_, cx| cx.quit()).ok();
                }).detach();
            }
            Fixture { renders: 0, caption: ui::caption(cx) }
        })).expect("open feedback caption fixture");
    });
}
