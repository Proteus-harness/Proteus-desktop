use super::{Remote, Target, cells};
use crate::ui::{kit, kit::ButtonLabel as _, workspace::Workspace};
use crate::{
    api::Api,
    terminal::{Base, Display, Grid, Key},
};
use gpui_kit::Size;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

impl Remote {
    pub(super) fn new(
        target: Target,
        api: Api,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        let keys = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if keys.is_focused(window)
                && weak
                    .update(cx, |view, cx| view.key(&event.keystroke, cx))
                    .unwrap_or(false)
            {
                cx.stop_propagation();
            }
        });
        let grid = Grid::new(80, 24);
        let tty = target.terminal.tty;
        let mut view = Self {
            target,
            api,
            workspace,
            focus,
            display: None,
            area: Rc::default(),
            grid,
            remote_grid: None,
            resize_inflight: false,
            cursor: None,
            pending: None,
            reading: false,
            scheduled: false,
            finished: false,
            process_finished: false,
            input_blocked: false,
            resume_input: false,
            exit_code: None,
            gap: false,
            pipe_cr: false,
            tty,
            stopping: false,
            error: None,
            notice: None,
            scroll_rest: 0.,
            _tasks: vec![],
            _intercept: intercept,
        };
        match Display::new(grid) {
            Ok(spawned) => view.connect(spawned, cx),
            Err(_) => view.error = Some("Не удалось запустить отображение терминала".into()),
        }
        view
    }
    pub(super) fn writable(&self) -> bool {
        self.tty
            && !self.process_finished
            && !self.input_blocked
            && !self.stopping
            && self.error.is_none()
            && self.display.is_some()
    }
    fn key(&mut self, key: &Keystroke, cx: &mut Context<Self>) -> bool {
        if !self.writable()
            || !self.visible(cx.entity_id(), cx)
            || key.modifiers.platform
            || crate::ui::keys::reserved_in_terminal(key, cx)
        {
            return false;
        }
        let display = self.display.as_ref().unwrap();
        if key.modifiers.control && key.modifiers.shift && key.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                display.paste(&text);
            }
            return true;
        }
        if key.modifiers.control && key.modifiers.shift {
            return false;
        }
        display.key(Key {
            key: &key.key,
            text: key.key_char.as_deref(),
            ctrl: key.modifiers.control,
            alt: key.modifiers.alt,
            shift: key.modifiers.shift,
        })
    }
    fn fit(&mut self, cell: Size<Pixels>, cx: &mut Context<Self>) {
        let Some(area) = self.area.get() else {
            return;
        };
        let columns = ((area.width.as_f32() - 2. * cells::PADDING) / cell.width.as_f32())
            .floor()
            .max(0.) as usize;
        let rows = ((area.height.as_f32() - 2. * cells::PADDING) / cell.height.as_f32())
            .floor()
            .max(0.) as usize;
        let grid = Grid::new(columns, rows);
        if self.grid != grid {
            self.grid = grid;
            if let Some(display) = &mut self.display {
                display.resize(grid);
            }
        }
        self.ensure_resize(cx);
    }
}
impl Focusable for Remote {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Remote {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_read(cx);
        let theme = cx.theme();
        let (background, foreground, muted) =
            (theme.background, theme.foreground, theme.muted_foreground);
        let family = theme.mono_font_family.clone();
        let size = theme.mono_font_size;
        let cell = cells::cell_size(&font(family.clone()), size, window);
        self.fit(cell, cx);
        let area = self.area.clone();
        let entity = cx.entity_id();
        let probe = canvas(
            move |bounds, _, cx| {
                if area.replace(Some(bounds.size)) != Some(bounds.size) {
                    cx.notify(entity);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let screen = self.display.as_ref().map(|display| {
            display.snapshot(Base {
                foreground: cells::rgb_bytes(foreground),
                background: cells::rgb_bytes(background),
            })
        });
        let state = if self.finished {
            self.exit_code.map_or_else(
                || "Процесс завершён".to_owned(),
                |code| format!("Процесс завершён · код {code}"),
            )
        } else if self.process_finished {
            "Процесс завершён · чтение вывода…".into()
        } else if self.stopping {
            "Остановка…".into()
        } else if self.tty {
            "Терминал команды агента".into()
        } else {
            "Только просмотр".into()
        };
        let header = h_flex()
            .px_2()
            .py_1p5()
            .gap_2()
            .text_xs()
            .text_color(muted)
            .child(div().flex_1().min_w_0().child(state))
            .when(!self.process_finished, |header| {
                header.child(
                    kit::button("agent-terminal-stop")
                        .quiet(cx)
                        .small()
                        .faded_label("Остановить")
                        .disabled(self.stopping)
                        .on_click(cx.listener(|view, _, _, cx| view.stop(cx))),
                )
            });
        let body = div()
            .id("agent-terminal-screen")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .bg(background)
            .font_family(family)
            .text_size(size)
            .line_height(cell.height)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, window, cx| view.focus.focus(window, cx)),
            )
            .on_scroll(cx.listener(move |view, event: &ScrollEvent, _, cx| {
                view.scroll_rest +=
                    event.delta.pixel_delta(cell.height).y.as_f32() / cell.height.as_f32();
                let lines = view.scroll_rest.trunc();
                if lines != 0. {
                    view.scroll_rest -= lines;
                    if let Some(display) = &view.display {
                        display.scroll(lines as i32);
                    }
                    cx.notify();
                }
            }))
            .child(probe)
            .children(screen.as_ref().map(|screen| {
                cells::screen(screen, cell, foreground, self.focus.is_focused(window))
            }));
        v_flex()
            .size_full()
            .bg(background)
            .child(header)
            .child(body)
            .when(self.gap, |view| {
                view.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(muted)
                        .child("Начало вывода больше недоступно; показан оставшийся вывод."),
                )
            })
            .when_some(self.notice.clone(), |view, notice| {
                view.child(
                    div()
                        .px_2()
                        .py_1()
                        .text_xs()
                        .text_color(muted)
                        .child(notice),
                )
            })
            .when_some(
                self.error.clone().or_else(|| {
                    self.input_blocked
                        .then(|| "Ввод отключён до проверки доступности терминала".into())
                }),
                |view, error| {
                    view.child(
                        h_flex()
                            .px_2()
                            .py_1p5()
                            .gap_2()
                            .text_xs()
                            .text_color(muted)
                            .child(div().flex_1().child(error))
                            .child(
                                kit::button("agent-terminal-retry")
                                    .quiet(cx)
                                    .small()
                                    .faded_label("Повторить")
                                    .on_click(cx.listener(|view, _, _, cx| {
                                        view.error = None;
                                        view.resume_input = true;
                                        view.ensure_read(cx);
                                        cx.notify();
                                    })),
                            ),
                    )
                },
            )
    }
}
