// Адаптировано из Zed: crates/markdown/src/mermaid.rs (drop_images).
//! Shared lifetime for prepared native bitmaps in bounded UI caches.
//!
//! The LRU and GPUI's painted-scene state own Arc<Bitmap> leases. Retained
//! element closures capture only Weak references or immutable SVG metadata.
//! Raw RenderImage Arcs from an old frame cannot revive an evicted bitmap
//! after its atlas removal has been scheduled.
mod element;

pub(super) use element::{retain, visible, with_lease};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

use gpui_kit::{App, AsyncApp, RenderImage};

pub(super) struct Bitmap {
    image: Option<Arc<RenderImage>>,
    cleanup: Option<AsyncApp>,
    scene_leases: AtomicUsize,
}

impl Bitmap {
    pub fn new(image: Arc<RenderImage>, cx: Option<&App>) -> Arc<Self> {
        Arc::new(Self {
            image: Some(image),
            cleanup: cx.map(App::to_async),
            scene_leases: AtomicUsize::new(0),
        })
    }

    pub fn image(&self) -> Arc<RenderImage> {
        self.image.as_ref().unwrap().clone()
    }

    pub fn has_scene_lease(&self) -> bool {
        self.scene_leases.load(Ordering::Relaxed) > 0
    }
}

/// Stored in GPUI element state, whose lifetime includes cached scene replay.
/// A raw paint-frame Arc is deliberately insufficient to acquire this lease.
struct SceneLease<K> {
    key: K,
    bitmap: Arc<Bitmap>,
}

impl<K> SceneLease<K> {
    fn new(key: K, bitmap: Arc<Bitmap>) -> Self {
        bitmap.scene_leases.fetch_add(1, Ordering::Relaxed);
        Self { key, bitmap }
    }
}

impl<K> Drop for SceneLease<K> {
    fn drop(&mut self) {
        self.bitmap.scene_leases.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Resolve only the cache lifetime token, never a Weak<RenderImage>.
pub(super) fn load(bitmap: &Weak<Bitmap>) -> Option<Arc<RenderImage>> {
    bitmap.upgrade().map(|bitmap| bitmap.image())
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        let (Some(image), Some(cx)) = (self.image.take(), self.cleanup.take()) else {
            return;
        };
        // Eviction can run inside a Window/Entity borrow, and last-owner Drop
        // need not have an App. AsyncApp is weak; defer performs atlas removal
        // outside any Window borrow, across every window that painted it.
        cx.foreground_executor()
            .spawn({
                let cx = cx.clone();
                async move {
                    cx.update(|cx| cx.defer(move |cx| cx.drop_image(image, None)));
                }
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn raw_paint_frame_cannot_keep_an_evicted_token_loadable() {
        let image = Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )]));
        let bitmap = Bitmap::new(image.clone(), None);
        let loader = Arc::downgrade(&bitmap);
        assert!(Arc::ptr_eq(&image, &load(&loader).unwrap()));
        drop(bitmap);
        assert!(Arc::strong_count(&image) > 0);
        assert!(load(&loader).is_none());
    }
}

#[cfg(test)]
mod scene_tests;
