//! Native regression fixture for a bounded window over 5,000 history rows.
//! cargo run --release --example history_scrollbar -- --verify REPORT.json
//! Add --mixed-heights --probe REPORT.json to verify pixel motion across tall/short rows.
use std::{path::PathBuf, time::Duration};

use gpui_kit::component::scroll::{ScrollableElement as _, ScrollableMask, ScrollbarHandle};
use gpui_kit::{component::*, *};

#[path = "../src/ui/chat/scroller/scrollbar.rs"]
mod scrollbar;
use scrollbar::{HistoryRange, HistoryScrollbar, ScrollTarget};

#[path = "history_scrollbar/probe.rs"]
mod probe;

const TOTAL: usize = 5000;
const PAGE: usize = 50;
const CACHE: usize = 200;

struct Fixture {
    list: ListState,
    scrollbar: HistoryScrollbar,
    start: usize,
    rows: usize,
    mixed_heights: bool,
    height_scale: f32,
    render_generation: u64,
    probe_frames: bool,
}

impl Fixture {
    fn new(cx: &mut Context<Self>) -> Self {
        let list = ListState::new(PAGE + 1, ListAlignment::Top, px(400.));
        let scrollbar = HistoryScrollbar::new(list.clone(), px(0.));
        let wheel = scrollbar.clone();
        let weak = cx.weak_entity();
        list.set_scroll_handler(move |_, _, cx| {
            wheel.cancel_target();
            let weak = weak.clone();
            cx.defer(move |cx| {
                let _ = weak.update(cx, |_, cx| cx.notify());
            });
        });
        let this = Self {
            list,
            scrollbar,
            start: TOTAL - PAGE,
            rows: PAGE,
            mixed_heights: std::env::args().any(|arg| arg == "--mixed-heights"),
            height_scale: 1.,
            render_generation: 0,
            probe_frames: false,
        };
        this.sync();
        this.list.scroll_to(ListOffset {
            item_ix: 3,
            offset_in_item: px(0.),
        });
        this
    }

    fn sync(&self) {
        self.scrollbar.set_range(
            Some(HistoryRange {
                start: self.start,
                rows: self.rows,
                total: TOTAL,
            }),
            false,
        );
    }

    fn older(&mut self) {
        self.start -= PAGE;
        self.list.splice(0..0, PAGE);
        self.rows += PAGE;
        if self.rows > CACHE {
            self.list.splice(CACHE..self.rows, 0);
            self.rows = CACHE;
        }
        self.sync();
    }

    fn seek(&mut self, target: ScrollTarget) {
        if target.row >= TOTAL {
            self.start = TOTAL - PAGE;
        } else if target.row < self.start || target.row >= self.start + self.rows {
            self.start = target.row.saturating_sub(PAGE / 2).min(TOTAL - PAGE);
        } else {
            self.scrollbar.scroll_to(target);
            return;
        }
        self.list.splice(0..self.rows, PAGE);
        self.rows = PAGE;
        self.sync();
        if target.row >= TOTAL {
            self.list.scroll_to_end();
        } else {
            self.scrollbar.scroll_to(target);
        }
    }

    fn verify(&self) -> serde_json::Value {
        let content = self.scrollbar.content_size();
        let top = self.list.logical_scroll_top();
        let physical_offset = (0..top.item_ix)
            .map(|index| row_height(self.start + index, self.mixed_heights) * self.height_scale)
            .sum::<f32>()
            + f32::from(top.offset_in_item);
        serde_json::json!({
            "start":self.start, "resident_rows":self.rows, "total":TOTAL,
            "content_height":f32::from(content.height),
            "offset":f32::from(-self.scrollbar.offset().y),
            "viewport_height":f32::from(self.list.viewport_bounds().size.height),
            "global_anchor":self.start+self.list.logical_scroll_top().item_ix,
            "row_offset":f32::from(self.list.logical_scroll_top().offset_in_item),
            "pixel_offset":f32::from(-self.list.scroll_px_offset_for_scrollbar().y),
            "physical_offset":physical_offset,
            "render_generation":self.render_generation,
            "height_scale":self.height_scale,
        })
    }
}

impl Render for Fixture {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_generation += 1;
        if self.probe_frames {
            window.request_animation_frame();
        }
        if let Some(target) = self.scrollbar.take_target() {
            self.seek(target);
        }
        if self.scrollbar.settle() {
            window.request_animation_frame();
        }
        let start = self.start;
        let count = self.rows;
        let mixed_heights = self.mixed_heights;
        let height_scale = self.height_scale;
        let rows = list(self.list.clone(), move |index, _, _| {
            if index == count {
                return div().h(px(0.)).into_any_element();
            }
            let row = start + index;
            div()
                .w_full()
                .h(px(row_height(row, mixed_heights) * height_scale))
                .px_4()
                .py_2()
                .border_b_1()
                .child(format!("Сообщение {} из {TOTAL}", row + 1))
                .into_any_element()
        })
        .size_full();
        div()
            .flex()
            .flex_col()
            .size_full()
            .p_4()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .pb_3()
                    .child("Полная история — 5 000 сообщений; в памяти — до 200"),
            )
            .child(
                div()
                    .id("history-scrollbar-fixture")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(rows)
                    .vertical_scrollbar(&self.scrollbar)
                    .child(ScrollableMask::new(Axis::Vertical, &self.list)),
            )
    }
}

fn row_height(row: usize, mixed: bool) -> f32 {
    if mixed {
        match row % 4 {
            0 => 960.,
            1 => 36.,
            2 => 180.,
            _ => 60.,
        }
    } else {
        70. + (row % 3) as f32 * 30.
    }
}

fn main() {
    let probe = std::env::args()
        .position(|arg| arg == "--probe")
        .and_then(|index| std::env::args().nth(index + 1))
        .map(PathBuf::from);
    let report = std::env::args()
        .position(|arg| arg == "--verify")
        .and_then(|index| std::env::args().nth(index + 1))
        .map(PathBuf::from);
    gpui_kit::application().with_assets(gpui_kit::assets::AllAssets).run(move |cx| {
        gpui_kit::init(cx);
        cx.on_window_closed(|cx, _| { if cx.windows().is_empty() { cx.quit(); } }).detach();
        gpui_kit::open_window(WindowOptions {
            app_id: Some("dev.proteus.history-scrollbar-fixture".into()),
            window_bounds: Some(WindowBounds::centered(size(px(1000.), px(780.)), cx)),
            // Match the chat window even if the compositor reports it inactive.
            inactive_frame_interval: None,
            ..Default::default()
        }, cx, move |_, cx| cx.new(|cx: &mut Context<Fixture>| {
            if let Some(path) = probe {
                probe::start(path, cx);
            }
            if let Some(report) = report {
                cx.spawn(async move |this, cx| {
                    let mut samples = Vec::new();
                    for _ in 0..100 {
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        if this.read_with(cx, |this, _| this.list.viewport_bounds().size.height > px(0.)).unwrap() { break }
                    }
                    for _ in 0..7 {
                        this.update(cx, |this, cx| {
                            this.list.scroll_to(ListOffset { item_ix: 3, offset_in_item: px(0.) });
                            cx.notify();
                        }).unwrap();
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        let before = this.update(cx, |this, cx| {
                            let before = this.verify();
                            this.older();
                            cx.notify();
                            before
                        }).unwrap();
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        let after = this.read_with(cx, |this, _| this.verify()).unwrap();
                        assert_eq!(before["content_height"], after["content_height"]);
                        assert_eq!(before["global_anchor"], after["global_anchor"]);
                        assert!((before["offset"].as_f64().unwrap()-after["offset"].as_f64().unwrap()).abs() < 0.1);
                        samples.push(serde_json::json!({"before":before,"after":after}));
                    }
                    for share in [0.5, 0., 1., 0.5] {
                        this.update(cx, |this, cx| {
                            let max = this.scrollbar.content_size().height-this.list.viewport_bounds().size.height;
                            this.scrollbar.set_offset(point(px(0.), -max*share));
                            cx.notify();
                        }).unwrap();
                        cx.background_executor().timer(Duration::from_millis(300)).await;
                        let sample = this.read_with(cx, |this, _| this.verify()).unwrap();
                        assert_eq!(sample["resident_rows"], PAGE);
                        if share == 0. { assert_eq!(sample["global_anchor"], 0); }
                        if share == 1. {
                            let max = sample["content_height"].as_f64().unwrap()-sample["viewport_height"].as_f64().unwrap();
                            assert!((sample["offset"].as_f64().unwrap()-max).abs() < 0.1);
                        }
                        samples.push(sample);
                    }
                    if let Some(parent) = report.parent() { std::fs::create_dir_all(parent).unwrap(); }
                    std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
                        "status":"passed", "scope":"native rendered paged scrollbar", "samples":samples,
                    })).unwrap()).unwrap();
                    eprintln!("PASS: native history scrollbar; {}", report.display());
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    this.update(cx, |_, cx| cx.quit()).ok();
                }).detach();
            }
            Fixture::new(cx)
        })).expect("open history scrollbar fixture");
    });
}
