use super::*;
use crate::settings::modes::{self, ModeList, ModeText};
use proteus_contracts::domain::{BehaviorModeDefinition, PermissionMode};

fn snapshot() -> ConfigBuilderSnapshot {
    ConfigBuilderSnapshot {
        writable: true,
        modes_writable: true,
        permission_mode: "normal".into(),
        modes: [(
            "review".into(),
            BehaviorModeDefinition {
                title: Some("Review".into()),
                instructions: Some(String::new()),
                tools: Some(vec![]),
                hooks: Some(vec!["unknown-hook".into()]),
                permission_mode: Some(PermissionMode::Plan),
            },
        )]
        .into(),
        ..Default::default()
    }
}

#[test]
fn mode_catalogue_roundtrip_crud_and_save_cas_do_not_touch_models_or_baseline_tools() {
    let snapshot = snapshot();
    let baseline = Draft::from_snapshot(&snapshot);
    let mut draft = baseline.clone();
    draft.rename_mode("review", "Custom / Ревью").unwrap();
    assert_eq!(draft.modes["Custom / Ревью"], baseline.modes["review"]);
    let request = draft.changes(&baseline);
    assert_eq!(request.expected_modes.as_ref(), Some(&baseline.modes));
    assert_eq!(request.modes.as_ref(), Some(&draft.modes));
    assert!(request.models.is_none() && request.model.is_none() && request.role_models.is_none());
    assert!(
        request.tools_enabled.is_none()
            && request.hooks.is_none()
            && request.permission_mode.is_none()
    );
    assert_eq!(draft.areas(&baseline), [Area::Modes]);
    assert_eq!(area_page(&Area::Modes), PageId::Modes);
    draft.rename_mode("Custom / Ревью", "review").unwrap();
    assert!(!draft.is_dirty(&baseline));
    draft.add_mode("new").unwrap();
    draft.modes.remove("new");
    assert!(!draft.is_dirty(&baseline));
    assert!(draft.add_mode("_default").is_err());
    assert!(draft.add_mode("  ").is_err());
    assert!(draft.add_mode("review").is_err());
}

#[test]
fn mode_text_undo_preserves_omitted_or_explicit_empty_and_unknown_ordered_lists() {
    for instructions in [None, Some(String::new())] {
        let baseline = BehaviorModeDefinition {
            instructions,
            hooks: Some(vec![" future hook ".into(), "unregistered".into()]),
            ..Default::default()
        };
        let mut edited = baseline.clone();
        modes::set_text(
            &mut edited,
            Some(&baseline),
            ModeText::Instructions,
            "extra",
        );
        modes::set_text(&mut edited, Some(&baseline), ModeText::Instructions, "");
        assert_eq!(edited, baseline);
        let mut draft = Draft::default();
        draft.modes.insert("x".into(), edited);
        assert!(modes::problems(&draft).is_empty());
        ModeList::Tools.set(draft.modes.get_mut("x").unwrap(), Some(vec![]));
        assert_eq!(draft.modes["x"].tools, Some(vec![]));
        ModeList::Tools.set(draft.modes.get_mut("x").unwrap(), None);
        assert_eq!(draft.modes["x"].tools, None);
        draft.modes.get_mut("x").unwrap().hooks =
            Some(vec!["duplicate".into(), "duplicate".into()]);
        assert!(!modes::problems(&draft).is_empty());
    }
}

#[test]
fn history_restores_owned_modes_but_never_replaces_external_catalogue() {
    let mut snapshot = snapshot();
    let mut state = ConfigBuilderState::default();
    state.permission_mode = "normal".into();
    state
        .modes
        .insert("old".into(), BehaviorModeDefinition::default());
    let restored = Draft::from_revision(&snapshot, &state);
    assert_eq!(restored.modes, state.modes);
    assert!(
        describe_changes(&snapshot, &Draft::from_snapshot(&snapshot), &restored)
            .iter()
            .any(|line| line.starts_with("Режимы:"))
    );
    snapshot.modes_writable = false;
    assert_eq!(
        Draft::from_revision(&snapshot, &state).modes,
        snapshot.modes
    );
}
