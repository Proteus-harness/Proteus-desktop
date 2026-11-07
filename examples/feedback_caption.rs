//! Native isolation probe for the real shimmer and shared ticker. After the
//! first settled frame, child ticks must not render its containing thread.
//! cargo run --release --example feedback_caption -- --verify REPORT.json
//! Add --reduced to check the static caption with no ticking animation.
use std::{
    cell::Cell,
    path::PathBuf,
    rc::Rc,
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
    child_ticks: Rc<Cell<usize>>,
    laid_out: Rc<Cell<bool>>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let laid_out = self.laid_out.clone();
        div()
            .size_full()
            .bg(cx.theme().background)
            .p_8()
            .text_size(px(28.))
            .child(self.caption.clone())
            .on_children_prepainted(move |bounds, _, _| {
                laid_out.set(bounds.first().is_some_and(|bounds| {
                    bounds.size.width > px(0.) && bounds.size.height > px(0.)
                }));
            })
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
        }, cx, move |window, cx| {
          let window = window.window_handle();
          cx.new(|cx: &mut Context<Fixture>| {
            let (caption, child_ticks) = ui::caption(cx);
            let laid_out = Rc::new(Cell::new(false));
            if let Some(report) = report {
                cx.spawn(async move |this, cx| {
                    let mut ready = false;
                    for _ in 0..100 {
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        let painted = this.read_with(cx, |this, _| this.renders > 0 && this.laid_out.get()
                            && (reduced || this.child_ticks.get() > 1)).unwrap();
                        if painted && window.update(cx, |_, window, _| window.is_window_active()).unwrap() {
                            ready = true;
                            break;
                        }
                    }
                    assert!(ready, "native caption did not lay out and animate in an active window");
                    cx.background_executor().timer(Duration::from_secs(2)).await;
                    let (before, ticks_before) = this.read_with(cx, |this, _| (this.renders, this.child_ticks.get())).unwrap();
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    let (after, ticks_after) = this.read_with(cx, |this, _| (this.renders, this.child_ticks.get())).unwrap();
                    assert!(window.update(cx, |_, window, _| window.is_window_active()).unwrap(), "caption window became inactive during observation");
                    assert_eq!(before, after, "caption ticks rendered the parent thread");
                    if reduced {
                        assert_eq!(ticks_before, 0, "reduced caption unexpectedly scheduled a tick");
                        assert_eq!(ticks_after, 0, "reduced caption unexpectedly scheduled a tick");
                    } else {
                        assert!(ticks_after > ticks_before + 1, "caption stopped animating during isolation observation");
                    }
                    if let Some(parent) = report.parent() { std::fs::create_dir_all(parent).unwrap(); }
                    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
                        "status": "passed", "scope": "native retained shimmer parent isolation",
                        "reduced": reduced, "parent_renders_before": before,
                        "parent_renders_after": after, "observation_seconds": 3,
                        "child_ticks_before": ticks_before, "child_ticks_after": ticks_after,
                        "active_window_layout_verified": true,
                    })).unwrap()).unwrap();
                    eprintln!("PASS: retained shimmer; {}", report.display());
                    this.update(cx, |_, cx| cx.quit()).ok();
                }).detach();
            }
            Fixture { renders: 0, caption, child_ticks, laid_out }
        })}).expect("open feedback caption fixture");
    });
}
