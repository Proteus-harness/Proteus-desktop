use super::*;
use crate::ui::settings::SettingsContext;

#[::core::prelude::v1::test]
fn modules_targets_open_their_owner_without_locking_it_open() {
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
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
            view.accept(ConfigBuilderSnapshot {
                permission_mode: "normal".into(),
                plugins: vec![
                    ConfigBuilderPlugin {
                        id: "first".into(),
                        command: "synthetic".into(),
                        description: None,
                        exports: vec![ConfigBuilderPluginExport {
                            slot: HOOK_SLOT.into(),
                            id: "first-hook".into(),
                            active: false,
                            description: None,
                            config_schema: None,
                        }],
                        tool_packs: vec![ConfigBuilderToolPack {
                            id: "tools".into(),
                            tools: vec!["synthetic-tool".into()],
                        }],
                    },
                    ConfigBuilderPlugin {
                        id: "first-extra".into(),
                        command: "synthetic".into(),
                        description: None,
                        exports: vec![ConfigBuilderPluginExport {
                            slot: "workflow".into(),
                            id: "first-workflow".into(),
                            active: false,
                            description: None,
                            config_schema: None,
                        }],
                        tool_packs: vec![],
                    },
                ],
                ..Default::default()
            });
            assert!(view.open.is_empty());
            for (target, owner) in [
                (spot::component("first-extra"), "first-extra"),
                (pack_key("first", "tools"), "first"),
                (spot::export(HOOK_SLOT, "first-hook"), "first"),
                (spot::export("workflow", "first-workflow"), "first-extra"),
            ] {
                view.open.clear();
                view.go_to(&Destination::new(PageId::Modules).spot(target), window, cx);
                let key = module_key(owner);
                assert_eq!(view.open, [key.clone()].into_iter().collect());
                assert_eq!(view.page, PageId::Modules);
                view.toggle_open(&key, cx);
                assert!(!view.is_open(&key), "a highlighted module can still close");
            }
            view.open.clear();
            view.go_to(
                &Destination::new(PageId::Modules).spot(pack_key("first-extra", "tools")),
                window,
                cx,
            );
            assert!(
                view.open.is_empty(),
                "a similar ID is not a configured owner"
            );
            view.go_to(
                &Destination::new(PageId::Tools).spot(pack_key("first", "tools")),
                window,
                cx,
            );
            assert!(view.open.is_empty(), "another page does not unfold Modules");
        });
    });
}
