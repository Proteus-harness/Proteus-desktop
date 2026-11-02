use super::*;

fn key(id: usize) -> Key {
    Key {
        connection: 1,
        dir: "session".into(),
        id: id.to_string(),
        edge: 96,
    }
}
fn preview(pixels: u64) -> Preview {
    Preview {
        image: Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )])),
        pixels,
    }
}

#[::core::prelude::v1::test]
fn independent_owners_share_one_lru_and_last_owner_releases_cpu_image() {
    let first = Arc::new(());
    let second = Arc::new(());
    let mut cache = Cache::default();
    let image = cache.insert(key(0), preview(1), &first, None);
    let weak = Arc::downgrade(&image);
    assert!(Arc::ptr_eq(&image, &cache.get(&key(0), &second).unwrap()));
    drop(image);
    drop(first);
    cache.prune();
    assert!(weak.upgrade().is_some());
    drop(second);
    cache.prune();
    assert!(weak.upgrade().is_none());
    assert_eq!(cache.pixels, 0);
}

#[::core::prelude::v1::test]
fn byte_budget_evicts_old_bitmap_even_when_local_owner_is_alive() {
    let owner = Arc::new(());
    let mut cache = Cache::default();
    let old = Arc::downgrade(&cache.insert(key(0), preview(MAX_PIXELS / 2), &owner, None));
    cache.insert(key(1), preview(MAX_PIXELS / 2), &owner, None);
    cache.get(&key(0), &owner); // 1 becomes least recently used.
    let second = Arc::downgrade(&cache.get(&key(1), &owner).unwrap());
    cache.get(&key(0), &owner);
    cache.insert(key(2), preview(MAX_PIXELS / 2), &owner, None);
    assert!(old.upgrade().is_some());
    assert!(second.upgrade().is_none());
    assert!(cache.pixels <= MAX_PIXELS);
}

#[::core::prelude::v1::test]
fn tiny_images_are_limited_by_entry_count_and_key_includes_connection_and_scale() {
    let owner = Arc::new(());
    let mut cache = Cache::default();
    for index in 0..200 {
        cache.insert(key(index), preview(1), &owner, None);
    }
    assert_eq!(cache.entries.len(), MAX_ENTRIES);
    let mut wrong = key(199);
    wrong.connection = 2;
    assert!(cache.get(&wrong, &owner).is_none());
    wrong = key(199);
    wrong.edge = 192;
    assert!(cache.get(&wrong, &owner).is_none());
}
