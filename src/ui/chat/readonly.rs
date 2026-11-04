//! A saved child conversation is a document, never another session writer.
//! Zed conversation_view::load_subagent_session supplies the lazy/deduplicated
//! navigation pattern; Proteus reads its existing journal page API instead.
//! Composer suppression follows Zed conversation_view/thread_view.rs::render_message_editor.
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::{AppSubagentSession, AppTranscriptCursor};
use serde::{Deserialize, Serialize};

use super::{Chat, ComposerExtras, Session};
use crate::api::Api;
use crate::ui::workspace::Workspace;

const PREFIX: &str = "subagent:";

/// Persist the exact binding with the document, including its workspace owner.
/// No catalog scan, thread UUID/path inference or role lookup is involved.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(in crate::ui) struct Target {
    pub owner: String,
    pub parent: String,
    /// A catalog supplies the durable session directly without a logical task id.
    pub child_thread_id: Option<String>,
    pub session: AppSubagentSession,
}

impl Target {
    pub fn id(&self) -> String {
        format!(
            "{PREFIX}{}",
            serde_json::to_string(self).expect("child document target")
        )
    }

    pub fn parse(id: &str) -> Option<Self> {
        let target: Self = serde_json::from_str(id.strip_prefix(PREFIX)?).ok()?;
        (crate::tabs::chat_key(&target.owner).is_some()
            && crate::tabs::chat_key(&target.parent).is_some()
            && target
                .child_thread_id
                .as_ref()
                .is_none_or(|id| !id.is_empty())
            && target.session.session_dir.is_absolute())
        .then_some(target)
    }
}

pub(super) struct Binding {
    pub target: Target,
    pub focus: FocusHandle,
    loaded: bool,
    dirty: bool,
    source: Option<String>,
    child_thread_id: Option<String>,
    error: Option<SharedString>,
    /// Physical HTTP read, independent of the history revision fence. Returning
    /// to the tail can invalidate a page before that network read has ended.
    inflight: Option<(u64, bool)>,
    refresh_pending: bool,
}

impl Chat {
    pub(in crate::ui) fn saved_subagent(
        api: Api,
        workspace: WeakEntity<Workspace>,
        target: Target,
        source: Option<String>,
        extras: ComposerExtras,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut chat = Self::base(
            Some(api),
            workspace,
            Session::new(target.session.session_dir.clone()),
            extras,
            window,
            cx,
        );
        let child_thread_id = target.child_thread_id.clone();
        chat.readonly = Some(Binding {
            target,
            focus: cx.focus_handle(),
            loaded: false,
            dirty: true,
            source,
            child_thread_id,
            error: None,
            inflight: None,
            refresh_pending: false,
        });
        chat._subscriptions.push(cx.subscribe(
            &cx.entity(),
            |this, _, event: &super::ChatEvent, cx| {
                let super::ChatEvent::OpenSubagent {
                    child_thread_id,
                    session,
                } = event
                else {
                    return;
                };
                let Some(binding) = &this.readonly else {
                    return;
                };
                let owner = binding.target.owner.clone();
                let parent = crate::tabs::chat(&binding.target.session.session_dir);
                this.workspace
                    .update(cx, |workspace, cx| {
                        workspace.defer_open_subagent(
                            owner,
                            parent,
                            child_thread_id.clone(),
                            session.clone(),
                            cx,
                        )
                    })
                    .ok();
            },
        ));
        // The first actual paint loads the saved tail. No pump, resume,
        // config/model request, runtime or composer contribution is started.
        chat
    }

    pub(in crate::ui) fn readonly_target(&self) -> Option<&Target> {
        self.readonly.as_ref().map(|binding| &binding.target)
    }

    pub(in crate::ui) fn readonly_child_thread_id(&self) -> Option<&str> {
        self.readonly.as_ref()?.child_thread_id.as_deref()
    }

    pub(in crate::ui) fn readonly_thread_bound(&mut self, child: Option<String>) {
        if let Some(child) = child
            && let Some(binding) = &mut self.readonly
        {
            binding.child_thread_id = Some(child);
        }
    }

    pub(in crate::ui) fn readonly_reopened(&mut self, cx: &mut Context<Self>) {
        if let Some(binding) = &mut self.readonly
            && binding.error.take().is_some()
        {
            binding.dirty = true;
            cx.notify();
        }
    }

    /// Parent lifecycle notifications coalesce and never poll a hidden document.
    pub(in crate::ui) fn readonly_source_changed(
        &mut self,
        source: Option<String>,
        force: bool,
        visible: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = &mut self.readonly else {
            return;
        };
        // A parent tail may evict the task row. Absence from that bounded
        // window is not a new lifecycle state or a reason to reread a final chat.
        let source = source
            .filter(|source| source != "unknown")
            .or_else(|| binding.source.clone());
        let terminal = source
            .as_deref()
            .is_some_and(|status| matches!(status, "completed" | "failed" | "cancelled" | "idle"));
        if binding.source == source && (!force || terminal) {
            return;
        }
        binding.source = source;
        binding.dirty = true;
        if visible {
            self.ensure_readonly_history(cx);
        }
    }

    pub(super) fn refresh_readonly(&mut self, cx: &mut Context<Self>) {
        if let Some(binding) = &mut self.readonly {
            binding.dirty = true;
        }
        // Explicit recovery of a rejected history cursor already lacked a
        // confirmed prefix. Ordinary background refresh never takes this path.
        self.show_latest(cx);
        self.ensure_readonly_history(cx);
    }

    pub(super) fn ensure_readonly_history(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = &self.readonly else {
            return;
        };
        if binding.inflight.is_some() {
            return;
        }
        // A seek queued after cancellation waits for the physical read to
        // return. Only its next visible paint resumes that pending gesture.
        if self.history.seeking() {
            self.load_history_seek(cx);
            return;
        }
        // A tail GET has no confirmed prefix for a newly changed revision.
        // Keep the page under the reader; refresh on their return to the tail.
        if self.history.archived() {
            return;
        }
        if !binding.dirty || self.history.request_pending() {
            self.schedule_readonly_refresh(cx);
            return;
        }
        let target = binding.target.clone();
        let Some(api) = self.api.clone() else {
            return;
        };
        let Some(request) = self.begin_history_request(true) else {
            return;
        };
        self.readonly.as_mut().unwrap().dirty = false;
        let document = target.id();
        let dir = target.session.session_dir;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.initial_history_page(&dir) })
                .await;
            this.update(cx, |this, cx| {
                if !this.finish_history_request(request, cx) {
                    return;
                }
                // A closed/replaced document cannot reappear. An in-flight
                // response can fill its retained hidden cache, but cannot
                // start a pending refresh until that document is visible.
                let Some(workspace) = this.workspace.upgrade() else {
                    return;
                };
                let workspace = workspace.read(cx);
                if !workspace.subagent_document_current(&document, cx.entity_id()) {
                    return;
                }
                let visible = workspace.subagent_document_visible(&document, cx.entity_id());
                match result {
                    Ok(page) if page.start + page.messages.len() == page.total => {
                        let cursor = AppTranscriptCursor {
                            revision: page.revision,
                            before: page.start,
                            base: page.base,
                        };
                        let (_, changes) = this.history.apply(
                            &mut this.session.transcript,
                            page.messages,
                            Some(cursor),
                        );
                        this.apply_history_changes(changes, cx);
                        let binding = this.readonly.as_mut().unwrap();
                        binding.loaded = true;
                        binding.error = None;
                    }
                    Ok(_) => {
                        this.readonly.as_mut().unwrap().error =
                            Some("Сервер вернул неполный конец истории".into())
                    }
                    Err(error) => {
                        this.readonly.as_mut().unwrap().error =
                            Some(format!("Не удалось прочитать историю: {error:#}").into())
                    }
                }
                if visible {
                    this.ensure_readonly_history(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn schedule_readonly_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(binding) = &mut self.readonly else {
            return;
        };
        if binding.refresh_pending || binding.inflight.is_some() || !binding.refreshing() {
            return;
        }
        binding.refresh_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::ui::motion::SAVED_CHAT_REFRESH)
                .await;
            this.update(cx, |this, cx| {
                let Some(binding) = &mut this.readonly else {
                    return;
                };
                binding.refresh_pending = false;
                let document = binding.target.id();
                let visible = this.workspace.upgrade().is_some_and(|workspace| {
                    workspace
                        .read(cx)
                        .subagent_document_visible(&document, cx.entity_id())
                });
                if visible && binding.refreshing() {
                    binding.dirty = true;
                    this.ensure_readonly_history(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn begin_history_request(&mut self, tail: bool) -> Option<u64> {
        if self
            .readonly
            .as_ref()
            .is_some_and(|binding| binding.inflight.is_some())
        {
            return None;
        }
        let request = self.history.begin_request()?;
        if let Some(binding) = &mut self.readonly {
            binding.inflight = Some((request, tail));
        }
        Some(request)
    }

    pub(super) fn finish_history_request(&mut self, request: u64, cx: &mut Context<Self>) -> bool {
        let tail = if let Some(binding) = &mut self.readonly {
            let Some((current, tail)) = binding.inflight else {
                return false;
            };
            if current != request {
                return false;
            }
            binding.inflight = None;
            tail
        } else {
            false
        };
        let accepted = self.history.finish_request(request);
        if let Some(binding) = &mut self.readonly {
            binding.dirty |= tail && !accepted;
            // Only a visible renderer may start the next pending read.
            cx.notify();
        }
        accepted
    }

    pub(super) fn render_readonly_header(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let binding = self.readonly.as_ref()?;
        let status = binding.error.clone().unwrap_or_else(|| {
            if self.history.request_pending() || binding.inflight.is_some() {
                "Чтение сохранённой истории…"
            } else if !binding.loaded {
                "История ещё не загружена"
            } else if self.display_transcript().entries().is_empty() {
                "Пока нет сохранённых сообщений"
            } else {
                "Сохранённый чат субагента · только просмотр"
            }
            .into()
        });
        Some(
            h_flex()
                .px_4()
                .py_2()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(div().flex_1().min_w_0().child(status)),
        )
    }
}

impl Binding {
    fn refreshing(&self) -> bool {
        matches!(
            self.source.as_deref(),
            Some("running" | "waiting_approval" | "waiting_input")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn saved_target_retains_exact_owner_and_directory_without_thread_inference() {
        let target = Target {
            owner: crate::tabs::chat(std::path::Path::new("/saved/parent")),
            parent: crate::tabs::chat(std::path::Path::new("/saved/parent")),
            child_thread_id: Some("logical-task".into()),
            session: AppSubagentSession {
                session_id: uuid::Uuid::from_u128(19),
                session_dir: "/elsewhere/child with spaces".into(),
            },
        };
        assert_eq!(Target::parse(&target.id()), Some(target));
        assert!(Target::parse("subagent:00000000-0000-0000-0000-000000000019").is_none());
    }

    #[::core::prelude::v1::test]
    fn refresh_timer_does_not_read_hidden_or_finished_documents() {
        let mut cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let window = cx.add_empty_window();
        let chat = window.update(|window, cx| {
            cx.new(|cx| {
                Chat::saved_subagent(
                    Api::new("http://127.0.0.1:1", "synthetic-never-used"),
                    WeakEntity::new_invalid(),
                    Target {
                        owner: crate::tabs::chat(std::path::Path::new("/saved/parent")),
                        parent: crate::tabs::chat(std::path::Path::new("/saved/parent")),
                        child_thread_id: None,
                        session: AppSubagentSession {
                            session_id: uuid::Uuid::from_u128(2),
                            session_dir: "/saved/child".into(),
                        },
                    },
                    Some("running".into()),
                    std::rc::Rc::new(|_, _, _| None),
                    window,
                    cx,
                )
            })
        });
        chat.update(&mut cx, |chat, cx| {
            chat.readonly.as_mut().unwrap().dirty = false;
            chat.readonly_thread_bound(Some("actual-task".into()));
            chat.readonly_thread_bound(None);
            assert_eq!(chat.readonly_child_thread_id(), Some("actual-task"));
            assert!(
                chat.readonly_target().unwrap().child_thread_id.is_none(),
                "learning task identity must not change the persisted document id"
            );
            chat.schedule_readonly_refresh(cx);
            chat.schedule_readonly_refresh(cx);
            assert!(chat.readonly.as_ref().unwrap().refresh_pending);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(crate::ui::motion::SAVED_CHAT_REFRESH);
        cx.run_until_parked();
        chat.update(&mut cx, |chat, cx| {
            let binding = chat.readonly.as_ref().unwrap();
            assert!(!binding.refresh_pending);
            assert!(!binding.dirty, "hidden timer cannot initiate history work");
            assert!(binding.inflight.is_none());
            chat.readonly_source_changed(Some("completed".into()), false, false, cx);
            chat.schedule_readonly_refresh(cx);
            assert!(
                !chat.readonly.as_ref().unwrap().refresh_pending,
                "finished history has no polling timer"
            );
            assert!(!chat.history.request_pending());
        });
    }

    #[::core::prelude::v1::test]
    fn saved_chat_is_lazy_without_pump_or_session_commands_and_retains_final_state() {
        let mut cx = ::gpui::TestAppContext::single();
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::ui::prefs::init_in_memory(cx);
        });
        let window = cx.add_empty_window();
        let parent = crate::tabs::chat(std::path::Path::new("/saved/parent"));
        let chat = window.update(|window, cx| {
            cx.new(|cx| {
                Chat::saved_subagent(
                    Api::new("http://127.0.0.1:1", "synthetic-never-used"),
                    WeakEntity::new_invalid(),
                    Target {
                        owner: parent.clone(),
                        parent,
                        child_thread_id: Some("logical-task".into()),
                        session: AppSubagentSession {
                            session_id: uuid::Uuid::from_u128(1),
                            session_dir: "/saved/child".into(),
                        },
                    },
                    Some("running".into()),
                    std::rc::Rc::new(|_, _, _| None),
                    window,
                    cx,
                )
            })
        });
        window.update(|_, cx| {
            chat.update(cx, |chat, cx| {
                assert!(
                    chat._pump.is_none(),
                    "saved document must not create child SSE"
                );
                assert!(!chat.supports_core());
                assert!(!chat.session.connected);
                assert!(
                    !chat.history.request_pending(),
                    "construction must not fetch child history"
                );
                chat.call(
                    cx,
                    |_| -> anyhow::Result<()> {
                        panic!("read-only command reached API");
                    },
                    |_, _, _| panic!("read-only command completed"),
                );
                chat.delete_queued(uuid::Uuid::from_u128(99), cx);
                // Revision cancellation releases the logical cursor but does not
                // start a second physical HTTP read while the first is returning.
                let request = chat.begin_history_request(true).unwrap();
                chat.show_latest(cx);
                assert!(chat.begin_history_request(false).is_none());
                assert!(!chat.finish_history_request(request, cx));
                assert!(chat.readonly.as_ref().unwrap().dirty);
                assert!(chat.readonly.as_ref().unwrap().inflight.is_none());
                // A hidden lifecycle update only marks the existing view dirty.
                chat.readonly.as_mut().unwrap().dirty = false;
                chat.readonly_source_changed(Some("completed".into()), false, false, cx);
                assert!(chat.readonly.as_ref().unwrap().dirty);
                assert!(!chat.history.request_pending());
                chat.readonly.as_mut().unwrap().dirty = false;
                chat.readonly_source_changed(Some("completed".into()), true, false, cx);
                assert!(
                    !chat.readonly.as_ref().unwrap().dirty,
                    "parent final signal must not reread already observed final status"
                );
                chat.readonly_source_changed(None, false, false, cx);
                assert_eq!(
                    chat.readonly.as_ref().unwrap().source.as_deref(),
                    Some("completed")
                );
                let binding = chat.readonly.as_mut().unwrap();
                binding.error = Some("synthetic read failure".into());
                binding.dirty = false;
                chat.readonly_reopened(cx);
                assert!(
                    chat.readonly.as_ref().unwrap().dirty,
                    "reopening a failed final history retries once without polling"
                );
                assert!(chat.readonly.as_ref().unwrap().error.is_none());
                assert!(!chat.history.request_pending());
            })
        });
        window.update(|window, cx| {
            chat.update(cx, |chat, cx| {
                // The mode guard remains decisive even if some future shared
                // renderer accidentally populates a connected execution state.
                chat.session.connected = true;
                chat.composer.update(cx, |composer, cx| {
                    composer.set_value("do not send", window, cx)
                });
                chat.send(window, cx);
                assert_eq!(chat.composer.read(cx).value().to_string(), "do not send");
                assert!(chat.session.feedback.visible_sends().next().is_none());
            })
        });
        cx.run_until_parked();
    }
}
