use super::*;

#[::core::prelude::v1::test]
fn old_removal_cannot_disable_a_replacement_before_its_registry_rescan() {
    let cx = ::gpui::TestAppContext::single();
    let id = "late-removal-race-fixture";
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("build/verification/package-races")
        .join(uuid::Uuid::new_v4().to_string());
    let path = root.join(id);
    let receipt = RemovedExtension {
        user_path: path.clone(),
        restored: None,
    };
    // Replacement is already published, but its worker has not rescanned yet.
    std::fs::create_dir_all(&path).unwrap();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
        prefs::update(cx, |prefs| {
            prefs.extensions.set_enabled(id, false);
            prefs.extensions.order.push(id.into());
            prefs.extensions.pinned.push(id.into());
        });
        assert_eq!(finish_removal(id, &receipt, cx).unwrap(), Some(false));
        assert!(!prefs::current(cx).extensions.is_enabled(id));
        assert!(
            prefs::current(cx)
                .extensions
                .order
                .iter()
                .any(|known| known == id)
        );
        assert!(
            prefs::current(cx)
                .extensions
                .pinned
                .iter()
                .any(|known| known == id)
        );
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[::core::prelude::v1::test]
fn native_removal_publication_waits_without_blocking_and_rechecks_afterward() {
    let cx = ::gpui::TestAppContext::single();
    let id = "deferred-removal-race-fixture";
    let receipt = RemovedExtension {
        user_path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("build/verification")
            .join(uuid::Uuid::new_v4().to_string()),
        restored: None,
    };
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
        prefs::update(cx, |prefs| prefs.extensions.order.push(id.into()));
        let guard = crate::packages::change_guard().unwrap();
        assert_eq!(finish_removal(id, &receipt, cx).unwrap(), None);
        assert!(
            prefs::current(cx)
                .extensions
                .order
                .iter()
                .any(|known| known == id)
        );
        drop(guard);
        assert_eq!(finish_removal(id, &receipt, cx).unwrap(), Some(true));
        assert!(
            !prefs::current(cx)
                .extensions
                .order
                .iter()
                .any(|known| known == id)
        );
    });
}
