//! One read and one ordered input write in flight, scoped to the exact view.
use super::{Remote, pending::Pending};
use crate::terminal::Signal;
use gpui_kit::*;

impl Remote {
    pub(super) fn connect(
        &mut self,
        spawned: crate::terminal::SpawnedDisplay,
        cx: &mut Context<Self>,
    ) {
        self.display = Some(spawned.display);
        let redraw = spawned.redraw;
        let signals = spawned.signals;
        let input = spawned.input;
        let api = self.api.clone();
        let target = self.target.clone();
        self._tasks.push(cx.spawn(async move |this, cx| {
            while redraw.recv().await.is_ok() {
                if this
                    .update(cx, |view, cx| {
                        if view.visible(cx.entity_id(), cx) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        self._tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(signal) = signals.recv().await {
                if this
                    .update(cx, |view, cx| {
                        if !view.current(cx.entity_id(), cx) {
                            return;
                        }
                        match signal {
                            Signal::Failed(_) => {
                                view.error = Some("Не удалось отобразить терминал".into())
                            }
                            Signal::InputRejected(_) => {
                                view.notice =
                                    Some("Очередь ввода заполнена; ввод не отправлен".into())
                            }
                            _ => return,
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
        self._tasks.push(cx.spawn(async move |this, cx| {
            while let Ok(bytes) = input.recv().await {
                let permitted = this
                    .read_with(cx, |view, cx| {
                        view.current(this.entity_id(), cx) && view.writable()
                    })
                    .unwrap_or(false);
                if !permitted {
                    continue;
                }
                let api = api.clone();
                let session = target.session_dir.clone();
                let id = target.terminal.id;
                // No retry. Bytes live only in this bounded input queue/request.
                let result = cx
                    .background_executor()
                    .spawn(async move { api.terminal_input(&session, id, &bytes) })
                    .await;
                if result.is_err() {
                    while input.try_recv().is_ok() {}
                }
                if this
                    .update(cx, |view, cx| {
                        if !view.current(cx.entity_id(), cx) {
                            return;
                        }
                        if result.is_err() {
                            view.notice =
                                Some("Ввод не подтверждён; повторной отправки не было".into());
                            view.input_blocked = true;
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    pub(super) fn ensure_read(&mut self, cx: &mut Context<Self>) {
        if self.reading
            || self.scheduled
            || self.error.is_some()
            || !self.visible(cx.entity_id(), cx)
        {
            return;
        }
        if !self.apply_pending() {
            self.schedule(cx);
            return;
        }
        if self.finished {
            return;
        }
        self.reading = true;
        let api = self.api.clone();
        let target = self.target.clone();
        let cursor = self.cursor;
        cx.spawn(async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        api.terminal_read(&target.session_dir, target.terminal.id, cursor)
                    })
                    .await;
            this.update(cx, |view, cx| {
                view.reading = false;
                if !view.current(cx.entity_id(), cx) {
                    return;
                }
                match result.and_then(|snapshot| Pending::new(snapshot, cursor)) {
                    Ok(pending) => {
                        view.pending = Some(pending);
                        view.apply_pending();
                    }
                    Err(_) => {
                        view.error = Some(
                            "Терминал недоступен. Возможно, процесс или сервер уже завершён."
                                .into(),
                        )
                    }
                }
                view.schedule(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn apply_pending(&mut self) -> bool {
        let Some(pending) = &mut self.pending else {
            return true;
        };
        let Some(display) = &self.display else {
            return false;
        };
        if pending.reset {
            if !display.reset() {
                return false;
            }
            pending.reset = false;
            self.gap = true;
            self.pipe_cr = false;
        }
        if !self.target.terminal.tty || !pending.snapshot.tty {
            pending.pipe_newlines(self.pipe_cr);
        }
        let bytes = pending.bytes.take().expect("pending output owns bytes");
        if let Err(bytes) = display.try_feed(bytes) {
            pending.bytes = Some(bytes);
            return false;
        }
        let pending = self.pending.take().unwrap();
        if let Some(cr) = pending.pipe_end {
            self.pipe_cr = cr;
        }
        self.cursor = Some(pending.snapshot.next_cursor);
        self.tty = self.target.terminal.tty && pending.snapshot.tty;
        self.finished = pending.snapshot.finished
            && pending.snapshot.next_cursor == pending.snapshot.end_cursor;
        self.process_finished = pending.snapshot.finished;
        if self.resume_input {
            self.input_blocked = false;
            self.resume_input = false;
            self.notice = None;
        }
        self.exit_code = pending.snapshot.exit_code;
        true
    }

    pub(super) fn schedule(&mut self, cx: &mut Context<Self>) {
        if self.scheduled
            || self.error.is_some()
            || self.finished
            || !self.visible(cx.entity_id(), cx)
        {
            return;
        }
        self.scheduled = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(crate::ui::motion::TERMINAL_REFRESH)
                .await;
            this.update(cx, |view, cx| {
                view.scheduled = false;
                view.ensure_read(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn ensure_resize(&mut self, cx: &mut Context<Self>) {
        if !self.writable()
            || self.resize_inflight
            || self.remote_grid == Some(self.grid)
            || !self.visible(cx.entity_id(), cx)
        {
            return;
        }
        self.resize_inflight = true;
        let grid = self.grid;
        let api = self.api.clone();
        let target = self.target.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    api.terminal_resize(
                        &target.session_dir,
                        target.terminal.id,
                        grid.columns as u16,
                        grid.rows as u16,
                    )
                })
                .await;
            this.update(cx, |view, cx| {
                view.resize_inflight = false;
                if !view.current(cx.entity_id(), cx) {
                    return;
                }
                // An uncertain response must not cause the same resize to loop.
                view.resize_finished(grid, result.is_err());
                view.ensure_resize(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn resize_finished(&mut self, grid: crate::terminal::Grid, failed: bool) {
        self.remote_grid = Some(grid);
        if failed && !self.process_finished {
            self.input_blocked = true;
            self.notice = Some("Изменение размера терминала не подтверждено".into());
        }
    }

    pub(super) fn stop(&mut self, cx: &mut Context<Self>) {
        if self.process_finished || self.stopping || !self.current(cx.entity_id(), cx) {
            return;
        }
        self.stopping = true;
        let api = self.api.clone();
        let target = self.target.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api.terminal_stop(&target.session_dir, target.terminal.id) })
                .await;
            this.update(cx, |view, cx| {
                if !view.current(cx.entity_id(), cx) {
                    return;
                }
                view.error = None;
                if result.is_err() {
                    view.stopping = false;
                    view.input_blocked = true;
                    view.notice = Some("Остановка процесса не подтверждена".into());
                }
                view.ensure_read(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
