//! Retained native fields moving between real windows. Run under background-gui:
//! cargo run --release --example input_rehost -- --verify REPORT.json
use gpui_kit::{
    component::{
        input::{InputEvent, *},
        *,
    },
    *,
};
use std::{cell::Cell, path::PathBuf, rc::Rc, time::Duration};

#[derive(Clone)]
struct Fields {
    line: Entity<InputState>,
    area: Entity<TextareaState>,
    code: Entity<EditorState>,
}
struct Fixture(Option<Fields>);
impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .children(self.0.as_ref().map(|fields| {
                v_flex()
                    .gap_4()
                    .child(Input::new(&fields.line))
                    .child(Textarea::new(&fields.area).h(px(80.)))
                    .child(Editor::new(&fields.code).h(px(120.)))
            }))
    }
}
#[derive(Default)]
struct Events {
    focus: Cell<usize>,
    blur: Cell<usize>,
    changes: Cell<usize>,
}

macro_rules! each {
    ($fields:expr, |$state:ident| $body:expr) => {{
        let $state = &$fields.line;
        $body;
        let $state = &$fields.area;
        $body;
        let $state = &$fields.code;
        $body;
    }};
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let report = args
        .iter()
        .position(|arg| arg == "--verify")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .expect("--verify REPORT.json");
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let mut fields = None;
            let (source, source_view) = gpui_kit::open_window(
                WindowOptions {
                    app_id: Some("dev.proteus.input-rehost-probe".into()),
                    window_bounds: Some(WindowBounds::centered(size(px(640.), px(400.)), cx)),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let retained = Fields {
                        line: cx.new(|cx| InputState::new(window, cx).default_value("before")),
                        area: cx.new(|cx| TextareaState::new(window, cx).default_value("before")),
                        code: cx.new(|cx| EditorState::new(window, cx).default_value("before")),
                    };
                    each!(retained, |state| state.update(cx, |state, cx| {
                        state.set_selected_range(6..6, cx);
                        state.replace_text_in_range(None, " after", window, cx);
                        state.set_selected_range(2..5, cx);
                    }));
                    fields = Some(retained.clone());
                    cx.new(|_| Fixture(Some(retained)))
                },
            )
            .unwrap();
            let fields = fields.unwrap();
            let events = Rc::new(Events::default());
            each!(fields, |state| {
                let received = events.clone();
                cx.subscribe(state, move |_, event: &InputEvent, _| match event {
                    InputEvent::Focus => received.focus.set(received.focus.get() + 1),
                    InputEvent::Blur => received.blur.set(received.blur.get() + 1),
                    _ => {}
                })
                .detach();
                let events = events.clone();
                cx.observe(state, move |_, _| {
                    events.changes.set(events.changes.get() + 1)
                })
                .detach();
            });
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                source_view.update(cx, |root, cx| {
                    root.0 = None;
                    cx.notify();
                });
                let moved = fields.clone();
                let (destination, _) = cx
                    .update(|cx| {
                        gpui_kit::open_window(
                            WindowOptions {
                                app_id: Some("dev.proteus.input-rehost-probe".into()),
                                window_bounds: Some(WindowBounds::centered(
                                    size(px(640.), px(400.)),
                                    cx,
                                )),
                                ..Default::default()
                            },
                            cx,
                            |_, cx| cx.new(|_| Fixture(Some(moved))),
                        )
                    })
                    .unwrap();
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                destination
                    .update(cx, |_, window, _| window.activate_window())
                    .unwrap();
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                // Rendering, rather than an explicit rehost call, must have rebound
                // all three modes. Selection/text survive the move before editing.
                destination
                    .update(cx, |_, _, cx| {
                        each!(fields, |state| {
                            assert_eq!(state.read(cx).value().as_ref(), "before after");
                            assert_eq!(state.read(cx).selected_range(), 2..5);
                        });
                    })
                    .unwrap();
                macro_rules! verify {
                    ($state:expr) => {{
                        let focus_before = events.focus.get();
                        let blur_before = events.blur.get();
                        destination
                            .update(cx, |_, window, cx| {
                                $state.update(cx, |state, cx| state.focus(window, cx))
                            })
                            .unwrap();
                        cx.background_executor()
                            .timer(Duration::from_millis(120))
                            .await;
                        assert_eq!(
                            events.focus.get(),
                            focus_before + 1,
                            "missing/duplicate destination focus"
                        );
                        destination
                            .update(cx, |_, window, cx| {
                                window.dispatch_action(Box::new(gpui_kit::base::input::Undo), cx)
                            })
                            .unwrap();
                        cx.background_executor()
                            .timer(Duration::from_millis(120))
                            .await;
                        destination
                            .update(cx, |_, _, cx| {
                                assert_eq!(
                                    $state.read(cx).value().as_ref(),
                                    "before",
                                    "undo lost on move"
                                )
                            })
                            .unwrap();
                        destination
                            .update(cx, |_, window, cx| {
                                window.dispatch_action(Box::new(gpui_kit::base::input::Redo), cx)
                            })
                            .unwrap();
                        cx.background_executor()
                            .timer(Duration::from_millis(120))
                            .await;
                        destination
                            .update(cx, |_, _, cx| {
                                assert_eq!(
                                    $state.read(cx).value().as_ref(),
                                    "before after",
                                    "redo lost on move"
                                )
                            })
                            .unwrap();
                        destination
                            .update(cx, |_, window, cx| window.blur(cx))
                            .unwrap();
                        cx.background_executor()
                            .timer(Duration::from_millis(150))
                            .await;
                        assert_eq!(
                            events.blur.get(),
                            blur_before + 1,
                            "missing/duplicate destination blur"
                        );
                    }};
                }
                verify!(fields.line);
                verify!(fields.area);
                verify!(fields.code);
                source
                    .update(cx, |_, window, _| window.remove_window())
                    .unwrap();
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                let before = events.changes.get();
                cx.background_executor()
                    .timer(Duration::from_millis(1600))
                    .await;
                let after = events.changes.get();
                assert_eq!(
                    before, after,
                    "blurred moved fields still notify on blink timers"
                );
                if let Some(parent) = report.parent() {
                    std::fs::create_dir_all(parent).unwrap();
                }
                std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({
                "status": "passed", "modes": ["Input", "Textarea", "Editor"],
                "text_selection_undo_redo_preserved": true,
                "destination_focus_blur": true, "source_closed": true,
                "blurred_notifications_before": before, "blurred_notifications_after": after,
                "idle_observation_ms": 1600,
            })).unwrap()).unwrap();
                eprintln!("PASS: native input rehost; {}", report.display());
                cx.update(|cx| cx.quit());
            })
            .detach();
        });
}
