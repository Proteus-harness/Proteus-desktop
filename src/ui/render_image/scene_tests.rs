//! Real GPUI cached-scene replay, rather than a model of bitmap Arc counts.
use std::cell::Cell;
use std::rc::Rc;

use ::gpui::TestAppContext;
use gpui_kit::*;

use super::*;

struct Picture {
    key: u64,
    bitmap: Weak<Bitmap>,
    paints: Rc<Cell<usize>>,
}

impl Render for Picture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let source = self.bitmap.clone();
        let paints = self.paints.clone();
        retain(
            "picture-lease",
            self.key,
            self.bitmap.clone(),
            div()
                .size_full()
                .child(
                    img(move |_: &mut Window, _: &mut App| load(&source).map(Ok))
                        .id("bitmap")
                        .size_full(),
                )
                .child(
                    canvas(
                        |_, _, _| (),
                        move |_, _, _, _| {
                            paints.set(paints.get() + 1);
                        },
                    )
                    .absolute()
                    .size_full(),
                ),
        )
    }
}

struct Root {
    picture: Entity<Picture>,
    mounted: bool,
}

impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let child = self.mounted.then(|| {
            self.picture
                .clone()
                .cached(StyleRefinement::default().size(px(100.)))
        });
        div().size(px(200.)).children(child)
    }
}

fn draw(cx: &mut TestAppContext, window: WindowHandle<Root>) {
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
        .unwrap();
}

#[::core::prelude::v1::test]
fn a_replayed_cached_scene_owns_its_bitmap_until_unmounted() {
    let mut cx = TestAppContext::single();
    let raw = Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::new(2, 2),
    )]));
    let cached = cx.update(|cx| Bitmap::new(raw.clone(), Some(cx)));
    let weak = Arc::downgrade(&cached);
    let paints = Rc::new(Cell::new(0));
    let window = cx.add_window({
        let bitmap = weak.clone();
        let paints = paints.clone();
        move |_, cx| Root {
            picture: cx.new(|_| Picture {
                key: 1,
                bitmap,
                paints,
            }),
            mounted: true,
        }
    });
    draw(&mut cx, window);
    assert!(
        cached.has_scene_lease(),
        "actual paint must acquire a GPUI scene lease"
    );
    drop(cached); // Simulate LRU eviction: the view itself owns only Weak.
    let first_paints = paints.get();
    assert!(first_paints > 0);
    for _ in 0..4 {
        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
        draw(&mut cx, window);
        cx.run_until_parked();
        assert!(
            weak.upgrade().is_some(),
            "replayed tiles still need their bitmap"
        );
    }
    assert_eq!(
        paints.get(),
        first_paints,
        "cached replay must not repaint or schedule a refresh loop"
    );

    window
        .update(&mut cx, |root, _, cx| {
            root.mounted = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    draw(&mut cx, window);
    cx.run_until_parked();
    assert!(
        weak.upgrade().is_none(),
        "unmount releases the last scene lease"
    );
    assert!(
        load(&weak).is_none(),
        "retained raw pixels cannot resurrect an evicted token"
    );
    assert_eq!(raw.size(0), size(DevicePixels(2), DevicePixels(2)));
}

fn assert_source_replacement(next_key: u64) {
    let mut cx = TestAppContext::single();
    let old_raw = Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::new(2, 2),
    )]));
    let old = cx.update(|cx| Bitmap::new(old_raw.clone(), Some(cx)));
    let old_source = Arc::downgrade(&old);
    let window = cx.add_window({
        let bitmap = old_source.clone();
        move |_, cx| Root {
            picture: cx.new(|_| Picture {
                key: 1,
                bitmap,
                paints: Rc::new(Cell::new(0)),
            }),
            mounted: true,
        }
    });
    draw(&mut cx, window);
    drop(old);
    assert!(old_source.upgrade().unwrap().has_scene_lease());

    let new = cx.update(|cx| {
        Bitmap::new(
            Arc::new(RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::new(3, 3),
            )])),
            Some(cx),
        )
    });
    let new_source = Arc::downgrade(&new);
    window
        .update(&mut cx, |root, _, cx| {
            root.picture.update(cx, |picture, cx| {
                picture.key = next_key;
                picture.bitmap = new_source.clone();
                cx.notify();
            })
        })
        .unwrap();
    draw(&mut cx, window);
    drop(new);
    draw(&mut cx, window);
    cx.run_until_parked();
    assert!(
        old_source.upgrade().is_none(),
        "replaced source releases the old painted-scene lease"
    );
    assert!(
        new_source.upgrade().unwrap().has_scene_lease(),
        "the lease must follow the bitmap Img actually painted"
    );

    window
        .update(&mut cx, |root, _, cx| {
            root.mounted = false;
            cx.notify();
        })
        .unwrap();
    draw(&mut cx, window);
    draw(&mut cx, window);
    cx.run_until_parked();
    assert!(
        new_source.upgrade().is_none(),
        "replacement releases its lease after unmount"
    );
    assert_eq!(old_raw.size(0), size(DevicePixels(2), DevicePixels(2)));
}

#[::core::prelude::v1::test]
fn replacing_a_painted_scene_key_releases_the_previous_bitmap() {
    assert_source_replacement(2);
}

#[::core::prelude::v1::test]
fn a_same_key_reload_leases_the_new_img_source_instead_of_the_old_bitmap() {
    assert_source_replacement(1);
}
