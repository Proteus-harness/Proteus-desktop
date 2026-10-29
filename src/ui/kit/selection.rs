// Адаптировано из Zed: crates/editor/src/editor.rs (selection Copy action).
//! A context menu over the native window selection. No parallel selection
//! state: both hit testing and the copied range belong to gpui-base.

use gpui_kit::base::TextSelection;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::{ClipboardItem, InteractiveElement, IntoElement};

use crate::ui::context::{ContextMenu, RightClickMenu as _};

pub(crate) trait SelectionCopyMenu:
    InteractiveElement + IntoElement + Sized + 'static
{
    fn selection_copy_menu(self) -> ContextMenu {
        self.right_click_menu(|menu, window, cx| {
            // Capture the native range when the menu opens, before clicking
            // an item can focus its control or clear the text's highlight.
            let selected = TextSelection::selected_text(window, cx);
            menu.item(
                PopupMenuItem::new("Копировать")
                    .disabled(selected.is_empty())
                    .on_click(move |_, _, cx| {
                        if !selected.is_empty() {
                            cx.write_to_clipboard(ClipboardItem::new_string(selected.clone()));
                        }
                    }),
            )
        })
        .at_pointer()
    }
}

impl<E: InteractiveElement + IntoElement + Sized + 'static> SelectionCopyMenu for E {}
