//! Application-wide thumbnail ownership. Views retain Weak bitmaps only.
use std::collections::VecDeque;
use std::sync::{Arc, Weak};

use gpui_kit::{App, AsyncApp, RenderImage};

use super::Key;
use super::decode::Preview;

const MAX_ENTRIES: usize = 64;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;

pub(super) struct Bitmap {
    image: Option<Arc<RenderImage>>,
    cleanup: Option<AsyncApp>,
}

impl Bitmap {
    pub fn image(&self) -> Arc<RenderImage> {
        self.image.as_ref().unwrap().clone()
    }
}

impl Drop for Bitmap {
    fn drop(&mut self) {
        let (Some(image), Some(cx)) = (self.image.take(), self.cleanup.take()) else {
            return;
        };
        // Drop may run inside a Window/Entity borrow or without an App (last
        // view owner closes). The weak AsyncApp schedules atlas removal on the
        // next UI turn, then defer guarantees no Window borrow is active.
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

struct Entry {
    key: Key,
    bitmap: Arc<Bitmap>,
    pixels: u64,
    owners: Vec<Weak<()>>,
}

#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<Entry>,
    pixels: u64,
}

impl Cache {
    pub fn prune(&mut self) {
        for entry in &mut self.entries {
            entry.owners.retain(|owner| owner.strong_count() > 0);
        }
        self.entries.retain(|entry| !entry.owners.is_empty());
        self.pixels = self.entries.iter().map(|entry| entry.pixels).sum();
    }

    pub fn get(&mut self, key: &Key, owner: &Arc<()>) -> Option<Arc<Bitmap>> {
        self.prune();
        let position = self.entries.iter().position(|entry| entry.key == *key)?;
        let mut entry = self.entries.remove(position)?;
        if !entry
            .owners
            .iter()
            .any(|old| old.as_ptr() == Arc::as_ptr(owner))
        {
            entry.owners.push(Arc::downgrade(owner));
        }
        let image = entry.bitmap.clone();
        self.entries.push_back(entry);
        Some(image)
    }

    pub fn insert(
        &mut self,
        key: Key,
        preview: Preview,
        owner: &Arc<()>,
        cx: Option<&App>,
    ) -> Arc<Bitmap> {
        // Concurrent readers can finish the same thumbnail. Reuse the one
        // already published; the duplicate has never reached an atlas.
        if let Some(image) = self.get(&key, owner) {
            return image;
        }
        assert!(preview.pixels <= MAX_PIXELS);
        while self.entries.len() >= MAX_ENTRIES || self.pixels + preview.pixels > MAX_PIXELS {
            let Some(old) = self.entries.pop_front() else {
                break;
            };
            self.pixels -= old.pixels;
        }
        let bitmap = Arc::new(Bitmap {
            image: Some(preview.image),
            cleanup: cx.map(App::to_async),
        });
        self.pixels += preview.pixels;
        self.entries.push_back(Entry {
            key,
            bitmap: bitmap.clone(),
            pixels: preview.pixels,
            owners: vec![Arc::downgrade(owner)],
        });
        bitmap
    }
}

#[cfg(test)]
mod tests;
