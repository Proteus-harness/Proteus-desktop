use super::*;

#[::core::prelude::v1::test]
fn stale_completion_cannot_replace_a_rerequest_with_same_image_id() {
    let key = Key {
        connection: 1,
        dir: "session".into(),
        id: "image".into(),
        edge: 96,
    };
    let slot = Slot {
        key: key.clone(),
        ticket: 10,
        owner: Arc::new(()),
        rearm_on_failure: false,
        state: Stored::Loading,
    };
    assert!(!slot.matches(&key, 9));
    assert!(slot.matches(&key, 10));
    let mut other = key.clone();
    other.connection = 2;
    assert!(!slot.matches(&other, 10));
    other = key;
    other.edge = 192;
    assert!(!slot.matches(&other, 10));
}

#[::core::prelude::v1::test]
fn cache_eviction_invalidates_ready_before_render_even_when_old_arc_survives() {
    let key = Key {
        connection: 1,
        dir: "session".into(),
        id: "evicted".into(),
        edge: 96,
    };
    let owner = Arc::new(());
    let make_preview = || decode::Preview {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        pixels: 16 * 1024 * 1024,
    };
    let outside = SHARED.with(|shared| {
        shared
            .borrow_mut()
            .cache
            .insert(key.clone(), make_preview(), &owner, None)
    });
    let mut images = SentImages::default();
    images.by_id.insert(
        key.id.clone(),
        Slot {
            key: key.clone(),
            ticket: 1,
            owner,
            rearm_on_failure: false,
            state: Stored::Ready(Arc::downgrade(&outside)),
        },
    );
    let replacement_owner = Arc::new(());
    let mut replacement = key.clone();
    replacement.id = "replacement".into();
    SHARED.with(|shared| {
        shared
            .borrow_mut()
            .cache
            .insert(replacement, make_preview(), &replacement_owner, None);
    });
    assert!(Arc::downgrade(&outside).upgrade().is_some());
    let reference = ImageRef {
        id: key.id.clone(),
        name: "fixture.png".into(),
        mime_type: "image/png".into(),
        path: "fixture.png".into(),
    };
    assert_eq!(
        images.missing(&[reference], key.connection, &key.dir, key.edge),
        vec![key.id.clone()]
    );
    assert!(matches!(images.by_id[&key.id].state, Stored::Loading));
    // The stale Arc may still be used by an old paint frame, but the new row
    // must be a placeholder until an LRU-owned replacement is published.
    assert!(!matches!(images.by_id[&key.id].state, Stored::Ready(_)));
}

#[::core::prelude::v1::test]
fn retained_loader_cannot_reupload_evicted_bitmap_even_while_cleanup_holds_raw_image() {
    let key = Key {
        connection: 7,
        dir: "session".into(),
        id: "old-loader".into(),
        edge: 96,
    };
    let owner = Arc::new(());
    let make_preview = || decode::Preview {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        pixels: 16 * 1024 * 1024,
    };
    let bitmap = SHARED.with(|shared| {
        shared
            .borrow_mut()
            .cache
            .insert(key.clone(), make_preview(), &owner, None)
    });
    let loader = Arc::downgrade(&bitmap);
    let raw_image = render_bitmap(&loader).unwrap();
    let raw_weak = Arc::downgrade(&raw_image);
    drop(bitmap);
    let mut replacement = key;
    replacement.id = "new-loader".into();
    SHARED.with(|shared| {
        shared
            .borrow_mut()
            .cache
            .insert(replacement, make_preview(), &owner, None);
    });
    assert!(
        raw_weak.upgrade().is_some(),
        "an old frame/cleanup still holds raw pixels"
    );
    assert!(loader.upgrade().is_none(), "cache token is already dead");
    assert!(
        render_bitmap(&loader).is_none(),
        "retained Img loader cannot reupload"
    );
}

fn retry_slot(id: &str, state: Stored) -> Slot {
    Slot {
        key: Key {
            connection: 41,
            dir: "retry-fixture".into(),
            id: id.into(),
            edge: 96,
        },
        ticket: 101,
        owner: Arc::new(()),
        rearm_on_failure: false,
        state,
    }
}

fn reference(id: &str) -> ImageRef {
    ImageRef {
        id: id.into(),
        name: "fixture.png".into(),
        mime_type: "image/png".into(),
        path: "fixture.png".into(),
    }
}

#[::core::prelude::v1::test]
fn failure_before_reconnect_is_rearmed_only_for_the_next_demanded_render() {
    let id = "rearm-existing-failure";
    let mut images = SentImages::default();
    let mut slot = retry_slot(id, Stored::Loading);
    let key = slot.key.clone();
    slot.complete(Err("temporary fixture error".into()));
    images.by_id.insert(id.into(), slot);
    for _ in 0..3 {
        assert!(
            images
                .missing(&[reference(id)], key.connection, &key.dir, key.edge)
                .is_empty()
        );
    }
    images.rearm_after_reconnect();
    assert!(matches!(images.by_id[id].state, Stored::Waiting));
    assert_eq!(
        images.missing(&[reference(id)], key.connection, &key.dir, key.edge),
        [id]
    );
    // This unit test stops at eligibility. fetch_images owns real admission
    // and replaces the slot with a fresh ticket and rearm_on_failure=false.
}

#[::core::prelude::v1::test]
fn failure_after_reconnect_consumes_one_coalesced_rearm_not_a_retry_loop() {
    let id = "rearm-late-failure";
    let mut images = SentImages::default();
    images
        .by_id
        .insert(id.into(), retry_slot(id, Stored::Loading));
    images.rearm_after_reconnect();
    images.rearm_after_reconnect();
    let slot = images.by_id.get_mut(id).unwrap();
    assert!(slot.rearm_on_failure);
    slot.complete(Err("old scheduled attempt".into()));
    assert!(matches!(slot.state, Stored::Waiting));
    assert!(!slot.rearm_on_failure);
    // A later admitted attempt starts unflagged, as fetch_images does.
    let mut replacement = retry_slot(id, Stored::Loading);
    replacement.ticket += 1;
    replacement.complete(Err("persistent failure".into()));
    assert!(matches!(replacement.state, Stored::Failed(_)));
    assert!(!replacement.rearm_on_failure);
}

#[::core::prelude::v1::test]
fn a_successful_pre_reconnect_attempt_is_kept_and_clears_its_retry_credit() {
    let id = "rearm-success";
    let mut images = SentImages::default();
    images
        .by_id
        .insert(id.into(), retry_slot(id, Stored::Loading));
    images.rearm_after_reconnect();
    let slot = images.by_id.get_mut(id).unwrap();
    slot.complete(Ok(Weak::new()));
    assert!(matches!(slot.state, Stored::Ready(_)));
    assert!(!slot.rearm_on_failure);
}

#[::core::prelude::v1::test]
fn rearming_preserves_ready_waiting_and_current_attempt_identity() {
    let mut images = SentImages::default();
    for (id, state) in [
        ("rearm-ready", Stored::Ready(Weak::new())),
        ("rearm-waiting", Stored::Waiting),
        ("rearm-loading", Stored::Loading),
    ] {
        images.by_id.insert(id.into(), retry_slot(id, state));
    }
    let key = images.by_id["rearm-loading"].key.clone();
    let owner = images.by_id["rearm-loading"].owner.clone();
    images.rearm_after_reconnect();
    assert!(matches!(
        images.by_id["rearm-ready"].state,
        Stored::Ready(_)
    ));
    assert!(matches!(
        images.by_id["rearm-waiting"].state,
        Stored::Waiting
    ));
    assert!(!images.by_id["rearm-ready"].rearm_on_failure);
    assert!(!images.by_id["rearm-waiting"].rearm_on_failure);
    let slot = &images.by_id["rearm-loading"];
    assert!(slot.matches(&key, 101));
    assert!(!slot.matches(&key, 100));
    let mut other = key.clone();
    other.connection += 1;
    assert!(!slot.matches(&other, 101));
    assert!(Arc::ptr_eq(&slot.owner, &owner));
    assert_eq!(images.by_id.len(), 3);
}

#[::core::prelude::v1::test]
fn only_connected_after_a_link_error_rearms_existing_failures_and_attempts() {
    use crate::connection::PumpItem;
    use crate::ui::chat::{ComposerExtras, Session};
    use std::rc::Rc;

    let mut cx = ::gpui::TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let window = cx.add_empty_window();
    let chat = window.update(|window, cx| {
        cx.new(|cx| {
            let extras: ComposerExtras = Rc::new(|_, _, _| None);
            Chat::base(
                None, // no API and no config/image HTTP
                WeakEntity::new_invalid(),
                Session::new("retry-fixture".into()),
                extras,
                window,
                cx,
            )
        })
    });
    chat.update(&mut cx, |chat, cx| {
        chat.sent_images.by_id.insert(
            "failed".into(),
            retry_slot("failed", Stored::Failed("fixture".into())),
        );
        chat.sent_images
            .by_id
            .insert("loading".into(), retry_slot("loading", Stored::Loading));
        assert!(chat.apply(PumpItem::Connected, cx));
        assert!(matches!(
            chat.sent_images.by_id["failed"].state,
            Stored::Failed(_)
        ));
        assert!(!chat.sent_images.by_id["loading"].rearm_on_failure);
        chat.apply(PumpItem::Disconnected("fixture disconnect".into()), cx);
        assert!(matches!(
            chat.sent_images.by_id["failed"].state,
            Stored::Failed(_)
        ));
        assert!(chat.apply(PumpItem::Connected, cx));
        assert!(matches!(
            chat.sent_images.by_id["failed"].state,
            Stored::Waiting
        ));
        assert!(chat.sent_images.by_id["loading"].rearm_on_failure);
        chat.sent_images
            .by_id
            .get_mut("loading")
            .unwrap()
            .complete(Ok(Weak::new()));
        chat.sent_images.by_id.insert(
            "later".into(),
            retry_slot("later", Stored::Failed("later error".into())),
        );
        chat.apply(PumpItem::Connected, cx); // duplicate, no new link error
        assert!(matches!(
            chat.sent_images.by_id["later"].state,
            Stored::Failed(_)
        ));
        assert!(!chat.sent_images.by_id["loading"].rearm_on_failure);
    });
}
