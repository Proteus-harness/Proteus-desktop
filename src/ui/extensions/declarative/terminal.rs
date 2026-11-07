//! Terminal tab: draws snapshots of the headless terminal and feeds it keys.
//!
//! Keys are taken by a keystroke interceptor while the tab is focused, so
//! shell shortcuts (Ctrl+N, Tab, Ctrl+C) reach the shell before the app's
//! bindings. Layout, pane and settings shortcuts stay with the app.

use crate::ui::kit::ButtonLabel as _;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::Size;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::terminal::{Base, Grid, Key, Signal, Terminal};
use crate::ui::kit;

use crate::ui::terminal::cells::{self, PADDING, cell_size, rgb_bytes};

enum State {
    Waiting,
    Running(Terminal),
    Failed(String, Option<Terminal>),
    Exited(Option<Terminal>),
}

impl State {
    fn terminal(&self) -> Option<&Terminal> {
        match self {
            Self::Running(terminal)
            | Self::Exited(Some(terminal))
            | Self::Failed(_, Some(terminal)) => Some(terminal),
            _ => None,
        }
    }

    fn finish(&mut self, error: Option<String>) {
        let previous = std::mem::replace(self, Self::Waiting);
        let terminal = match previous {
            Self::Running(terminal) => Some(terminal),
            Self::Exited(terminal) | Self::Failed(_, terminal) => terminal,
            Self::Waiting => None,
        };
        *self = match error {
            Some(error) => Self::Failed(error, terminal),
            None => Self::Exited(terminal),
        };
    }
}

pub(super) struct TerminalView {
    focus: FocusHandle,
    cwd: Option<PathBuf>,
    state: State,
    notice: Option<String>,
    area: Rc<Cell<Option<Size<Pixels>>>>,
    scroll_rest: f32,
    _tasks: Vec<Task<()>>,
    _intercept: Subscription,
}

impl TerminalView {
    pub(super) fn new(cwd: PathBuf, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let weak = cx.weak_entity();
        let keys = focus.clone();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if !keys.is_focused(window) {
                return;
            }
            let handled = weak
                .update(cx, |this, cx| this.key(&event.keystroke, cx))
                .unwrap_or(false);
            if handled {
                cx.stop_propagation();
            }
        });
        Self {
            focus,
            cwd: Some(cwd),
            state: State::Waiting,
            notice: None,
            area: Rc::default(),
            scroll_rest: 0.,
            _tasks: Vec::new(),
            _intercept: intercept,
        }
    }

    pub(super) fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }

    /// Release the PTY even if a cached element still holds this entity.
    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        self._tasks.clear();
        self.state = State::Exited(None);
        self.cwd = None;
        self.notice = None;
        self.area.set(None);
        cx.notify();
    }

    fn start(&mut self, grid: Grid, cx: &mut Context<Self>) {
        let Some(cwd) = self.cwd.clone() else {
            return;
        };
        let spawned = match Terminal::spawn(&cwd, grid) {
            Ok(spawned) => spawned,
            Err(error) => {
                self.state = State::Failed(format!("{error:#}"), None);
                return;
            }
        };
        let redraw = spawned.redraw;
        let signals = spawned.signals;
        self._tasks = vec![
            cx.spawn(async move |this, cx| {
                while redraw.recv().await.is_ok() {
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
            cx.spawn(async move |this, cx| {
                while let Ok(signal) = signals.recv().await {
                    let finished = matches!(signal, Signal::Exited | Signal::Failed(_));
                    let alive = this.update(cx, |this, cx| {
                        match signal {
                            Signal::Exited => {
                                this.state.finish(None);
                                this.notice = None;
                            }
                            Signal::Failed(error) => {
                                this.state.finish(Some(error));
                                this.notice = None;
                            }
                            Signal::InputRejected(error) => this.notice = Some(error),
                            Signal::Title(_) | Signal::Bell => return,
                        }
                        cx.notify();
                    });
                    if finished || alive.is_err() {
                        break;
                    }
                }
            }),
        ];
        self.state = State::Running(spawned.terminal);
    }

    fn restart(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state = State::Waiting;
        self._tasks.clear();
        self.notice = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Feeds one keystroke to the shell; `false` leaves it to the app.
    fn key(&mut self, keystroke: &Keystroke, cx: &mut Context<Self>) -> bool {
        let State::Running(terminal) = &self.state else {
            return false;
        };
        let modifiers = &keystroke.modifiers;
        if modifiers.platform || crate::ui::keys::reserved_in_terminal(keystroke, cx) {
            return false;
        }
        if modifiers.control && modifiers.shift && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                terminal.paste(&text);
            }
            return true;
        }
        if modifiers.control && modifiers.shift {
            return false;
        }
        let key = Key {
            key: &keystroke.key,
            text: keystroke.key_char.as_deref(),
            ctrl: modifiers.control,
            alt: modifiers.alt,
            shift: modifiers.shift,
        };
        let handled = terminal.key(key);
        if handled {
            self.notice = None;
            cx.notify();
        }
        handled
    }

    fn scroll(&mut self, event: &ScrollEvent, line: Pixels) {
        let Some(terminal) = self.state.terminal() else {
            return;
        };
        self.scroll_rest += event.delta.pixel_delta(line).y.as_f32() / line.as_f32();
        let lines = self.scroll_rest.trunc();
        if lines != 0. {
            self.scroll_rest -= lines;
            terminal.scroll(lines as i32);
        }
    }

    /// Fits the grid to the measured area; starts the shell once it is known.
    fn fit(&mut self, cell: Size<Pixels>, cx: &mut Context<Self>) {
        let Some(area) = self.area.get() else {
            return;
        };
        let columns = ((area.width.as_f32() - 2. * PADDING) / cell.width.as_f32()).floor();
        let rows = ((area.height.as_f32() - 2. * PADDING) / cell.height.as_f32()).floor();
        let grid = Grid::new(columns.max(0.) as usize, rows.max(0.) as usize);
        match &mut self.state {
            State::Running(terminal)
            | State::Failed(_, Some(terminal))
            | State::Exited(Some(terminal)) => terminal.resize(grid),
            State::Waiting => self.start(grid, cx),
            State::Failed(_, None) | State::Exited(None) => {}
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let family = theme.mono_font_family.clone();
        let size = theme.mono_font_size;
        let background = theme.background;
        let foreground = theme.foreground;
        let muted = theme.muted_foreground;
        let cell = cell_size(&font(family.clone()), size, window);
        self.fit(cell, cx);

        let area = self.area.clone();
        let view = cx.entity_id();
        // A new size refits the grid on the next frame, even when nothing
        // else redraws the terminal.
        let probe = canvas(
            move |bounds, _, cx| {
                if area.replace(Some(bounds.size)) != Some(bounds.size) {
                    cx.notify(view);
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let base = div()
            .id("terminal")
            .w_full()
            .flex_1()
            .min_h_0()
            .relative()
            .overflow_hidden()
            .track_focus(&self.focus)
            .bg(background)
            .font_family(family)
            .text_size(size)
            .line_height(cell.height)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
            )
            .on_scroll(cx.listener(move |this, event, _, cx| {
                this.scroll(event, cell.height);
                cx.notify();
            }))
            .child(probe);

        let notice = |text: String, cx: &mut Context<Self>| {
            v_flex()
                .flex_none()
                .items_start()
                .p(px(PADDING))
                .gap_2()
                .text_color(muted)
                .child(text)
                .child(
                    kit::button("terminal-restart")
                        .small()
                        .primary()
                        .faded_label("Перезапустить")
                        .on_click(cx.listener(|this, _, window, cx| this.restart(window, cx))),
                )
        };
        let rendered = match self.state.terminal() {
            None => base,
            Some(terminal) => {
                let mut screen = terminal.snapshot(Base {
                    foreground: rgb_bytes(foreground),
                    background: rgb_bytes(background),
                });
                if !matches!(self.state, State::Running(_)) {
                    // Unfocused live terminals show an outline cursor; an
                    // exited process has no cursor at all.
                    screen.cursor = None;
                }
                base.child(cells::screen(
                    &screen,
                    cell,
                    foreground,
                    matches!(self.state, State::Running(_)) && self.focus.is_focused(window),
                ))
            }
        };
        // Status occupies its own layout space. An overlay here would cover
        // the last output lines precisely when the shell has finished.
        let terminal = v_flex().size_full().bg(background).child(rendered);
        let terminal = match &self.state {
            State::Failed(error, _) => terminal.child(notice(format!("Терминал: {error}"), cx)),
            State::Exited(_) => terminal.child(notice("Процесс оболочки завершился.".into(), cx)),
            _ => terminal,
        };
        terminal.when_some(self.notice.clone(), |this, text| {
            this.child(
                div()
                    .flex_none()
                    .p(px(PADDING))
                    .text_color(muted)
                    .child(text),
            )
        })
    }
}
