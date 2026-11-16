use std::{cell::RefCell, rc::Rc};

use gpui::{
    BorrowAppContext as _, Context, Global, IntoElement, ParentElement as _, Render, TestApp,
    Window, WindowId, div,
};

#[derive(Clone, Default, Debug, PartialEq)]
struct Visual(u32);
impl Global for Visual {}

#[derive(Clone, Default)]
struct Agent(u32);
impl Global for Agent {}

struct Reader(Rc<RefCell<Vec<(WindowId, u32)>>>);
impl Render for Reader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = window.window_handle().window_id();
        assert_eq!(cx.current_execution_window_id(), Some(id));
        let value = cx.global::<Visual>().0;
        self.0.borrow_mut().push((id, value));
        div().child(value.to_string())
    }
}

fn app() -> TestApp {
    let mut app = TestApp::new();
    app.update(|cx| {
        cx.set_global(Visual(10));
        cx.set_global(Agent(100));
        cx.register_window_global::<Visual>();
    });
    app
}

#[test]
fn window_visual_globals_isolate_initial_draw_and_all_accessor_writes() {
    let mut app = app();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut first = app.open_window({
        let seen = seen.clone();
        move |_, cx| {
            assert_eq!(cx.global::<Visual>().0, 10);
            cx.set_global(Visual(1));
            Reader(seen)
        }
    });
    let mut second = app.open_window({
        let seen = seen.clone();
        move |_, cx| {
            assert_eq!(cx.global::<Visual>().0, 10);
            cx.set_global(Visual(2));
            Reader(seen)
        }
    });
    let first_id = first.handle().window_id();
    let second_id = second.handle().window_id();
    assert!(seen.borrow().contains(&(first_id, 1)));
    assert!(seen.borrow().contains(&(second_id, 2)));
    first.update(|_, _, cx| {
        cx.global_mut::<Visual>().0 = 3;
        cx.update_global::<Visual, _>(|value, _| value.0 += 1);
        assert_eq!(cx.remove_global::<Visual>(), Visual(4));
        assert!(!cx.has_global::<Visual>());
        assert!(cx.try_global::<Visual>().is_none());
        cx.default_global::<Visual>().0 = 5;
        assert_eq!(cx.global::<Visual>().0, 5);
        cx.set_global(Agent(200));
    });
    second.update(|_, _, cx| {
        assert_eq!(cx.global::<Visual>().0, 2);
        assert_eq!(cx.global::<Agent>().0, 200);
    });
    app.read(|cx| {
        assert_eq!(cx.current_execution_window_id(), None);
        assert_eq!(cx.global::<Visual>().0, 10);
        assert_eq!(cx.global::<Agent>().0, 200);
        assert_eq!(cx.window_global::<Visual>(first_id), Some(&Visual(5)));
        assert_eq!(cx.window_global::<Visual>(second_id), Some(&Visual(2)));
    });
}

#[test]
fn window_visual_globals_restore_nested_scopes_and_async_window_owner() {
    let mut app = app();
    let mut first = app.open_window(|_, _| Reader(Default::default()));
    let mut second = app.open_window(|_, _| Reader(Default::default()));
    let first_id = first.handle().window_id();
    let second_handle = second.handle();
    let second_id = second_handle.window_id();
    let mut first_async = first.update(|_, window, cx| {
        cx.set_global(Visual(1));
        cx.set_reduce_motion(true);
        second_handle
            .update(cx, |_, _, cx| {
                assert_eq!(cx.current_execution_window_id(), Some(second_id));
                assert_eq!(cx.global::<Visual>().0, 10);
                assert!(!cx.reduce_motion());
                cx.set_global(Visual(2));
            })
            .unwrap();
        assert_eq!(cx.current_execution_window_id(), Some(first_id));
        assert_eq!(cx.global::<Visual>().0, 1);
        assert!(cx.reduce_motion());
        window.to_async(cx)
    });
    second.update(|_, _, cx| {
        assert_eq!(cx.global::<Visual>().0, 2);
        assert!(!cx.reduce_motion());
    });
    first_async
        .update(|_, cx| {
            assert_eq!(cx.current_execution_window_id(), Some(first_id));
            assert_eq!(cx.global::<Visual>().0, 1);
            cx.set_global(Visual(3));
        })
        .unwrap();
    app.read(|cx| {
        assert_eq!(cx.global::<Visual>().0, 10);
        assert_eq!(cx.window_global::<Visual>(first_id), Some(&Visual(3)));
        assert!(!cx.reduce_motion());
    });
}

#[test]
fn immutable_and_async_trait_reads_use_their_owner_not_focus_or_app_default() {
    let mut app = app();
    let mut first = app.open_window(|_, cx| {
        cx.set_global(Visual(1));
        Reader(Default::default())
    });
    let second = app.open_window(|_, cx| {
        cx.set_global(Visual(2));
        Reader(Default::default())
    });
    let async_first = first.update(|_, window, cx| window.to_async(cx));
    assert_eq!(first.read(|_, cx| cx.global::<Visual>().0), 1);
    assert_eq!(second.read(|_, cx| cx.global::<Visual>().0), 2);
    let value = gpui::AppContext::read_global::<Visual, _>(&async_first, |value, cx| {
        assert_eq!(value.0, cx.global::<Visual>().0);
        value.0
    });
    assert_eq!(value, 1);
    app.read(|cx| assert_eq!(cx.global::<Visual>().0, 10));
}

#[test]
fn window_redraw_uses_its_owner_and_closed_windows_release_values() {
    let mut app = app();
    let seen = Rc::new(RefCell::new(Vec::new()));
    let mut first = app.open_window({
        let seen = seen.clone();
        move |_, cx| {
            cx.set_global(Visual(1));
            Reader(seen)
        }
    });
    let mut second = app.open_window(|_, cx| {
        cx.set_global(Visual(2));
        Reader(Default::default())
    });
    let first_id = first.handle().window_id();
    let first_handle: gpui::AnyWindowHandle = first.handle().into();
    let second_handle = second.handle();
    app.update(|cx| {
        // A direct draw must be entered without already borrowing its root
        // entity: drawing renders that same entity again.
        first_handle
            .update(cx, |_, window, cx| {
                seen.borrow_mut().clear();
                window.refresh();
                window.draw(cx).clear(cx);
                assert!(seen.borrow().contains(&(first_id, 1)));
            })
            .unwrap();
    });
    first.update(|_, window, _| window.remove_window());
    app.read(|cx| {
        assert!(cx.window_global::<Visual>(first_id).is_none());
        assert!(
            cx.window_global::<Visual>(second_handle.window_id())
                .is_some()
        );
    });
    second.update(|_, _, cx| assert_eq!(cx.global::<Visual>().0, 2));
}

#[test]
fn windowless_entity_updates_do_not_guess_the_focused_visual_owner() {
    let mut app = app();
    let window = app.open_window(|_, cx| {
        cx.set_global(Visual(1));
        Reader(Default::default())
    });
    let model = app.new_entity(|_| ());
    app.update_entity(&model, |_, cx| {
        assert_eq!(cx.current_execution_window_id(), None);
        assert_eq!(cx.global::<Visual>().0, 10);
        cx.set_global(Visual(99));
    });
    assert_eq!(window.read(|_, cx| cx.global::<Visual>().0), 1);
    app.read(|cx| assert_eq!(cx.global::<Visual>().0, 99));
}

#[test]
fn registering_a_visual_type_after_windows_exist_seeds_each_from_the_app_default() {
    #[derive(Clone)]
    struct LateVisual(u32);
    impl Global for LateVisual {}

    let mut app = app();
    let mut first = app.open_window(|_, _| Reader(Default::default()));
    let second = app.open_window(|_, _| Reader(Default::default()));
    app.update(|cx| {
        cx.set_global(LateVisual(7));
        cx.register_window_global::<LateVisual>();
    });
    first.update(|_, _, cx| cx.set_global(LateVisual(8)));
    assert_eq!(first.read(|_, cx| cx.global::<LateVisual>().0), 8);
    assert_eq!(second.read(|_, cx| cx.global::<LateVisual>().0), 7);
    app.read(|cx| assert_eq!(cx.global::<LateVisual>().0, 7));
}
