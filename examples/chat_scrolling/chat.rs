use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    path::PathBuf,
    rc::Rc,
};

use gpui_kit::{component::*, *};

#[path = "probe.rs"]
mod probe;
#[path = "../../src/ui/chat/scroller.rs"]
mod scroller;

// History loading is irrelevant here: all fixture rows are resident. The
// production scroller's edge notifications still execute and are not mocked.
mod history {
    pub enum Direction {
        Older,
        Newer,
    }
}

const ROWS: usize = 400;

fn row_height(row: usize) -> f32 {
    if row % 7 == 0 {
        880.
    } else {
        24. + (row % 3) as f32 * 13.
    }
}

#[derive(Default)]
struct Geometry {
    viewport: Cell<Bounds<Pixels>>,
    rows: RefCell<BTreeMap<usize, Bounds<Pixels>>>,
    layouts: Cell<u64>,
    frames: RefCell<Vec<serde_json::Value>>,
}

struct Fixture {
    scroller: Entity<scroller::ChatScroller>,
    geometry: Rc<Geometry>,
    renders: u64,
    scroller_notifications: Rc<Cell<u64>>,
    _observe: Subscription,
}

impl Fixture {
    fn new(cx: &mut Context<Self>) -> Self {
        let scroller = cx.new(|cx| {
            let mut scroller = scroller::ChatScroller::new(false);
            scroller.splice(0..0, ROWS, cx);
            scroller.set_archived(true, cx);
            scroller.set_history_range(
                Some(scroller::HistoryRange {
                    start: 0,
                    rows: ROWS,
                    total: ROWS,
                }),
                false,
                cx,
            );
            scroller.scroll_to_history(
                scroller::ScrollTarget {
                    row: 40,
                    fraction: 0.,
                },
                cx,
            );
            scroller
        });
        let scroller_notifications = Rc::new(Cell::new(0));
        let notifications = scroller_notifications.clone();
        let observe = cx.observe(&scroller, move |_, _, cx| {
            notifications.set(notifications.get() + 1);
            cx.notify();
        });
        Self {
            scroller,
            geometry: Rc::default(),
            renders: 0,
            scroller_notifications,
            _observe: observe,
        }
    }

    fn snapshot(&self, cx: &App) -> anyhow::Result<serde_json::Value> {
        let visible = self.scroller.read(cx).visible_rows();
        let bounds = self.geometry.rows.borrow();
        let row = bounds.get(&visible.start).ok_or_else(|| {
            anyhow::anyhow!("visible row {} has no prepaint geometry", visible.start)
        })?;
        let viewport = self.geometry.viewport.get();
        anyhow::ensure!(
            viewport.size.height > px(0.),
            "native viewport has not been laid out"
        );
        // Expected heights are part of the synthetic corpus, not a duplicate
        // of the production scroll algorithm. Position comes from prepaint.
        let preceding: f32 = (0..visible.start).map(row_height).sum();
        let physical = preceding - f32::from(row.top() - viewport.top());
        Ok(serde_json::json!({
            "physical_offset": physical,
            "first_visible_row": visible.start, "visible_end": visible.end,
            "first_row_y": f32::from(row.top()),
            "first_row_height": f32::from(row.size.height),
            "viewport_top": f32::from(viewport.top()),
            "viewport_height": f32::from(viewport.size.height),
            "parent_renders": self.renders, "layouts": self.geometry.layouts.get(),
            "scroller_notifications": self.scroller_notifications.get(),
        }))
    }
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.renders += 1;
        let geometry = self.geometry.clone();
        let rows = self.geometry.clone();
        let frames = self.geometry.clone();
        let scroller = self.scroller.clone();
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .h(px(60.))
                    .flex_shrink_0()
                    .p_3()
                    .child("Production ChatList · pixel wheel and held middle button"),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .on_prepaint(move |bounds, _, _| {
                        geometry.viewport.set(bounds);
                        geometry.rows.borrow_mut().clear();
                        geometry.layouts.set(geometry.layouts.get() + 1);
                    })
                    .child(scroller::ChatList::new(
                        "probe-chat",
                        self.scroller.clone(),
                        move |row, _, cx| {
                            let bounds = rows.clone();
                            div()
                                .w_full()
                                .h(px(row_height(row)))
                                .overflow_hidden()
                                .border_b_1()
                                .border_color(cx.theme().border)
                                .bg(if row % 7 == 0 {
                                    cx.theme().muted
                                } else {
                                    cx.theme().background
                                })
                                .on_prepaint(move |geometry, _, _| {
                                    bounds.rows.borrow_mut().insert(row, geometry);
                                })
                                .child(format!("Row {row}: {} physical pixels", row_height(row)))
                                .into_any_element()
                        },
                    ))
                    .child(canvas(move |_, _, cx| {
                        let visible = scroller.read(cx).visible_rows();
                        if let Some(row) = frames.rows.borrow().get(&visible.start) {
                            let physical = (0..visible.start).map(row_height).sum::<f32>()
                                - f32::from(row.top() - frames.viewport.get().top());
                            let mut trace = frames.frames.borrow_mut();
                            // Bound memory even when left open for manual input.
                            if trace.len() < 4096 {
                                trace.push(serde_json::json!({
                                    "layout":frames.layouts.get(), "first_visible_row":visible.start,
                                    "physical_offset":physical,
                                }));
                            }
                        }
                    }, |_, _, _, _| {}).absolute().top_0().left_0().size_full()),
            )
            // The release test targets this sibling, outside the chat hitbox.
            .child(
                div()
                    .h(px(60.))
                    .flex_shrink_0()
                    .p_3()
                    .child("Release outside the list here"),
            )
    }
}

pub(crate) fn run() {
    let args: Vec<_> = std::env::args().collect();
    let report = args
        .iter()
        .position(|arg| arg == "--verify")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from);
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("dev.proteus.chat-scrolling-fixture".into()),
                    window_bounds: Some(WindowBounds::centered(size(px(1000.), px(780.)), cx)),
                    inactive_frame_interval: None,
                    ..Default::default()
                },
                cx,
                move |window, cx| {
                    cx.new(|cx| {
                        if let Some(report) = report {
                            probe::start(report, window.window_handle(), cx);
                        }
                        Fixture::new(cx)
                    })
                },
            )
            .expect("open chat scrolling fixture");
        });
}
