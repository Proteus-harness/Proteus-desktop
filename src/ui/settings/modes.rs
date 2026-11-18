// Адаптировано из Zed: crates/settings_ui/src/settings_ui.rs (editable rows).
//! Named behavior definitions are saved to the agent config; selecting one is
//! a separate session-setting operation performed by the mode extension.

mod actions;

use std::rc::Rc;

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::domain::{
    BehaviorModeDefinition, ConfigField, ConfigValueSchema, PermissionMode,
};
use serde_json::Value;

use super::widgets::{
    Fit, MenuOption, Tone, card, chip, disclosure, field_row, hashed_id, note, row, select_menu,
};
use super::{
    SettingsView,
    tags::editor::{self, TagEditor},
};
use crate::settings::modes::{self, ModeList};
use crate::ui::fade::faded;
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use crate::ui::prefs;

pub(super) fn mode_key(id: &str) -> String {
    format!("behavior-mode:{id}")
}

impl SettingsView {
    pub(super) fn modes_writable(&self) -> bool {
        self.snapshot()
            .is_some_and(|snapshot| snapshot.writable && snapshot.modes_writable)
    }

    pub(super) fn modes_page(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let editable = self.modes_writable();
        let mut body = v_flex().gap_3().children(
            self.draft
                .modes
                .iter()
                .map(|(id, mode)| self.behavior_card(id, mode, editable, cx)),
        );
        if !editable {
            body = body.child(note(
                if self.snapshot().is_some_and(|snapshot| snapshot.writable) {
                    "Режимы редактируются в исходном конфиге."
                } else {
                    "Профиль доступен только для чтения."
                },
                cx,
            ));
        } else if let Some(input) = &self.editors.modes.name {
            let name = input.read(cx).value();
            let problem = modes::id_problem(&self.draft, &name, None);
            body = body.child(row(
                "Новый режим",
                None,
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .max_w_full()
                    .flex_wrap()
                    .child(
                        div()
                            .w(prefs::scaled(220.))
                            .min_w_0()
                            .max_w_full()
                            .child(kit::input(input).small().w_full()),
                    )
                    .child(
                        kit::button("behavior-add")
                            .faded_label("Добавить")
                            .small()
                            .disabled(problem.is_some())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_behavior_mode(window, cx)
                            })),
                    ),
                false,
                cx,
            ));
            if !name.is_empty()
                && let Some(problem) = problem
            {
                body = body.child(note(problem, cx));
            }
        }
        let problems = modes::problems(&self.draft);
        if !problems.is_empty() {
            body =
                body.child(v_flex().gap_1().children(
                    problems.into_iter().map(|problem| {
                        div().text_sm().text_color(cx.theme().danger).child(problem)
                    }),
                ));
        }
        vec![body.into_any_element()]
    }

    fn behavior_card(
        &self,
        id: &str,
        mode: &BehaviorModeDefinition,
        editable: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let key = mode_key(id);
        let title = mode.title.as_deref().unwrap_or(id).to_owned();
        let target = id.to_owned();
        let delete = kit::icon_button(hashed_id("behavior-remove", &id))
            .icon(Icon::new(Lucide::Trash))
            .quiet(cx)
            .small()
            .accessibility_label(format!("Удалить режим {id}"))
            .hint("Удалить режим", cx)
            .disabled(!editable)
            .on_click(cx.listener(move |this, _, _, cx| this.remove_behavior_mode(&target, cx)));
        let view = cx.weak_entity();
        let toggle = key.clone();
        let header = div().px_4().py_3().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().child(disclosure(
                    hashed_id("behavior-toggle", &id),
                    self.is_open(&key),
                    title,
                    false,
                    move |cx| {
                        let _ = view.update(cx, |this, cx| this.toggle_open(&toggle, cx));
                    },
                    cx,
                )))
                .child(delete),
        );
        let content = self.reveal(&key, cx, || {
            v_flex()
                .children(self.behavior_fields(id, mode, editable, cx))
                .into_any_element()
        });
        self.spotted(
            &key,
            card([header.into_any_element()].into_iter().chain(content), cx),
            cx,
        )
        .into_any_element()
    }

    fn behavior_fields(
        &self,
        id: &str,
        mode: &BehaviorModeDefinition,
        editable: bool,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        if !editable {
            let mut rows = vec![
                row(
                    "Имя",
                    None,
                    div().text_sm().child(faded(id.to_owned())),
                    false,
                    cx,
                )
                .into_any_element(),
            ];
            if let Some(title) = &mode.title {
                rows.push(
                    row(
                        "Название",
                        None,
                        div().text_sm().child(faded(title.clone())),
                        false,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            if let Some(text) = &mode.instructions {
                rows.push(
                    row(
                        "Инструкции",
                        None,
                        div().text_sm().child(text.clone()),
                        true,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            for kind in [ModeList::Tools, ModeList::Hooks] {
                let value = kind.values(mode).map_or_else(
                    || "Как в конфиге".into(),
                    |items| {
                        if items.is_empty() {
                            "Пустой список".into()
                        } else {
                            items.join(", ")
                        }
                    },
                );
                rows.push(
                    row(kind.key(), None, div().text_sm().child(value), false, cx)
                        .into_any_element(),
                );
            }
            if let Some(permission) = &mode.permission_mode {
                rows.push(
                    row(
                        "Разрешения",
                        None,
                        chip(permission_title(permission), Tone::Muted, cx),
                        false,
                        cx,
                    )
                    .into_any_element(),
                );
            }
            return rows;
        }
        let Some(fields) = self.editors.modes.fields.get(id) else {
            return Vec::new();
        };
        let renamed = fields.id.read(cx).value();
        let problem = modes::id_problem(&self.draft, &renamed, Some(id));
        let target = id.to_owned();
        let mut rows = vec![
            row(
                "Имя",
                None,
                h_flex()
                    .gap_2()
                    .min_w_0()
                    .max_w_full()
                    .flex_wrap()
                    .child(
                        div()
                            .w(prefs::scaled(220.))
                            .min_w_0()
                            .max_w_full()
                            .child(kit::input(&fields.id).small().w_full()),
                    )
                    .child(
                        kit::button(hashed_id("behavior-rename", &id))
                            .faded_label("Переименовать")
                            .small()
                            .disabled(renamed == id || problem.is_some())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.rename_behavior_mode(&target, window, cx)
                            })),
                    ),
                false,
                cx,
            )
            .into_any_element(),
            field_row(
                "Название",
                kit::input(&fields.title).small().w_full(),
                Fit::Fill,
            )
            .into_any_element(),
            field_row(
                div().text_sm().child("Дополнительные инструкции"),
                self.text_area(&fields.instructions, cx),
                Fit::Below,
            )
            .into_any_element(),
        ];
        if renamed != id
            && let Some(problem) = problem
        {
            rows.push(note(problem, cx).into_any_element());
        }
        for kind in [ModeList::Tools, ModeList::Hooks] {
            rows.extend(self.behavior_list(id, mode, kind, cx));
        }
        let options = [
            (None, "Как в конфиге"),
            (Some(PermissionMode::Normal), "С подтверждением"),
            (Some(PermissionMode::Auto), "Автоматически"),
            (Some(PermissionMode::Plan), "Только чтение"),
        ];
        let label = mode
            .permission_mode
            .as_ref()
            .map_or("Как в конфиге", permission_title);
        let view = cx.weak_entity();
        let target = id.to_owned();
        rows.push(
            row(
                "Разрешения",
                None,
                select_menu(
                    hashed_id("behavior-permission", &id),
                    label,
                    options
                        .into_iter()
                        .map(|(value, label)| MenuOption {
                            checked: value == mode.permission_mode,
                            label: label.into(),
                            value,
                        })
                        .collect(),
                    move |value, cx| {
                        let _ = view.update(cx, |this, cx| {
                            this.set_behavior_permission(&target, value, cx);
                        });
                    },
                ),
                false,
                cx,
            )
            .into_any_element(),
        );
        rows
    }

    fn behavior_list(
        &self,
        id: &str,
        mode: &BehaviorModeDefinition,
        kind: ModeList,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let overridden = kind.values(mode).is_some();
        let view = cx.weak_entity();
        let target = id.to_owned();
        let choice = select_menu(
            hashed_id("behavior-list-source", &(id, kind)),
            if overridden {
                "Свой список"
            } else {
                "Как в конфиге"
            },
            [
                MenuOption {
                    label: "Как в конфиге".into(),
                    value: false,
                    checked: !overridden,
                },
                MenuOption {
                    label: "Свой список".into(),
                    value: true,
                    checked: overridden,
                },
            ]
            .into(),
            move |override_list, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.override_behavior_list(&target, kind, override_list, cx)
                });
            },
        );
        let mut rows = vec![
            row(
                match kind {
                    ModeList::Tools => "Tools",
                    ModeList::Hooks => "Hooks",
                },
                None,
                choice,
                false,
                cx,
            )
            .into_any_element(),
        ];
        if let Some(values) = kind.values(mode) {
            if values.is_empty() {
                rows.push(
                    note(
                        match kind {
                            ModeList::Tools => "Инструменты отключены.",
                            ModeList::Hooks => "Hooks отключены.",
                        },
                        cx,
                    )
                    .into_any_element(),
                );
            }
            let input = self
                .editors
                .modes
                .fields
                .get(id)
                .and_then(|fields| fields.queries.get(&kind))
                .cloned();
            let field = ConfigField::new(kind.key(), kind.key(), "", ConfigValueSchema::strings());
            let field = match kind {
                ModeList::Tools => field.suggest_tools(),
                ModeList::Hooks => field.suggest_exports("hook"),
            };
            let view = cx.weak_entity();
            let target = id.to_owned();
            rows.push(
                div()
                    .px_4()
                    .pb_3()
                    .child(editor::render(
                        Rc::new(TagEditor {
                            key: format!("behavior:{id}:{}", kind.key()),
                            values: values.iter().cloned().map(Value::String).collect(),
                            field,
                            input,
                            revision: self.profile_revision.rotate_left(32) ^ self.edit_revision,
                            apply: Rc::new(move |expected, edit, cx| {
                                view.update(cx, |this, cx| {
                                    this.apply_behavior_tag_edit(&target, kind, expected, edit, cx)
                                })
                                .unwrap_or(false)
                            }),
                        }),
                        self.snapshot(),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        rows
    }
}
fn permission_title(mode: &PermissionMode) -> &'static str {
    match mode {
        PermissionMode::Normal => "С подтверждением",
        PermissionMode::Auto => "Автоматически",
        PermissionMode::Plan => "Только чтение",
    }
}
