//! The window commands' keymap, rebuilt from the shortcut preferences.

use gpui_kit::*;

use super::*;
use crate::shortcuts::{Command, Shortcuts};

fn action(command: Command) -> Box<dyn Action> {
    match command {
        Command::NewSession => Box::new(NewSession),
        Command::NewTerminal => Box::new(NewTerminal),
        Command::ToggleSettings => Box::new(ToggleSettings),
        Command::ToggleSessions => Box::new(ToggleSessions),
        Command::ToggleFiles => Box::new(ToggleFiles),
        Command::NextTab => Box::new(NextTab),
        Command::PreviousTab => Box::new(PreviousTab),
        Command::NavigateBack => Box::new(NavigateBack),
        Command::NavigateForward => Box::new(NavigateForward),
        Command::CloseTab => Box::new(CloseTab),
        Command::ReopenClosedTab => Box::new(ReopenClosedTab),
        Command::SplitRight => Box::new(SplitRight),
        Command::SplitDown => Box::new(SplitDown),
        Command::FocusPaneLeft => Box::new(FocusPaneLeft),
        Command::FocusPaneRight => Box::new(FocusPaneRight),
        Command::FocusPaneUp => Box::new(FocusPaneUp),
        Command::FocusPaneDown => Box::new(FocusPaneDown),
        Command::ZoomIn => Box::new(ZoomIn),
        Command::ZoomOut => Box::new(ZoomOut),
        Command::ZoomReset => Box::new(ZoomReset),
    }
}

fn command_of(bound: &dyn Action) -> Option<Command> {
    Command::ALL
        .into_iter()
        .find(|command| action(*command).name() == bound.name())
}

/// Replaces the commands' bindings, keeping every other binding in order.
pub(super) fn bind(shortcuts: &Shortcuts, cx: &mut App) {
    let keymap = cx.key_bindings();
    let rest: Vec<KeyBinding> = keymap
        .borrow()
        .bindings()
        .filter(|binding| command_of(binding.action()).is_none())
        .cloned()
        .collect();
    let cx = &*cx;
    let commands = Command::ALL.into_iter().flat_map(|command| {
        shortcuts
            .chords(command)
            .into_iter()
            .filter_map(move |chord| {
                KeyBinding::load(&chord, action(command), None, false, None, cx).ok()
            })
    });
    let mut keymap = keymap.borrow_mut();
    keymap.clear();
    keymap.add_bindings(rest.into_iter().chain(commands));
}

/// Whether the keystroke runs a command that wins over a focused terminal.
pub(in crate::ui) fn reserved_in_terminal(keystroke: &Keystroke, cx: &App) -> bool {
    cx.key_bindings()
        .borrow()
        .all_bindings_for_input(std::slice::from_ref(keystroke))
        .iter()
        .any(|binding| command_of(binding.action()).is_some_and(Command::in_terminal))
}
