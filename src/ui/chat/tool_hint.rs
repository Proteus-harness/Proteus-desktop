// Адаптировано из Zed: crates/agent_ui/src/sandbox_status_tooltip.rs
//! Tool parameters are read only when a tooltip opens. A preview has a small
//! character/node budget and never serializes or clones a large tool result.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use proteus_contracts::app_protocol::AppTranscriptTool;
use serde_json::Value;

use super::{Chat, tool_row::tool_args};
use crate::ui::tip::Tip as _;

const ARGUMENTS: usize = 6;
const PREVIEW_CHARS: usize = 600;
const RECENT_CALLS: usize = 2;

pub(super) fn row(
    trigger: impl InteractiveElement + ParentElement + IntoElement + Styled,
    chat: WeakEntity<Chat>,
    key: u64,
    call_id: String,
) -> impl IntoElement {
    // A tool label may also be the only child of a vertical container (the
    // subagent tool list). Do not let that container stretch its anchor.
    trigger.self_start().max_w_full().tip_element(move |_, cx| {
        chat.read_with(cx, |chat, cx| {
            let tool = chat
                .display_transcript()
                .entries()
                .iter()
                .find(|entry| entry.key == key)
                .and_then(|entry| {
                    entry
                        .tool
                        .as_ref()
                        .filter(|tool| tool.call_id == call_id)
                        .or_else(|| {
                            entry
                                .subagent
                                .as_ref()?
                                .tools
                                .iter()
                                .find(|tool| tool.call_id == call_id)
                        })
                });
            tool.map_or_else(|| unavailable(cx), |tool| details(tool, PREVIEW_CHARS, cx))
        })
        .unwrap_or_else(|_| unavailable(cx))
    })
}

pub(super) fn group(
    trigger: impl InteractiveElement + ParentElement + IntoElement,
    chat: WeakEntity<Chat>,
    head: u64,
    name: String,
) -> impl IntoElement {
    trigger.tip_element(move |_, cx| {
        chat.read_with(cx, |chat, cx| {
            let entries = chat.display_transcript().entries();
            let Some(range) = entries
                .iter()
                .position(|entry| entry.key == head)
                .and_then(|ix| crate::transcript::tool_chain(entries, ix))
            else {
                return unavailable(cx);
            };
            let matching = || {
                entries[range.clone()]
                    .iter()
                    .filter_map(|entry| entry.tool.as_ref())
                    .filter(|tool| tool.name == name)
            };
            let count = matching().count();
            let mut recent: Vec<_> = matching().rev().take(RECENT_CALLS).collect();
            recent.reverse();
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Вызовов: {count} · последние {}", recent.len())),
                )
                .children(
                    recent
                        .into_iter()
                        .map(|tool| details(tool, PREVIEW_CHARS / RECENT_CALLS, cx)),
                )
        })
        .unwrap_or_else(|_| unavailable(cx))
    })
}

fn unavailable(cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("Вызов больше не загружен")
}

fn details(tool: &AppTranscriptTool, budget: usize, cx: &App) -> Div {
    let status = match tool.status.as_str() {
        "done" => "Выполнен",
        "running" | "approved" => "Выполняется",
        "waiting_approval" => "Ожидает разрешения",
        "failed" => "Ошибка",
        "denied" => "Отклонён",
        _ => "Прерван",
    };
    let args = tool_args(tool);
    let mut remaining = budget;
    let rows = match args.as_object() {
        Some(args) => args
            .iter()
            .take(ARGUMENTS)
            .map(|(key, value)| {
                let key = preview(&Value::String(key.chars().take(64).collect()), 64);
                let value = preview(value, remaining.min(220));
                remaining = remaining.saturating_sub(key.chars().count() + value.chars().count());
                (key.trim_matches('"').to_owned(), value)
            })
            .take_while(|(_, value)| !value.is_empty())
            .collect::<Vec<_>>(),
        None => vec![("Аргументы".into(), preview(args, budget))],
    };
    v_flex()
        .gap_1()
        .text_xs()
        .child(
            h_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(tool.name.clone()),
                )
                .child(div().text_color(cx.theme().muted_foreground).child(status)),
        )
        .children(rows.into_iter().map(|(key, value)| {
            v_flex()
                .min_w_0()
                .child(div().text_color(cx.theme().muted_foreground).child(key))
                .child(
                    div()
                        .font_family(cx.theme().mono_font_family.clone())
                        .child(value),
                )
        }))
        .when(
            args.as_object().is_some_and(|args| args.len() > ARGUMENTS) || remaining == 0,
            |body| {
                body.child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child("Полные параметры — в раскрытом вызове"),
                )
            },
        )
}

/// A bounded nested JSON preview, including actual values rather than only
/// array/object lengths. Traversal stops as soon as its output budget ends.
fn preview(value: &Value, limit: usize) -> String {
    struct Preview {
        text: String,
        remaining: usize,
        nodes: usize,
    }
    impl Preview {
        fn push(&mut self, text: &str) {
            for ch in text.chars().take(self.remaining) {
                self.text.push(ch);
                self.remaining -= 1;
            }
        }
        fn value(&mut self, value: &Value, depth: usize) {
            if self.remaining == 0 {
                return;
            }
            if self.nodes == 0 || depth == 4 {
                self.push("…");
                return;
            }
            self.nodes -= 1;
            match value {
                Value::String(text) => {
                    let clipped: String = text.chars().take(self.remaining).collect();
                    self.push(&serde_json::to_string(&clipped).unwrap_or_default());
                    if clipped.len() < text.len() {
                        self.push("…");
                    }
                }
                Value::Array(values) => {
                    self.push("[");
                    for (index, value) in values.iter().take(3).enumerate() {
                        if index > 0 {
                            self.push(", ");
                        }
                        self.value(value, depth + 1);
                        if self.remaining == 0 {
                            break;
                        }
                    }
                    if values.len() > 3 {
                        self.push(", …");
                    }
                    self.push("]");
                }
                Value::Object(values) => {
                    self.push("{");
                    for (index, (key, value)) in values.iter().take(3).enumerate() {
                        if index > 0 {
                            self.push(", ");
                        }
                        self.value(&Value::String(key.chars().take(64).collect()), depth + 1);
                        self.push(": ");
                        self.value(value, depth + 1);
                        if self.remaining == 0 {
                            break;
                        }
                    }
                    if values.len() > 3 {
                        self.push(", …");
                    }
                    self.push("}");
                }
                _ => self.push(&value.to_string()),
            }
        }
    }
    let mut out = Preview {
        text: String::new(),
        remaining: limit,
        nodes: 24,
    };
    out.value(value, 0);
    if out.remaining == 0 && limit > 0 {
        out.text.pop();
        out.text.push('…');
    }
    out.text
}

#[cfg(test)]
mod tests {
    use super::preview;
    use serde_json::json;

    #[test]
    fn nested_parameters_show_values_and_unicode_stays_bounded() {
        let value = json!({"command": ["echo", {"cwd": "/папка", "flag": true}]});
        let text = preview(&value, 200);
        assert!(text.contains("echo"));
        assert!(text.contains("/папка"));
        assert!(text.contains("true"));
        assert!(
            preview(&json!({"text": "я".repeat(100_000)}), 80)
                .chars()
                .count()
                <= 80
        );
    }

    #[test]
    fn giant_collections_and_deep_values_do_not_expand_the_preview() {
        let value = Value::Array((0..100_000).map(|n| json!({"n": n})).collect());
        let text = preview(&value, 200);
        assert!(text.contains("\"n\": 0"));
        assert!(!text.contains("\"n\": 3"));
        assert!(text.contains('…'));
        assert_eq!(preview(&value, 0), "");
        let mut deep = json!("hidden");
        for _ in 0..16 {
            deep = json!({"nested": deep});
        }
        let text = preview(&deep, 200);
        assert!(text.contains('…'));
        assert!(!text.contains("hidden"));
    }
    use serde_json::Value;
}
