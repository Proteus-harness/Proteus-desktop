use super::{Host, Lifetime};
use crate::tabs::ChatKey;
use gpui_kit::WeakEntity;

#[test]
fn local_manifest_scope_cannot_be_widened_by_binding_a_core_chat() {
    let mut host = Host::new(WeakEntity::new_invalid(), "local-test");
    // The same scope set by Host::new from dependencies.core=false.
    host.runtime_scope.content_only = true;
    let bound = host.for_chat(
        WeakEntity::new_invalid(),
        ChatKey::Proteus("/sessions/core".into()),
    );
    assert!(bound.runtime_scope().content_only);
    assert_eq!(
        bound.context_session(|| panic!("local host read active chat")),
        None
    );
    let child = bound.with_extension("core-required-child");
    assert_eq!(
        child.context_session(|| panic!("child widened local host")),
        None
    );
    assert!(child.runtime_scope().content_only);
}

#[test]
fn external_bound_context_cannot_fall_back_to_a_core_directory() {
    let host = Host::new(WeakEntity::new_invalid(), "renderer").for_chat(
        WeakEntity::new_invalid(),
        ChatKey::Codex("opaque:codex".into()),
    );
    assert!(host.runtime_scope().content_only);
    assert_eq!(
        host.context_session(|| panic!("bound Codex metadata read workspace/chat")),
        None
    );
    let (server, _) = host.runtime_scope().connection(
        || panic!("Codex content read Core credentials"),
        || Some("/project".into()),
    );
    let server = server.unwrap();
    assert!(server.token.is_empty());
    assert!(server.origin.is_empty());
}

#[test]
fn bound_directory_survives_the_chat_entity_and_dynamic_root_has_no_stale_fallback() {
    let mut inherited = Host::new(WeakEntity::new_invalid(), "files");
    inherited.inherit(Some((
        WeakEntity::new_invalid(),
        ChatKey::Proteus("/sessions/b".into()),
    )));
    assert_eq!(inherited.context_session(|| None), None);
    assert_eq!(
        inherited.context_session(|| Some(ChatKey::Proteus("/sessions/a".into()))),
        Some("/sessions/a".into())
    );

    assert_eq!(
        inherited.context_session(|| Some(ChatKey::Codex("active:external".into()))),
        None,
        "active external driver must suppress an inherited Core directory"
    );

    let bound = inherited.for_chat(
        WeakEntity::new_invalid(),
        ChatKey::Proteus("/sessions/bound".into()),
    );
    assert_eq!(
        bound.context_session(|| panic!("bound Core metadata read workspace/chat")),
        Some("/sessions/bound".into())
    );
    assert_eq!(
        bound.context_session(|| Some(ChatKey::Proteus("/sessions/a".into()))),
        Some("/sessions/bound".into())
    );
}

#[test]
fn content_inline_and_rehost_clones_cannot_escape_the_view_owner() {
    for lifetime in [Lifetime::Settings, Lifetime::Preview] {
        let production = Host::new(WeakEntity::new_invalid(), "renderer");
        assert!(production.retains_process());
        let parent = production.clone().isolated(lifetime);
        let child = parent.with_extension("nested-renderer");
        let inline = child
            .clone()
            .with_extension("inline-renderer")
            .rehost(WeakEntity::new_invalid());
        for host in [&parent, &child, &inline] {
            assert_eq!(host.runtime_scope(), parent.runtime_scope());
            assert_eq!(host.scope(), parent.scope());
            assert!(
                !host.retains_process(),
                "nested runtime entered Workspace.extension_processes"
            );
            assert!(host.runtime_scope().lifetime.allowed(false));
            host.runtime_scope().connection(
                || panic!("nested host accessed production server"),
                || panic!("nested host accessed production folder"),
            );
        }
        assert_ne!(
            parent.runtime_scope().owner,
            production.isolated(lifetime).runtime_scope().owner
        );
    }
}

#[test]
fn retained_space_context_ignores_other_active_and_inherited_roots() {
    let root = ChatKey::Proteus("/sessions/space-a".into());
    let mut host = Host::new(WeakEntity::new_invalid(), "files").for_space(root.clone());
    host.inherit(Some((
        WeakEntity::new_invalid(),
        ChatKey::Proteus("/sessions/inherited".into()),
    )));
    assert!(host.inherited.is_none());
    assert_eq!(
        host.context_key(|| panic!("retained document read displayed root")),
        Some(root)
    );
    assert_eq!(
        host.context_session(|| Some(ChatKey::Proteus("/sessions/space-b".into()))),
        Some("/sessions/space-a".into())
    );
    assert_eq!(
        host.context_session(|| None),
        Some("/sessions/space-a".into())
    );
}

#[test]
fn retained_codex_space_and_local_scope_cannot_read_core_context() {
    let codex =
        Host::new(WeakEntity::new_invalid(), "files").for_space(ChatKey::Codex("same-id".into()));
    assert!(codex.runtime_scope().content_only);
    assert_eq!(
        codex.context_session(|| panic!("Codex space read current Core root")),
        None
    );
    let mut local = Host::new(WeakEntity::new_invalid(), "terminal");
    local.runtime_scope.content_only = true;
    let local = local.for_space(ChatKey::Proteus("/sessions/core".into()));
    assert_eq!(
        local.context_session(|| panic!("local space read current Core root")),
        None
    );
    assert!(local.with_extension("files").runtime_scope().content_only);
}

#[test]
fn bound_child_context_wins_and_clears_space_pin() {
    let host = Host::new(WeakEntity::new_invalid(), "renderer")
        .for_space(ChatKey::Proteus("/sessions/root".into()))
        .for_chat(
            WeakEntity::new_invalid(),
            ChatKey::Proteus("/sessions/child".into()),
        );
    assert!(host.space.is_none());
    assert_eq!(
        host.context_session(|| panic!("bound child read workspace root")),
        Some("/sessions/child".into())
    );
}

#[test]
fn retained_spaces_share_process_identity_but_have_distinct_wire_contexts() {
    let host = Host::new(WeakEntity::new_invalid(), "files");
    let a = host
        .clone()
        .for_space(ChatKey::Proteus("/sessions/a".into()));
    let b = host
        .clone()
        .for_space(ChatKey::Proteus("/sessions/b".into()));
    assert_eq!(a.identity(), b.identity());
    assert!(a.retains_process());
    assert_ne!(a.scope(), b.scope());
    assert_eq!(a.scope(), a.clone().with_extension("git").scope());
    assert_eq!(a.scope(), a.rehost(a.workspace.clone()).scope());
    let detached = a.rehost(WeakEntity::new_invalid());
    assert_ne!(a.scope(), detached.scope());
    assert_eq!(
        a.context_key(|| panic!("source document read another root")),
        detached.context_key(|| panic!("detached document read another root"))
    );
    assert_eq!(super::space_context("window".into(), None), "window");
    assert_ne!(
        super::space_context("window".into(), Some(&ChatKey::Proteus("same-id".into()))),
        super::space_context("window".into(), Some(&ChatKey::Codex("same-id".into())))
    );
}

#[test]
fn isolated_document_tree_discards_space_pin_without_widening_capabilities() {
    let host = Host::new(WeakEntity::new_invalid(), "files")
        .for_space(ChatKey::Proteus("/sessions/root".into()))
        .isolated(Lifetime::Preview);
    assert!(host.space.is_none());
    let retained = host
        .clone()
        .for_space(ChatKey::Proteus("/sessions/other".into()));
    assert_eq!(host.scope(), retained.scope());
    assert_eq!(
        retained.context_session(|| panic!("preview read workspace root")),
        None
    );
    assert!(!retained.retains_process());
}

#[test]
fn detached_document_inherits_only_its_own_fixed_root() {
    let root = ChatKey::Proteus("/sessions/fixed".into());
    let other = ChatKey::Proteus("/sessions/other".into());
    let mut host = Host::new(WeakEntity::new_invalid(), "files").for_space(root.clone());
    host.inherit(Some((WeakEntity::new_invalid(), root.clone())));
    assert!(host.inherited_root(&root).is_some());
    assert!(host.inherited_root(&other).is_none());
    assert_eq!(
        host.context_session(|| Some(other.clone())),
        Some("/sessions/fixed".into())
    );
    host.inherit(Some((WeakEntity::new_invalid(), other.clone())));
    assert!(host.inherited_root(&root).is_none());
    let mut dynamic = Host::new(WeakEntity::new_invalid(), "files");
    dynamic.inherit(Some((WeakEntity::new_invalid(), root.clone())));
    assert!(dynamic.inherited_root(&root).is_none());
    assert_eq!(dynamic.context_session(|| None), None);
}

#[test]
fn root_panels_and_command_badges_choose_independent_dynamic_sessions() {
    let root = ChatKey::Proteus("/sessions/root-a".into());
    let child = ChatKey::Proteus("/sessions/child-c".into());
    let panel = Host::new(WeakEntity::new_invalid(), "model");
    let badge = panel.clone().for_command_chat();
    assert_eq!(
        panel.context_session(|| panel.dynamic_key(
            || Some(root.clone()),
            || panic!("root panel read command chat"),
        )),
        Some("/sessions/root-a".into())
    );
    assert_eq!(
        badge.context_session(|| badge.dynamic_key(
            || panic!("composer badge read area root"),
            || Some(child.clone()),
        )),
        Some("/sessions/child-c".into())
    );
    assert_eq!(
        badge.context_session(|| badge.dynamic_key(
            || panic!("composer badge fell back to root"),
            || Some(root.clone()),
        )),
        Some("/sessions/root-a".into()),
        "a later command target is resolved again without rebinding the badge"
    );
    assert_eq!(
        badge.context_session(
            || badge.dynamic_key(|| panic!("empty command target fell back to root"), || None,)
        ),
        None
    );
    assert_eq!(
        badge.context_session(|| badge.dynamic_key(
            || panic!("external command target fell back to Core root"),
            || Some(ChatKey::Codex("child-codex".into())),
        )),
        None
    );
}

#[test]
fn command_badges_share_process_identity_but_have_their_own_dynamic_scope() {
    let panel = Host::new(WeakEntity::new_invalid(), "model");
    let badge = panel.clone().for_command_chat();
    assert_eq!(panel.identity(), badge.identity());
    assert!(badge.retains_process());
    assert_eq!(badge.scope(), format!("{}:command", panel.scope()));
    assert_eq!(badge.scope(), badge.with_extension("mode").scope());
    assert_eq!(badge.scope(), badge.rehost(badge.workspace.clone()).scope());
    assert!(badge.bound.is_none());
    assert!(badge.space.is_none());
    let fixed = badge
        .clone()
        .for_space(ChatKey::Proteus("/sessions/root".into()));
    assert!(!fixed.command_chat);
    assert_ne!(fixed.scope(), badge.scope());
    assert_eq!(
        fixed.context_session(|| panic!("fixed document read command target")),
        Some("/sessions/root".into())
    );
    let bound = badge.clone().for_chat(
        WeakEntity::new_invalid(),
        ChatKey::Proteus("/sessions/bound".into()),
    );
    assert!(!bound.command_chat);
    assert_eq!(
        bound.context_session(|| panic!("bound renderer read command target")),
        Some("/sessions/bound".into())
    );
    let dynamic = bound.for_command_chat();
    assert!(dynamic.bound.is_none());
    assert_eq!(dynamic.identity(), panel.identity());
    assert_eq!(dynamic.scope(), badge.scope());
}

#[test]
fn command_context_cannot_widen_local_external_or_isolated_capabilities() {
    let mut local = Host::new(WeakEntity::new_invalid(), "local-widget");
    local.runtime_scope.content_only = true;
    let command = local.for_command_chat();
    assert!(command.runtime_scope().content_only);
    assert_eq!(
        command.context_session(|| panic!("local command read Core session")),
        None
    );
    let external = Host::new(WeakEntity::new_invalid(), "model")
        .for_chat(WeakEntity::new_invalid(), ChatKey::Codex("external".into()))
        .for_command_chat();
    assert_eq!(
        external.context_session(|| panic!("command widened external host")),
        None
    );
    for lifetime in [Lifetime::Preview, Lifetime::Settings] {
        let isolated = Host::new(WeakEntity::new_invalid(), "model").isolated(lifetime);
        let command = isolated.clone().for_command_chat();
        assert_eq!(command.scope(), isolated.scope());
        assert_eq!(command.runtime_scope(), isolated.runtime_scope());
        assert!(!command.retains_process());
        assert_eq!(
            command.context_session(|| panic!("isolated command read production context")),
            None
        );
        command.runtime_scope().connection(
            || panic!("isolated command read server credentials"),
            || panic!("isolated command read production folder"),
        );
        assert!(!command.isolated(lifetime).command_chat);
    }
}
