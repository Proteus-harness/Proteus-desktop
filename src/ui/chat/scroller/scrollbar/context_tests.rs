//! Real grouped Chat rows must not rewrite global facts at partial page edges.
use super::*;
use crate::ui::chat::{Chat, Session, row_layout};
use ::gpui::{TestAppContext, VisualTestContext};
use proteus_contracts::app_protocol::{
    AppRun, AppRunStatus, AppTranscriptMessage, AppTranscriptSubagent, AppTranscriptTool,
};
use std::{ops::Range, time::Instant};

struct GroupedRows {
    bar: HistoryScrollbar,
    chat: Entity<Chat>,
}

impl Render for GroupedRows {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = self.bar.clone();
        let range = bar.state.get().range.unwrap();
        let context = row_layout::row_context(
            self.chat.read(cx).display_transcript().entries(),
            true,
            Some(range),
        );
        let chat = self.chat.clone();
        let at = Instant::now();
        list(bar.list.clone(), move |ix, window, cx| {
            let row = if ix == range.rows {
                div().h(px(0.)).into_any_element()
            } else {
                chat.update(cx, |chat, cx| chat.render_row(ix, at, window, cx))
            };
            bar.measure_chat_row(ix, row, context.missing(ix).is_none())
        })
        .size_full()
    }
}

fn messages(hidden_suffix: bool) -> Vec<AppTranscriptMessage> {
    let mut rows: Vec<_> = (0..10)
        .map(|row| AppTranscriptMessage {
            timestamp_ms: Some(1000 + row as i64),
            message_id: Some(uuid::Uuid::from_u128(row as u128 + 1)),
            phase: None,
            role: if (1..6).contains(&row) {
                "system"
            } else {
                "user"
            }
            .into(),
            text: "synthetic row".into(),
            images: Vec::new(),
            tool: (1..6).contains(&row).then(|| AppTranscriptTool {
                call_id: format!("call-{row}"),
                name: "read_file".into(),
                args: serde_json::json!({"path":"synthetic.rs"}),
                effective_args: None,
                status: "done".into(),
                result: Some("synthetic result".into()),
                metadata: serde_json::Value::Null,
                terminal: None,
            }),
            subagent: None,
            streaming: false,
        })
        .collect();
    if hidden_suffix {
        rows[0].role = "system".into();
        rows[0].subagent = Some(AppTranscriptSubagent {
            child_thread_id: "exact-child".into(),
            session: None,
            role: "worker".into(),
            description: None,
            status: "completed".into(),
            iterations: None,
            tools: Vec::new(),
        });
        let mut hidden = rows[5].clone();
        hidden.message_id = Some(uuid::Uuid::from_u128(100));
        hidden.tool.as_mut().unwrap().call_id = "child-progress".into();
        hidden.tool.as_mut().unwrap().metadata =
            serde_json::json!({"tool":"task", "child_thread_id":"exact-child"});
        rows.insert(6, hidden);
    }
    rows
}

fn chat(cx: &mut VisualTestContext) -> Entity<Chat> {
    cx.update(|window, cx| {
        cx.new(|cx| {
            let mut session = Session::new("/synthetic/group-context".into());
            // A partial archive tail cannot add a footer until its successor
            // arrives. A running live session has that same rendering rule.
            session.execution.active = Some(AppRun {
                run_id: "synthetic-running".into(),
                options: Default::default(),
                status: AppRunStatus::Running,
                error: None,
            });
            Chat::base(
                None,
                WeakEntity::new_invalid(),
                session,
                Rc::new(|_, _, _| None),
                window,
                cx,
            )
        })
    })
}

fn visit(
    cx: &mut VisualTestContext,
    bar: &HistoryScrollbar,
    chat: &Entity<Chat>,
    messages: &[AppTranscriptMessage],
    rows: Range<usize>,
) {
    cx.update(|_, cx| {
        chat.update(cx, |chat, _| {
            chat.session
                .transcript
                .reset(messages[rows.clone()].to_vec());
        });
    });
    bar.list.splice(0..bar.list.item_count(), rows.len() + 1);
    bar.set_range(
        Some(HistoryRange {
            start: rows.start,
            rows: rows.len(),
            total: messages.len(),
        }),
        false,
    );
    bar.list.scroll_to(ListOffset::default());
    bar.set_viewport_width(px(700.));
    let view = cx.update(|_, cx| {
        cx.new(|_| GroupedRows {
            bar: bar.clone(),
            chat: chat.clone(),
        })
    });
    for _ in 0..2 {
        cx.draw(point(px(0.), px(0.)), size(px(700.), px(1000.)), |_, _| {
            view.clone().into_any_element()
        });
        bar.did_layout();
    }
}

#[::core::prelude::v1::test]
fn clipped_tool_runs_keep_complete_group_facts_on_repeated_native_visits() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    let chat = chat(cx);
    let messages = messages(false);
    let bar = HistoryScrollbar::new(ListState::new(0, ListAlignment::Top, px(400.)), px(0.));
    visit(cx, &bar, &chat, &messages, 0..messages.len());
    let facts: Vec<_> = (1..6)
        .map(|row| bar.geometry.borrow().estimated_height(row).unwrap())
        .collect();
    assert!(facts[0] > px(0.));
    assert_eq!(&facts[1..], &[px(0.); 4]);
    let extent = bar.content_size().height;
    for _ in 0..3 {
        // Leading run creates a false interior head; a two-call trailing run
        // unfolds full tool cards; a longer trailing run loses its footer.
        for rows in [0..3, 3..10, 4..10, 0..6] {
            visit(cx, &bar, &chat, &messages, rows.clone());
            assert_eq!(bar.content_size().height, extent, "window {rows:?}");
            for (row, expected) in (1..6).zip(&facts) {
                assert_eq!(
                    bar.geometry.borrow().estimated_height(row),
                    Some(*expected),
                    "window {rows:?}, global row {row}"
                );
            }
            let row = if rows.start > 0 { rows.start + 1 } else { 1 };
            cx.update(|_, cx| {
                let context = row_layout::row_context(
                    chat.read(cx).display_transcript().entries(),
                    true,
                    bar.state.get().range,
                );
                assert_eq!(
                    context.missing(row - rows.start),
                    Some(if rows.start > 0 {
                        crate::ui::chat::history::Direction::Older
                    } else {
                        crate::ui::chat::history::Direction::Newer
                    }),
                    "a fractional seek must request the missing neighbor"
                );
            });
            assert_eq!(bar.native_height(row), None);
            assert!(bar.scroll_to(ScrollTarget { row, fraction: 0.5 }));
            bar.settle();
            assert!(bar.state.get().local.is_some(), "clipped row cannot settle");
        }
        visit(cx, &bar, &chat, &messages, 0..messages.len());
        bar.settle();
        assert!(bar.state.get().local.is_none());
        assert_eq!(bar.content_size().height, extent);
    }
}

#[::core::prelude::v1::test]
fn hidden_progress_suffix_keeps_the_interior_folded_footer_owner_incomplete() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    let chat = chat(cx);
    let messages = messages(true);
    let bar = HistoryScrollbar::new(ListState::new(0, ListAlignment::Top, px(400.)), px(0.));
    visit(cx, &bar, &chat, &messages, 0..messages.len());
    let height = bar.geometry.borrow().estimated_height(1).unwrap();
    let extent = bar.content_size().height;
    for _ in 0..3 {
        visit(cx, &bar, &chat, &messages, 0..7);
        assert_eq!(bar.content_size().height, extent);
        assert_eq!(bar.geometry.borrow().estimated_height(1), Some(height));
        assert_eq!(bar.native_height(1), None, "head is interior to the page");
        assert!(bar.scroll_to(ScrollTarget {
            row: 1,
            fraction: 0.5
        }));
        bar.settle();
        assert!(bar.state.get().local.is_some());
        visit(cx, &bar, &chat, &messages, 0..messages.len());
        bar.settle();
        assert!(bar.state.get().local.is_none());
        assert_eq!(bar.native_height(1), Some(height));
    }
}

#[::core::prelude::v1::test]
fn an_oversized_group_can_finish_at_row_start_without_confirming_its_fractional_height() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    let chat = chat(cx);
    let sample = messages(false);
    let mut messages = vec![sample[0].clone()];
    for row in 1..=300 {
        let mut tool = sample[1].clone();
        tool.message_id = Some(uuid::Uuid::from_u128(row as u128 + 1));
        tool.tool.as_mut().unwrap().call_id = format!("oversized-{row}");
        messages.push(tool);
    }
    let mut end = sample[6].clone();
    end.message_id = Some(uuid::Uuid::from_u128(1000));
    messages.push(end);
    let bar = HistoryScrollbar::new(ListState::new(0, ListAlignment::Top, px(400.)), px(0.));
    visit(cx, &bar, &chat, &messages, 0..200);
    let extent = bar.content_size().height;
    let target = ScrollTarget {
        row: 100,
        fraction: 0.5,
    };
    assert!(bar.scroll_to(target));
    bar.settle();
    assert!(bar.state.get().local.is_some());
    assert_eq!(bar.native_height(target.row), None);
    assert!(bar.scroll_to(ScrollTarget {
        fraction: 0.,
        ..target
    }));
    assert!(
        bar.state.get().local.is_none(),
        "bounded fallback completes the pending navigation"
    );
    assert_eq!(bar.native_height(target.row), None);
    assert_eq!(
        bar.content_size().height,
        extent,
        "fallback does not publish partial group heights"
    );
}
