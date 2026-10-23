//! The bar above the tabs: the logo menu and extension icons at both ends,
//! drawn inside the window's own title bar. The project lives above the
//! composer and in the session list.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariants as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::logo::logo;
use super::projects::Projects;
use super::title::title_bar;
use crate::preferences::BadgePlace;
use crate::ui::chat::ChatState;
use crate::ui::kit;
use crate::ui::tip::Tip as _;
use crate::ui::workspace::{Status, Workspace};

/// Logo cell size before zoom.
const LOGO_CELL: f32 = 2.;

impl Workspace {
    pub(in crate::ui) fn render_top_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let workspace = cx.weak_entity();
        let ready = matches!(self.status, Status::Ready);
        let busy = self
            .chats
            .values()
            .any(|chat| matches!(chat.view.read(cx).session.state(), ChatState::Busy(_)));
        let left = h_flex()
            .gap_1()
            .items_center()
            .child(logo_button(
                busy,
                logo_menu(None, ready.then_some(workspace)),
            ))
            .when(ready, |left| {
                left.child(self.render_badge_strip(BadgePlace::HeaderStart, cx))
            });
        let right = h_flex().gap_1().items_center().when(ready, |right| {
            right.child(self.render_badge_strip(BadgePlace::HeaderEnd, cx))
        });
        title_bar(left, right, window, cx)
    }
}

/// The bar of the folder picker: only the logo menu.
pub(in crate::ui) fn picker_top_bar(
    projects: Projects,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement + use<> {
    title_bar(
        logo_button(false, logo_menu(Some(projects), None)),
        div(),
        window,
        cx,
    )
}

fn logo_button(
    busy: bool,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    crate::ui::context::menu_button(
        "top-logo-menu",
        kit::button("top-logo")
            .ghost()
            .small()
            .tip_text(if busy {
                "Proteus · агент работает"
            } else {
                "Proteus"
            })
            .child(logo(LOGO_CELL, busy)),
        false,
        menu,
    )
}

/// Projects of the picker screen, the settings of a running workspace, the
/// version and quitting.
fn logo_menu(
    projects: Option<Projects>,
    settings: Option<WeakEntity<Workspace>>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |mut menu, _, cx| {
        if let Some(projects) = &projects {
            menu = projects.items(menu, cx).separator();
        }
        if let Some(workspace) = settings.clone() {
            menu = menu
                .item(
                    PopupMenuItem::new("Настройки")
                        .icon(Icon::new(Lucide::Settings))
                        .on_click(move |_, window, cx| {
                            workspace
                                .update(cx, |this, cx| this.toggle_settings(window, cx))
                                .ok();
                        }),
                )
                .separator();
        }
        menu.label(crate::version::LABEL).item(
            PopupMenuItem::new("Выход")
                .icon(Icon::new(Lucide::LogOut))
                .on_click(|_, _, cx| cx.quit()),
        )
    }
}
