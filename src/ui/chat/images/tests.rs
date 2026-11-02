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
