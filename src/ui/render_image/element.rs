//! GPUI element-state leases survive reuse_paint together with its sprite tiles.
use std::sync::{Arc, Weak};

use gpui_kit::*;

use super::{Bitmap, SceneLease};

pub(in super::super) fn visible(bounds: Bounds<Pixels>, mask: Bounds<Pixels>) -> bool {
    let intersection = bounds.intersect(&mask);
    intersection.size.width > px(0.) && intersection.size.height > px(0.)
}

/// Access during paint only. GPUI replays this accessed state alongside a
/// cached scene, and drops it when the scene unmounts or is freshly painted.
pub(in super::super) fn with_lease<K: Clone + Eq + 'static>(
    id: &GlobalElementId,
    key: &K,
    window: &mut Window,
    cx: &mut App,
    resolve: impl FnOnce(Option<Arc<Bitmap>>, &mut Window, &mut App) -> Option<Arc<Bitmap>>,
    paint: impl FnOnce(Option<&Arc<Bitmap>>, &mut Window, &mut App),
) {
    window.with_element_state(id, |previous: Option<Option<SceneLease<K>>>, window| {
        let previous = previous.flatten().filter(|lease| lease.key == *key);
        let bitmap = resolve(
            previous.as_ref().map(|lease| lease.bitmap.clone()),
            window,
            cx,
        );
        // Create the successor lease before dropping the previous one. The
        // cache may already have evicted this bitmap while the scene kept it.
        let lease = bitmap.map(|bitmap| SceneLease::new(key.clone(), bitmap));
        drop(previous);
        paint(lease.as_ref().map(|lease| &lease.bitmap), window, cx);
        ((), lease)
    });
}

/// Adds scene lifetime to the original Img element, keeping its animation,
/// layout, clipping and image-source behavior intact.
pub(in super::super) fn retain<K: Clone + Eq + 'static>(
    id: impl Into<ElementId>,
    key: K,
    bitmap: Weak<Bitmap>,
    image: impl IntoElement,
) -> impl IntoElement {
    RetainedImage {
        id: id.into(),
        key,
        bitmap,
        image: image.into_any_element(),
    }
}

struct RetainedImage<K> {
    id: ElementId,
    key: K,
    bitmap: Weak<Bitmap>,
    image: AnyElement,
}

impl<K: Clone + Eq + 'static> IntoElement for RetainedImage<K> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<K: Clone + Eq + 'static> Element for RetainedImage<K> {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (self.image.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        self.image.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !visible(bounds, window.content_mask().bounds) {
            return;
        }
        let bitmap = &self.bitmap;
        let image = &mut self.image;
        with_lease(
            id.expect("RetainedImage has a stable id"),
            &self.key,
            window,
            cx,
            |previous, _, _| {
                // A completed same-key reload can replace the weak source.
                // Lease exactly what Img will paint, not an older same-key Arc.
                previous
                    .filter(|previous| Arc::as_ptr(previous) == bitmap.as_ptr())
                    .or_else(|| bitmap.upgrade())
            },
            |_, window, cx| image.paint(window, cx),
        );
    }
}
