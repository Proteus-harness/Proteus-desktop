//! Sent-message previews have fixed UI geometry, bounded native bitmaps and
//! application-wide fetch/decode admission. Encoded originals never enter GPUI.
mod cache;
mod decode;
mod schedule;

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::domain::ImageRef;

use super::Chat;
use crate::ui::prefs;
use crate::ui::render_image::load as render_bitmap;
use crate::ui::tip::Tip as _;

const THUMB: f32 = 96.;
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Key {
    connection: u64,
    dir: PathBuf,
    id: String,
    edge: u32,
}

enum Stored {
    Loading,
    Waiting,
    Ready(Weak<cache::Bitmap>),
    Failed(SharedString),
}

struct Slot {
    key: Key,
    ticket: u64,
    owner: Arc<()>,
    state: Stored,
}

impl Slot {
    fn matches(&self, key: &Key, ticket: u64) -> bool {
        self.key == *key && self.ticket == ticket && matches!(self.state, Stored::Loading)
    }
}

#[derive(Default)]
struct Shared {
    cache: cache::Cache,
    queue: schedule::Queue,
}

thread_local! {
    static SHARED: RefCell<Shared> = RefCell::default();
}

#[derive(Default)]
pub(super) struct SentImages {
    by_id: HashMap<String, Slot>,
}

impl SentImages {
    pub(super) fn retain(&mut self, entries: &[crate::transcript::Entry]) {
        let ids: std::collections::HashSet<_> = entries
            .iter()
            .flat_map(|row| row.images.iter().map(|image| image.id.as_str()))
            .collect();
        self.by_id.retain(|id, _| ids.contains(id.as_str()));
        SHARED.with(|shared| shared.borrow_mut().cache.prune());
    }

    pub(super) fn missing(
        &mut self,
        images: &[ImageRef],
        connection: u64,
        dir: &std::path::Path,
        edge: u32,
    ) -> Vec<String> {
        let mut missing = Vec::new();
        for image in images {
            let key = Key {
                connection,
                dir: dir.into(),
                id: image.id.clone(),
                edge,
            };
            if self
                .by_id
                .get(&image.id)
                .is_some_and(|slot| slot.key != key)
            {
                self.by_id.remove(&image.id);
            }
            if let Some(slot) = self.by_id.get_mut(&image.id) {
                if let Some(image) =
                    SHARED.with(|shared| shared.borrow_mut().cache.get(&key, &slot.owner))
                {
                    slot.state = Stored::Ready(Arc::downgrade(&image));
                    continue;
                }
                if let Stored::Ready(bitmap) = &slot.state
                    && bitmap
                        .upgrade()
                        .is_some_and(|bitmap| bitmap.has_scene_lease())
                {
                    // Replayed GPUI scenes keep their bitmap safe after LRU
                    // eviction; reloading would also reset GIF frame timing.
                    continue;
                }
                if !matches!(&slot.state, Stored::Ready(_) | Stored::Waiting) {
                    continue;
                }
                // A raw frame/deferred cleanup Arc has no scene lease and
                // cannot make an evicted bitmap safe to upload again.
                slot.state = Stored::Loading;
            }
            missing.push(image.id.clone());
        }
        SHARED.with(|shared| shared.borrow_mut().cache.prune());
        missing
    }
}

impl Drop for SentImages {
    fn drop(&mut self) {
        self.by_id.clear();
        let _ = SHARED.try_with(|shared| {
            if let Ok(mut shared) = shared.try_borrow_mut() {
                shared.cache.prune();
            }
        });
    }
}

impl Chat {
    pub(super) fn fetch_images(&mut self, ids: Vec<String>, edge: u32, cx: &mut Context<Self>) {
        let Some(api) = &self.api else { return };
        for id in ids {
            let key = Key {
                connection: api.connection_id(),
                dir: self.session.dir.clone(),
                id: id.clone(),
                edge,
            };
            let ticket = NEXT_TICKET.fetch_add(1, Ordering::Relaxed);
            let owner = Arc::new(());
            let request = schedule::Request {
                chat: cx.entity().downgrade(),
                owner: Arc::downgrade(&owner),
                key: key.clone(),
                ticket,
            };
            let accepted = SHARED.with(|shared| shared.borrow_mut().queue.push(request));
            self.sent_images.by_id.insert(
                id,
                Slot {
                    key,
                    ticket,
                    owner,
                    state: if accepted {
                        Stored::Loading
                    } else {
                        Stored::Waiting
                    },
                },
            );
        }
        // Defer: the current Chat is borrowed while a row is rendering.
        cx.defer(schedule::drain);
    }
}

pub(super) fn thumbnail_edge(window: &Window, cx: &App) -> u32 {
    decode::edge(window.scale_factor() * prefs::scale(cx))
}

pub(super) fn render_sent_images(
    images: &[ImageRef],
    store: &SentImages,
    cx: &App,
) -> Option<AnyElement> {
    if images.is_empty() {
        return None;
    }
    let theme = cx.theme();
    let thumbs = images.iter().map(|image| {
        let frame = div()
            .id(ElementId::Name(image.id.clone().into()))
            .size(prefs::scaled(THUMB))
            .flex_none()
            .rounded(theme.radius)
            .bg(theme.muted);
        match store
            .by_id
            .get(&image.id)
            .map(|slot| (&slot.key, &slot.state))
        {
            Some((key, Stored::Ready(bitmap))) if bitmap.strong_count() > 0 => frame
                .tip_text(image.name.clone())
                .child({
                    let source = bitmap.clone();
                    let image =
                        img(move |_: &mut Window, _: &mut App| render_bitmap(&source).map(Ok))
                            // GPUI keeps frame timing only for stateful images.
                            .id("bitmap")
                            .size_full()
                            .rounded(theme.radius)
                            .object_fit(ObjectFit::Cover);
                    crate::ui::render_image::retain(
                        "bitmap-lease",
                        key.clone(),
                        bitmap.clone(),
                        image,
                    )
                })
                .into_any_element(),
            Some((_, Stored::Failed(reason))) => frame
                .tip_text(format!("{}: {reason}", image.name))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    Icon::new(Lucide::ImageOff)
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .into_any_element(),
            _ => frame.into_any_element(),
        }
    });
    Some(
        h_flex()
            .flex_wrap()
            .justify_end()
            .gap_2()
            .children(thumbs)
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests;
