//! Exercise the real parent layout, including a measured width-dependent
//! child. A second draw at the same progress cannot repair the first one.
use super::*;
use ::gpui::{TestAppContext, VisualTestContext};
use gpui_kit::base::ElementExt as _;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Observed {
    hits: Rc<RefCell<Vec<Hitbox>>>,
    paints: Rc<Cell<usize>>,
}

struct Content {
    lines: f32,
    observed: Observed,
}

impl IntoElement for Content {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for Content {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some("width-dependent-content".into())
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.flex_shrink = 0.;
        let lines = self.lines;
        (
            window.request_measured_layout(style, move |known, available, _, _| {
                let width = known.width.unwrap_or_else(|| match available.width {
                    AvailableSpace::Definite(width) => width,
                    _ => px(200.),
                });
                size(
                    width,
                    px((lines * 200. / width.max(px(1.)).to_f64() as f32).ceil() * 20.),
                )
            }),
            (),
        )
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        self.observed
            .hits
            .borrow_mut()
            .push(window.insert_hitbox(bounds, HitboxBehavior::Normal));
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
        self.observed.paints.set(self.observed.paints.get() + 1);
    }
}

fn draw(cx: &mut VisualTestContext, width: f32, element: AnyElement) -> Pixels {
    let bottom = Rc::new(Cell::new(None));
    cx.draw(point(px(0.), px(0.)), size(px(width), px(2000.)), |_, _| {
        div()
            .flex()
            .flex_col()
            .w(px(width))
            .child(element)
            .child(div().h(px(1.)).on_prepaint({
                let bottom = bottom.clone();
                move |bounds, _, _| bottom.set(Some(bounds.top()))
            }))
            .into_any_element()
    });
    bottom
        .get()
        .expect("the following sibling must be laid out")
}

fn content(lines: f32, observed: &Observed) -> AnyElement {
    Content {
        lines,
        observed: observed.clone(),
    }
    .into_any_element()
}

#[::core::prelude::v1::test]
fn current_content_and_width_change_the_same_fractional_layout_frame() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let observed = Observed::default();
    for (width, lines, height) in [
        (200., 5., 50.),
        (200., 8., 80.),
        (100., 8., 160.),
        (400., 8., 40.),
    ] {
        observed.hits.borrow_mut().clear();
        let bottom = draw(
            cx,
            width,
            Reveal::new("changing", Some(0.5), content(lines, &observed)).into_any_element(),
        );
        assert_eq!(
            bottom,
            px(height),
            "width={width}, lines={lines}: same-frame natural height"
        );
        let hits = observed.hits.borrow();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].bounds.size.width, px(width));
        assert_eq!(hits[0].bounds.top(), px(0.));
        assert_eq!(hits[0].content_mask.bounds.size.height, px(height));
    }
}

#[::core::prelude::v1::test]
fn nested_reveals_and_their_following_sibling_use_one_layout_generation() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let observed = Observed::default();
    for (width, inner, expected) in [
        (200., 0., 10.),
        (200., 0.4, 30.),
        (100., 0.4, 50.),
        (100., 0.8, 90.),
        (100., 0.2, 30.),
    ] {
        let inner = Reveal::new("inner", Some(inner), content(5., &observed));
        let child = div()
            .flex()
            .flex_col()
            .w_full()
            .child(div().h(px(20.)).flex_shrink_0())
            .child(inner);
        let outer = Reveal::new("outer", Some(0.5), child.into_any_element());
        assert_eq!(draw(cx, width, outer.into_any_element()), px(expected));
    }
}

#[::core::prelude::v1::test]
fn preview_reversal_and_settled_endpoints_have_continuous_native_heights() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let observed = Observed::default();
    for (progress, expected) in [
        (Some(0.), 20.),
        (Some(0.5), 60.),
        (Some(0.8), 84.),
        (Some(0.5), 60.),
        (Some(0.), 20.),
    ] {
        let reveal = Reveal::preview(
            "previewed",
            progress,
            true,
            div().w_full().h(px(20.)).into_any_element(),
            content(5., &observed),
        );
        assert_eq!(draw(cx, 200., reveal.into_any_element()), px(expected));
    }
    let at_end = Reveal::preview(
        "previewed",
        Some(1.),
        true,
        div().w_full().h(px(20.)).into_any_element(),
        content(5., &observed),
    );
    assert_eq!(draw(cx, 200., at_end.into_any_element()), px(100.));
    let settled = Reveal::preview(
        "previewed",
        None,
        true,
        div().w_full().h(px(20.)).into_any_element(),
        content(5., &observed),
    );
    assert_eq!(draw(cx, 200., settled.into_any_element()), px(100.));
    let closed = Reveal::preview(
        "previewed",
        None,
        false,
        div().w_full().h(px(20.)).into_any_element(),
        content(5., &observed),
    );
    assert_eq!(draw(cx, 200., closed.into_any_element()), px(20.));
}

#[::core::prelude::v1::test]
fn an_empty_reveal_does_not_prepaint_hidden_content_or_create_its_hitboxes() {
    let mut app = TestAppContext::single();
    let cx = app.add_empty_window();
    let observed = Observed::default();
    assert_eq!(
        draw(
            cx,
            200.,
            Reveal::new("hidden", Some(0.), content(5., &observed)).into_any_element()
        ),
        px(0.)
    );
    assert!(observed.hits.borrow().is_empty());
    assert_eq!(observed.paints.get(), 0);
    assert_eq!(
        draw(
            cx,
            200.,
            Reveal::new("hidden", Some(0.5), content(5., &observed)).into_any_element()
        ),
        px(50.)
    );
    assert_eq!(observed.hits.borrow().len(), 1);
    assert_eq!(observed.paints.get(), 1);
}
