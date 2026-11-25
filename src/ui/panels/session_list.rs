//! Search and keyboard control of the session list: the filter narrows it
//! by title, arrows move the cursor, Enter opens the session under it and
//! F2 renames it. Esc clears the filter, then gives the keys back to the chat.

use proteus_tabs::ChatKey;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::sessions::ItemId;
use crate::ui::workspace::Workspace;

const LIST_CONTEXT: &str = "ProteusSessionList";
const OVERDRAW: Pixels = px(240.);
/// The whole panel, the filter included.
const PANEL_CONTEXT: &str = "ProteusSessions";

gpui_kit::actions!(
    proteus_sessions,
    [
        SelectNext,
        SelectPrevious,
        SelectFirst,
        SelectLast,
        OpenSession,
        RenameSession,
        FocusFilter,
        Dismiss,
    ]
);

pub(in crate::ui) fn bind_keys(cx: &mut App) {
    let context = Some(LIST_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("up", SelectPrevious, context),
        KeyBinding::new("home", SelectFirst, context),
        KeyBinding::new("end", SelectLast, context),
        KeyBinding::new("enter", OpenSession, context),
        KeyBinding::new("f2", RenameSession, context),
        KeyBinding::new("ctrl-f", FocusFilter, Some(PANEL_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(PANEL_CONTEXT)),
    ]);
}

#[derive(PartialEq, Eq)]
pub(super) struct LayoutSlot {
    pub id: ItemId,
    pub moving: bool,
    pub focus: Option<FocusHandle>,
}

pub(in crate::ui) struct SessionList {
    pub filter: Entity<InputState>,
    /// What the filter holds, trimmed.
    pub search: String,
    /// The row the arrows are on, when they moved.
    pub cursor: Option<ChatKey>,
    /// All visible navigation targets, including those outside the viewport.
    shown: Vec<(Rc<ChatKey>, usize)>,
    slots: Vec<LayoutSlot>,
    pub(super) scroll: ListState,
    pub focus: FocusHandle,
    /// Project folders folded shut.
    pub collapsed: HashSet<PathBuf>,
    /// Project folders showing all their chats, not only the latest.
    pub expanded: HashSet<PathBuf>,
    /// Parent sessions whose child-session branch is open.
    pub branches: HashSet<ChatKey>,
}

impl SessionList {
    pub(in crate::ui) fn new(
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> (Self, Subscription) {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Поиск по сессиям"));
        let events = cx.subscribe_in(
            &filter,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.session_list.search = state.read(cx).value().trim().to_string();
                    this.continue_codex_catalogs(cx);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => this.enter_session_list(window, cx),
                _ => {}
            },
        );
        let list = Self {
            filter,
            search: String::new(),
            cursor: None,
            shown: Vec::new(),
            slots: Vec::new(),
            scroll: ListState::new(0, ListAlignment::Top, OVERDRAW),
            focus: cx.focus_handle(),
            collapsed: HashSet::new(),
            expanded: HashSet::new(),
            branches: HashSet::new(),
        };
        (list, events)
    }

    pub(super) fn fold_limit(&self, window: &Window) -> usize {
        let viewport = self.scroll.viewport_bounds().size.height;
        ((viewport + 2. * OVERDRAW) / row_height(window)).ceil() as usize
    }

    pub(super) fn set_layout(
        &mut self,
        slots: Vec<LayoutSlot>,
        shown: Vec<(Rc<ChatKey>, usize)>,
        window: &Window,
    ) {
        if self.slots != slots {
            let prefix = self
                .slots
                .iter()
                .zip(&slots)
                .take_while(|(a, b)| a == b)
                .count();
            let suffix = self.slots[prefix..]
                .iter()
                .rev()
                .zip(slots[prefix..].iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let replacement = &slots[prefix..slots.len() - suffix];
            self.scroll.splice_focusable(
                prefix..self.slots.len() - suffix,
                replacement.iter().map(|slot| slot.focus.clone()),
            );
            if !replacement.is_empty() {
                self.scroll
                    .clone()
                    .with_uniform_item_height(row_height(window));
            }
            self.slots = slots;
        }
        // Only moving list items need their dimensions recalculated at each
        // frame. The splice above also invalidates the final, settled extent.
        for (index, _) in self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.moving)
        {
            self.scroll.remeasure_items(index..index + 1);
        }
        self.shown = shown;
    }

    pub(super) fn reveal_chat(&self, key: &ChatKey) {
        if let Some(&(_, index)) = self
            .shown
            .iter()
            .find(|(candidate, _)| candidate.as_ref() == key)
        {
            self.reveal_slot(index);
        }
    }

    fn reveal_slot(&self, index: usize) {
        if self.scroll.bounds_for_item(index).is_some() {
            self.scroll.scroll_to_reveal_item(index);
        } else {
            // Home/End can jump past every measured row. Place the target
            // directly at the anchor when its measured bounds aren't known.
            self.scroll.scroll_to(ListOffset {
                item_ix: index,
                offset_in_item: px(0.),
            });
        }
    }
}

fn row_height(window: &Window) -> Pixels {
    rems(2.).to_pixels(window.rem_size())
}

impl Workspace {
    /// Registers the list keys on the panel.
    pub(super) fn session_list_actions(&self, element: Div, cx: &mut Context<Self>) -> Div {
        element
            .key_context(LIST_CONTEXT)
            .track_focus(&self.session_list.focus)
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.step_session(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| this.step_session(-1, cx)))
            .on_action(
                cx.listener(|this, _: &SelectFirst, _, cx| this.step_session(isize::MIN, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.step_session(isize::MAX, cx)))
            .on_action(cx.listener(|this, _: &OpenSession, window, cx| {
                if let Some(dir) = this.session_cursor() {
                    this.select_session(dir, 2, window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &RenameSession, window, cx| {
                if let Some(dir) = this.session_cursor() {
                    this.session_list.reveal_chat(&dir);
                    this.start_rename(dir, window, cx);
                }
            }))
    }

    /// Esc anywhere on the panel, the filter included.
    pub(super) fn session_panel_actions(&self, element: Div, cx: &mut Context<Self>) -> Div {
        element
            .key_context(PANEL_CONTEXT)
            .on_action(cx.listener(|this, _: &FocusFilter, window, cx| {
                crate::ui::search::focus(&this.session_list.filter, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &Dismiss, window, cx| this.dismiss_session_list(window, cx)),
            )
    }

    fn dismiss_session_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let filter = self.session_list.filter.clone();
        if filter.read(cx).value().is_empty() {
            self.focus_active_chat(window, cx);
            return;
        }
        filter.update(cx, |filter, cx| filter.set_value("", window, cx));
        self.session_list.search.clear();
        cx.notify();
    }

    /// The row under the cursor: where the arrows left it, or the open
    /// session.
    pub(super) fn session_cursor(&self) -> Option<ChatKey> {
        let shown = &self.session_list.shown;
        let cursor = self.session_list.cursor.as_ref();
        let active = self.active_chat_key();
        let at = |dir| shown.iter().find(|(row, _)| row.as_ref() == dir);
        cursor
            .and_then(at)
            .or_else(|| active.as_ref().and_then(at))
            .map(|(dir, _)| dir.as_ref().clone())
    }

    /// Moves the cursor by `offset` rows; `isize::MIN`/`MAX` jump to the ends.
    fn step_session(&mut self, offset: isize, cx: &mut Context<Self>) {
        let shown = &self.session_list.shown;
        if shown.is_empty() {
            return;
        }
        let current = self
            .session_cursor()
            .and_then(|dir| shown.iter().position(|(row, _)| row.as_ref() == &dir));
        let next = cursor_index(shown.len(), current, offset);
        let (dir, child) = &shown[next];
        self.session_list.reveal_slot(*child);
        self.session_list.cursor = Some(dir.as_ref().clone());
        cx.notify();
    }

    /// Enter in the filter hands the keyboard over to the list, on its
    /// first row.
    fn enter_session_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.session_list.focus.focus(window, cx);
        self.session_list.cursor = None;
        self.step_session(isize::MIN, cx);
    }
}

fn cursor_index(len: usize, current: Option<usize>, offset: isize) -> usize {
    let last = len.saturating_sub(1) as isize;
    match offset {
        isize::MIN => 0,
        isize::MAX => last as usize,
        _ => match current {
            Some(current) => (current as isize).saturating_add(offset).clamp(0, last) as usize,
            None if offset > 0 => 0,
            None => last as usize,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::cursor_index;
    #[test]
    fn list_ends_do_not_reverse_when_active_chat_is_filtered_out() {
        for current in [None, Some(1)] {
            assert_eq!(cursor_index(3, current, isize::MIN), 0);
            assert_eq!(cursor_index(3, current, isize::MAX), 2);
        }
        assert_eq!(cursor_index(3, None, 1), 0);
        assert_eq!(cursor_index(3, None, -1), 2);
    }
}
