use super::*;
use std::cell::RefCell;

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, Modifiers, MouseButton,
    ParentElement as _, Render, ScrollDelta, Styled as _, TestAppContext, VisualTestContext, div,
};

#[derive(Clone)]
struct Handle {
    viewport: Rc<Cell<Bounds<Pixels>>>,
    offset: Rc<Cell<Point<Pixels>>>,
    content: Size<Pixels>,
}

impl ScrollbarHandle for Handle {
    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.viewport.get()
    }

    fn offset(&self) -> Point<Pixels> {
        self.offset.get()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.offset.set(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        self.content
    }
}

#[derive(Clone)]
struct Observation {
    state: ScrollbarState,
    geometry: ThumbGeometry,
    thumb: Bounds<Pixels>,
    thumb_bg: Background,
}

type Observed = Rc<RefCell<Option<Observation>>>;

struct ObservedScrollbar(Scrollbar, Observed);

impl IntoElement for ObservedScrollbar {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for ObservedScrollbar {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.0)
    }

    fn source_location(&self) -> Option<&'static Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        self.0.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let prepaint = self.0.prepaint(id, inspector_id, bounds, state, window, cx);
        if let Some(axis) = prepaint.states.first() {
            *self.1.borrow_mut() = Some(Observation {
                state: prepaint.scrollbar_state.clone(),
                geometry: axis.geometry,
                thumb: axis.thumb_bounds,
                thumb_bg: axis.thumb_bg,
            });
        }
        prepaint
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.0
            .paint(id, inspector_id, bounds, state, prepaint, window, cx);
    }
}

struct Fixture {
    handle: Handle,
    observed: Observed,
    track_size: Size<Pixels>,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .w(self.track_size.width)
            .h(self.track_size.height)
            .on_scroll(cx.listener(|this, event: &ScrollEvent, window, cx| {
                let delta = event.delta.pixel_delta(window.line_height());
                this.handle.set_offset(this.handle.offset() + delta);
                cx.notify();
            }))
            .child(ObservedScrollbar(
                Scrollbar::vertical(&self.handle)
                    .id("observed-scrollbar")
                    .mode(ScrollbarMode::Always)
                    .viewport_from_layout()
                    .scroll_viewport_from_handle(),
                self.observed.clone(),
            ))
    }
}

struct CachedRoot(Entity<Fixture>);

impl Render for CachedRoot {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0
            .clone()
            .cached(gpui::StyleRefinement::default().size_full())
    }
}

fn fixture(
    cx: &mut TestAppContext,
    track_size: Size<Pixels>,
    viewport: Size<Pixels>,
    content: Size<Pixels>,
) -> (&mut VisualTestContext, Handle, Observed) {
    let handle = Handle {
        viewport: Rc::new(Cell::new(Bounds::new(Point::default(), viewport))),
        offset: Rc::new(Cell::new(Point::default())),
        content,
    };
    let observed = Rc::new(RefCell::new(None));
    let (_, cx) = cx.add_window_view({
        let handle = handle.clone();
        let observed = observed.clone();
        move |_, cx| {
            CachedRoot(cx.new(|_| Fixture {
                handle,
                observed,
                track_size,
            }))
        }
    });
    draw(cx);
    (cx, handle, observed)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn wheel_scroll_updates_thumb_hover_under_a_stationary_pointer(cx: &mut TestAppContext) {
    let (cx, handle, observed) = fixture(
        cx,
        size(px(100.), px(100.)),
        size(px(100.), px(100.)),
        size(px(100.), px(500.)),
    );
    let normal_bg = observed.borrow().as_ref().unwrap().thumb_bg;
    let pointer = point(px(95.), px(20.));
    cx.simulate_mouse_move(pointer, None, Modifiers::default());
    draw(cx);
    let hovered_bg = observed.borrow().as_ref().unwrap().thumb_bg;
    assert_ne!(hovered_bg, normal_bg);

    cx.simulate_event(ScrollEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(0.), px(-400.))),
        ..Default::default()
    });
    draw(cx);
    assert_eq!(handle.offset().y, px(-400.));
    let after = observed.borrow().as_ref().unwrap().clone();
    assert!(!after.thumb.contains(&pointer));
    assert_eq!(
        after.state.get().hovered_on_thumb,
        None,
        "scrolling must clear thumb hover without another mouse move"
    );
    assert_eq!(after.thumb_bg, normal_bg);

    cx.simulate_event(ScrollEvent {
        position: pointer,
        delta: ScrollDelta::Pixels(point(px(0.), px(400.))),
        ..Default::default()
    });
    draw(cx);
    let after = observed.borrow().as_ref().unwrap().clone();
    assert!(after.thumb.contains(&pointer));
    assert_eq!(after.state.get().hovered_on_thumb, Some(Axis::Vertical));
    assert_eq!(after.thumb_bg, hovered_bg);
}

#[gpui::test]
fn full_height_track_reads_the_handles_current_logical_viewport(cx: &mut TestAppContext) {
    let (cx, handle, observed) = fixture(
        cx,
        size(px(100.), px(200.)),
        size(px(100.), px(100.)),
        size(px(100.), px(280.)),
    );
    // Layout can change the handle's viewport after the scrollbar was built.
    handle
        .viewport
        .set(Bounds::new(Point::default(), size(px(100.), px(80.))));
    cx.update(|window, _| window.refresh());
    draw(cx);
    let geometry = observed.borrow().as_ref().unwrap().geometry;
    assert_eq!(geometry.extent, px(200.));
    let logical_thumb = geometry.length + geometry.inset * 2.;
    assert!((f32::from(logical_thumb) - 200. * 80. / 280.).abs() < 0.001);

    cx.simulate_mouse_down(
        point(px(95.), px(20.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    draw(cx);
    cx.simulate_mouse_move(
        point(px(95.), px(199.)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    draw(cx);
    cx.simulate_mouse_up(
        point(px(95.), px(199.)),
        MouseButton::Left,
        Modifiers::default(),
    );
    assert_eq!(handle.offset().y, px(-200.));
}

#[test]
fn an_explicit_logical_viewport_overrides_the_handle_policy() {
    let handle = Handle {
        viewport: Rc::new(Cell::new(Bounds::new(
            Point::default(),
            size(px(100.), px(80.)),
        ))),
        offset: Rc::new(Cell::new(Point::default())),
        content: size(px(100.), px(280.)),
    };
    let scrollbar = Scrollbar::vertical(&handle)
        .scroll_viewport_from_handle()
        .scroll_viewport_size(size(px(100.), px(60.)));
    assert_eq!(
        scrollbar.resolved_scroll_viewport_size(size(px(100.), px(200.))),
        size(px(100.), px(60.))
    );
}
