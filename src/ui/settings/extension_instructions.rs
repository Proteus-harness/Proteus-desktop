//! A renderer's packaged instructions are for the person configuring it.
//! The agent's catalog remains the sole source of Skills navigation targets.

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::app_protocol::addons::AppAddonsSnapshot;

use super::SettingsView;
use super::spot::{self, Destination};
use super::widgets::{Tone, card, chip, hashed_id, item_row, mono, note, section, settings_action};
use crate::declarative::manifest::AgentInstruction;
use crate::settings::catalog::PageId;
use crate::ui::extensions::{Manifest, content::FormattedText};
use crate::ui::motion::{self, Spinner};

pub(super) struct Instructions {
    extension: &'static str,
    skill: &'static str,
    request: uuid::Uuid,
    open: bool,
    /// Cancels a read on collapse, replacement, page navigation or view drop.
    _task: Option<Task<()>>,
    content: Content,
}

enum Content {
    Loading,
    Ready {
        view: Entity<FormattedText>,
        _changes: Subscription,
    },
    Failed(SharedString),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Availability {
    Found,
    Disabled,
    Missing,
    Unknown,
}

impl Availability {
    fn label(self) -> &'static str {
        match self {
            Self::Found => "Найден в Skills",
            Self::Disabled => "Выключен в Skills",
            Self::Missing => "Не найден в Skills",
            Self::Unknown => "Статус Skills неизвестен",
        }
    }

    fn known(self) -> bool {
        matches!(self, Self::Found | Self::Disabled)
    }
}

fn availability(id: &str, addons: Option<&AppAddonsSnapshot>, allowed: bool) -> Availability {
    let Some(addons) = addons else {
        return Availability::Unknown;
    };
    let mut found = false;
    let mut enabled = false;
    for (_, skill) in crate::settings::addons::skills(addons) {
        if skill.id == id {
            found = true;
            enabled |= skill.enabled;
        }
    }
    if found {
        return if allowed && enabled {
            Availability::Found
        } else {
            Availability::Disabled
        };
    }
    if addons.catalogs.is_empty()
        || addons
            .catalogs
            .iter()
            .any(|catalog| catalog.error.is_some() || catalog.catalog.is_none())
    {
        Availability::Unknown
    } else {
        Availability::Missing
    }
}

fn destination(id: &str, status: Availability) -> Destination {
    let target = Destination::new(PageId::Skills);
    if status.known() {
        target.spot(spot::skill(id))
    } else {
        target
    }
}

fn reveal_key(extension: &str, skill: &str) -> String {
    format!("extension-instructions:{extension}:{skill}")
}

impl SettingsView {
    pub(super) fn extension_instructions_section(
        &self,
        manifest: &'static Manifest,
        cx: &Context<Self>,
    ) -> AnyElement {
        let rows = manifest.agent_instructions.iter().map(|instruction| {
            let status = availability(
                &instruction.skill,
                self.addons(),
                self.draft.skill_enabled(&instruction.skill),
            );
            let selected = self
                .extension_instructions
                .as_ref()
                .filter(|view| view.extension == manifest.id && view.skill == instruction.skill);
            let open = selected.is_some_and(|view| view.open);
            let key = reveal_key(manifest.id, &instruction.skill);
            let action = settings_action(
                hashed_id("extension-instructions-read", &key),
                if open {
                    "Скрыть инструкцию"
                } else {
                    "Читать инструкцию"
                },
                cx,
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.extension_instructions.as_ref().is_some_and(|view| {
                    view.extension == manifest.id && view.skill == instruction.skill && view.open
                }) {
                    if let Some(view) = &mut this.extension_instructions {
                        view.open = false;
                        view._task = None;
                    }
                    motion::toggled(super::reveal_key(&key), false, cx);
                    // The disappearing document may own keyboard focus.
                    this.page_focus.focus(window, cx);
                    cx.notify();
                } else {
                    this.read_extension_instructions(manifest, instruction, cx);
                }
            }));
            let body = motion::reveal(
                super::reveal_key(&reveal_key(manifest.id, &instruction.skill)),
                open,
                cx,
                || match selected.map(|view| &view.content) {
                    Some(Content::Ready { view, .. }) => div()
                        .px_4()
                        .pb_3()
                        .min_w_0()
                        .child(view.clone())
                        .into_any_element(),
                    Some(Content::Failed(error)) => {
                        note(format!("Не удалось прочитать инструкцию: {error}"), cx)
                            .text_color(cx.theme().danger)
                            .into_any_element()
                    }
                    _ => h_flex()
                        .px_4()
                        .py_3()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(Spinner::new())
                        .child("Читаю инструкцию…")
                        .into_any_element(),
                },
            );
            v_flex()
                .w_full()
                .min_w_0()
                .child(item_row(
                    mono(instruction.skill.clone(), cx),
                    [
                        chip(status.label(), Tone::Muted, cx).into_any_element(),
                        self.settings_link(
                            hashed_id("extension-instructions-skill", &instruction.skill),
                            "Skills",
                            destination(&instruction.skill, status),
                            cx,
                        ),
                    ],
                ))
                .child(div().px_4().pb_2().child(action))
                .children(body)
                .into_any_element()
        });
        section(
            "Skill для агента",
            Some("Установка расширения не подключает skill к агенту.".into()),
            None,
            card(rows, cx),
            cx,
        )
        .into_any_element()
    }

    fn read_extension_instructions(
        &mut self,
        manifest: &'static Manifest,
        instruction: &'static AgentInstruction,
        cx: &mut Context<Self>,
    ) {
        let request = uuid::Uuid::new_v4();
        self.extension_instructions = Some(Instructions {
            extension: manifest.id,
            skill: &instruction.skill,
            request,
            open: true,
            _task: None,
            content: Content::Loading,
        });
        let key = reveal_key(manifest.id, &instruction.skill);
        motion::toggled(super::reveal_key(&key), true, cx);
        let task = cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::catalog::schema::read_skill(manifest.package_dir, &instruction.path)
                })
                .await;
            this.update(cx, |this, cx| {
                // A collapse followed by reopening the same skill is a new read.
                // Never let the old answer replace it or resurrect an old page.
                if this.page != PageId::Extension(manifest.id.to_owned())
                    || !this
                        .extension_instructions
                        .as_ref()
                        .is_some_and(|view| view.request == request && view.open)
                {
                    return;
                }
                let content = match result {
                    Ok(text) => {
                        let view = cx.new(|_| {
                            FormattedText::new(this.workspace.clone(), "text/markdown", text)
                        });
                        let changes = cx.observe(&view, |_, _, cx| cx.notify());
                        Content::Ready {
                            view,
                            _changes: changes,
                        }
                    }
                    Err(error) => Content::Failed(format!("{error:#}").into()),
                };
                this.extension_instructions.as_mut().unwrap().content = content;
                cx.notify();
            })
            .ok();
        });
        self.extension_instructions.as_mut().unwrap()._task = Some(task);
        cx.notify();
    }

    /// The reader survives only the common closing animation, never a page
    /// switch. There is no retained document or background read once closed.
    pub(super) fn release_closed_instructions(&mut self, cx: &App) {
        if self.extension_instructions.as_ref().is_some_and(|view| {
            !view.open
                && !motion::moving(
                    &super::reveal_key(&reveal_key(view.extension, view.skill)),
                    false,
                    cx,
                )
        }) {
            self.extension_instructions = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn snapshot(id: &str, enabled: bool) -> AppAddonsSnapshot {
        serde_json::from_value(json!({
            "reload_error": null,
            "writable": true,
            "settings": {
                "addons": { "disabled_skills": [], "disabled_mcp_servers": [], "plugins": [] },
                "mcp_servers": []
            },
            "catalogs": [{
                "provider": "skills",
                "catalog": {
                    "skills": [{
                        "id": id, "name": id, "description": "", "path": "/skills/example",
                        "source": "project", "enabled": enabled
                    }],
                    "warnings": []
                },
                "error": null
            }],
            "mcp_servers": [],
            "plugins": []
        }))
        .unwrap()
    }

    #[::core::prelude::v1::test]
    fn availability_matches_the_exact_catalog_id_and_both_disabled_states() {
        let mut addons = snapshot("team:visualize", true);
        assert_eq!(
            availability("team:visualize", Some(&addons), true),
            Availability::Found
        );
        assert_eq!(
            availability("visualize", Some(&addons), true),
            Availability::Missing
        );
        assert_eq!(
            availability("team:visualize", Some(&addons), false),
            Availability::Disabled
        );
        addons.catalogs[0].catalog.as_mut().unwrap().skills[0].enabled = false;
        assert_eq!(
            availability("team:visualize", Some(&addons), true),
            Availability::Disabled
        );
    }

    #[::core::prelude::v1::test]
    fn failed_provider_or_missing_catalog_is_unknown_instead_of_absent() {
        let mut addons = snapshot("other", true);
        addons.catalogs[0].catalog = None;
        addons.catalogs[0].error = Some("offline".into());
        assert_eq!(
            availability("visualize", Some(&addons), true),
            Availability::Unknown
        );
        addons.catalogs.clear();
        assert_eq!(
            availability("visualize", Some(&addons), true),
            Availability::Unknown
        );
    }

    #[::core::prelude::v1::test]
    fn an_unavailable_catalog_never_invents_a_skill_anchor() {
        assert_eq!(availability("visualize", None, true), Availability::Unknown);
        for status in [Availability::Unknown, Availability::Missing] {
            let target = destination("visualize", status);
            assert_eq!(target.page, PageId::Skills);
            assert_eq!(target.spot, None);
        }
        for status in [Availability::Found, Availability::Disabled] {
            assert_eq!(
                destination("visualize", status).spot,
                Some(spot::skill("visualize"))
            );
        }
    }
}
