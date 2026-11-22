use super::*;
use crate::settings::FieldPath;
use crate::settings::catalog::{POLICY_SLOT, PageId};
use crate::ui::settings::{slots, spot};
use proteus_contracts::app_protocol::config_builder::{ConfigBuilderModule, ConfigBuilderSlot};
use proteus_contracts::domain::{ConfigField, ConfigValueSchema, ModuleConfigSchema};

#[::core::prelude::v1::test]
fn policy_rules_start_closed_and_search_unfolds_the_implementation_and_form_or_json() {
    for json in [false, true] {
        let mut cx = context();
        let window = cx.add_empty_window();
        let view = window.update(|window, cx| {
            cx.new(|cx| {
                SettingsView::new(
                    WeakEntity::new_invalid(),
                    SettingsContext::for_chat(None, || None),
                    window,
                    cx,
                )
            })
        });
        window.update(|window, cx| {
            view.update(cx, |view, cx| {
                let mut snapshot = snapshot("synthetic");
                snapshot.slots.push(ConfigBuilderSlot {
                    id: POLICY_SLOT.into(),
                    active_module: Some("fixture-policy".into()),
                    modules: vec![ConfigBuilderModule {
                        id: "fixture-policy".into(),
                        slot: POLICY_SLOT.into(),
                        config_schema: Some(ModuleConfigSchema {
                            fields: vec![ConfigField::new(
                                "limit",
                                "Предел вызовов",
                                "",
                                ConfigValueSchema::integer(0),
                            )],
                        }),
                        ..Default::default()
                    }],
                    ..Default::default()
                });
                view.accept(snapshot);
                view.page = PageId::Modes;
                view.prepare_editors(window, cx);
                assert!(!view.is_open(slots::POLICY_RULES));
                assert!(
                    view.editors.fields.is_empty(),
                    "closed rules do not prepare field editors"
                );
                if json {
                    view.json
                        .insert((POLICY_SLOT.into(), "fixture-policy".into()));
                }
                for (query, anchor) in [
                    (
                        "fixture-policy",
                        spot::module(POLICY_SLOT, "fixture-policy"),
                    ),
                    (
                        "Предел вызовов",
                        if json {
                            spot::params_json(POLICY_SLOT, "fixture-policy")
                        } else {
                            spot::field(&FieldPath::new(POLICY_SLOT, "fixture-policy", "limit"))
                        },
                    ),
                ] {
                    let hit = view
                        .search(query, cx)
                        .into_iter()
                        .find(|hit| hit.target.spot.as_deref() == Some(anchor.as_str()))
                        .expect("search must index hidden rules");
                    assert!(hit.target.open.iter().any(|key| key == slots::POLICY_RULES));
                    assert_eq!(hit.target.page, PageId::Modes);
                    view.go_to(&hit.target, window, cx);
                    assert!(view.is_open(slots::POLICY_RULES));
                    view.prepare_editors(window, cx);
                    view.open.remove(slots::POLICY_RULES);
                }
                if json {
                    assert!(
                        view.editors
                            .json
                            .contains_key(&(POLICY_SLOT.into(), "fixture-policy".into()))
                    );
                } else {
                    assert!(view.editors.fields.contains_key(&FieldPath::new(
                        POLICY_SLOT,
                        "fixture-policy",
                        "limit"
                    )));
                }
            })
        });
    }
}
