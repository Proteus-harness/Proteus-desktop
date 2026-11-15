//! Registry regressions. Fixtures do not call attach, render or reload.
//! Fixtures construct Runtime directly and explicitly register it; production
//! factory registration still needs separate source/integration verification.

use super::super::super::{
    Host, Installed, LiveRuntimes, Runtime, Runtimes, ViewRuntimes, live_runtimes,
    register_runtime, rehost_runtime, runtime_of,
};
use super::runtimes;
use crate::ui::extensions::host::Lifetime;
use crate::ui::workspace::Workspace;
use gpui_kit::*;
use std::collections::HashSet;

fn with_context(run: impl FnOnce(&mut Window, &mut App)) {
    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    cx.add_empty_window().update(|window, cx| run(window, cx));
}

fn fixture(
    extension: &'static str,
    lifetime: Lifetime,
    window: &mut Window,
    cx: &mut App,
) -> (Entity<Workspace>, Host, Entity<Runtime>) {
    let workspace = cx.new(|cx| Workspace::test_empty(window, cx));
    let host = Host::new(workspace.downgrade(), extension);
    let host = if lifetime == Lifetime::Workspace {
        host
    } else {
        host.isolated(lifetime)
    };
    let installed = Box::leak(Box::new(Installed {
        dir: "/synthetic-registry-fixture".into(),
        revision: 0,
        manifest: serde_json::from_value(serde_json::json!({
            "protocol": 4, "id": extension, "title": "Registry fixture",
            "command": ["never-start"], "surfaces": {"document": {}}
        }))
        .unwrap(),
    }));
    let runtime = cx.new(|cx| Runtime::new(installed, host.clone(), window, cx));
    register_runtime(&installed.manifest.id, &runtime, cx);
    if let Some(owner) = host.runtime_scope().owner {
        cx.default_global::<ViewRuntimes>()
            .0
            .insert((owner, extension), runtime.downgrade());
    } else {
        cx.default_global::<Runtimes>()
            .0
            .insert((host.identity(), extension), runtime.downgrade());
    }
    (workspace, host, runtime)
}

fn ids(runtimes: Vec<Entity<Runtime>>) -> HashSet<EntityId> {
    let count = runtimes.len();
    let ids: HashSet<_> = runtimes
        .into_iter()
        .map(|runtime| runtime.entity_id())
        .collect();
    assert_eq!(ids.len(), count, "each live recipient is returned once");
    ids
}

#[::core::prelude::v1::test]
fn rehost_keeps_displaced_recipient_and_changes_only_default_selection() {
    with_context(|window, cx| {
        let (_a, host_a, moved) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        let (_b, host_b, retained) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        assert_eq!(
            runtime_of(&host_b, cx).unwrap().entity_id(),
            retained.entity_id()
        );
        rehost_runtime(&moved, &host_b, cx);
        assert_eq!(
            runtime_of(&host_a, cx).unwrap().entity_id(),
            moved.entity_id()
        );
        assert_eq!(
            runtime_of(&host_b, cx).unwrap().entity_id(),
            moved.entity_id()
        );
        let expected = HashSet::from([moved.entity_id(), retained.entity_id()]);
        assert_eq!(ids(runtimes("registry-fixture", cx)), expected);
        assert_eq!(ids(live_runtimes(None, cx)), expected);
        // Repeated moves/aliases do not duplicate fanout or reload recipients.
        rehost_runtime(&moved, &host_b, cx);
        assert_eq!(ids(runtimes("registry-fixture", cx)), expected);
    });
}

#[::core::prelude::v1::test]
fn owned_views_and_other_packages_preserve_lookup_and_fanout_boundaries() {
    with_context(|window, cx| {
        let (_a, production_host, production) =
            fixture("registry-fixture", Lifetime::Workspace, window, cx);
        let (_b, settings_host, settings) =
            fixture("registry-fixture", Lifetime::Settings, window, cx);
        let (_c, preview_host, preview) =
            fixture("registry-fixture", Lifetime::Preview, window, cx);
        let (_d, foreign_host, foreign) = fixture("other-fixture", Lifetime::Workspace, window, cx);
        for (host, runtime) in [
            (&production_host, &production),
            (&settings_host, &settings),
            (&preview_host, &preview),
            (&foreign_host, &foreign),
        ] {
            assert_eq!(
                runtime_of(host, cx).unwrap().entity_id(),
                runtime.entity_id()
            );
        }
        assert_eq!(
            ids(runtimes("registry-fixture", cx)),
            HashSet::from([
                production.entity_id(),
                settings.entity_id(),
                preview.entity_id()
            ])
        );
        assert_eq!(
            ids(runtimes("other-fixture", cx)),
            HashSet::from([foreign.entity_id()])
        );
        assert!(runtimes("absent-fixture", cx).is_empty());
        assert_eq!(live_runtimes(None, cx).len(), 4);
    });
}

#[::core::prelude::v1::test]
fn registering_again_does_not_duplicate_or_change_defaults() {
    with_context(|window, cx| {
        let (_a, host_a, first) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        let (_b, host_b, second) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        register_runtime("registry-fixture", &first, cx);
        register_runtime("registry-fixture", &first, cx);
        assert_eq!(
            runtime_of(&host_a, cx).unwrap().entity_id(),
            first.entity_id()
        );
        assert_eq!(
            runtime_of(&host_b, cx).unwrap().entity_id(),
            second.entity_id()
        );
        assert_eq!(
            ids(runtimes("registry-fixture", cx)),
            HashSet::from([first.entity_id(), second.entity_id()])
        );
    });
}

#[::core::prelude::v1::test]
fn expired_weak_entries_are_skipped_and_pruned_without_removing_live_entries() {
    with_context(|window, cx| {
        let (_a, _host_a, old) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        let (_b, _host_b, live) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        // Deterministic expired-reference fixture, not a native GC timing test.
        cx.default_global::<LiveRuntimes>().0.insert(
            old.entity_id(),
            ("registry-fixture", WeakEntity::new_invalid()),
        );
        assert_eq!(
            ids(runtimes("registry-fixture", cx)),
            HashSet::from([live.entity_id()])
        );
        assert_eq!(cx.global::<LiveRuntimes>().0.len(), 2);
        let (_c, _host_c, new) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        assert!(!cx.global::<LiveRuntimes>().0.contains_key(&old.entity_id()));
        assert_eq!(
            ids(runtimes("registry-fixture", cx)),
            HashSet::from([live.entity_id(), new.entity_id()])
        );
        assert_eq!(cx.global::<LiveRuntimes>().0.len(), 2);
    });
}

#[::core::prelude::v1::test]
fn snapshots_release_the_global_borrow_before_subsequent_registration() {
    with_context(|window, cx| {
        let (_a, _host_a, first) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        let snapshot = live_runtimes(None, cx);
        let (_b, _host_b, second) = fixture("registry-fixture", Lifetime::Workspace, window, cx);
        assert_eq!(ids(snapshot), HashSet::from([first.entity_id()]));
        assert_eq!(
            ids(live_runtimes(None, cx)),
            HashSet::from([first.entity_id(), second.entity_id()])
        );
    });
}

#[::core::prelude::v1::test]
fn empty_registry_needs_no_default_global() {
    with_context(|_window, cx| {
        assert!(cx.try_global::<LiveRuntimes>().is_none());
        assert!(runtimes("absent-fixture", cx).is_empty());
        assert!(live_runtimes(None, cx).is_empty());
        assert!(cx.try_global::<LiveRuntimes>().is_none());
    });
}
