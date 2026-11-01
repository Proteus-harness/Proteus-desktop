// Адаптировано из Zed: crates/title_bar/src/title_bar.rs (render_project_name)
// и crates/breadcrumbs/src/breadcrumbs.rs (active item context).
//! The project and command-target chat stay visible when a file has focus.
//! These are navigation links, with no second project selection state.

use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::folder_name;
use crate::preferences::SESSIONS_PANEL;
use crate::ui::extensions::OpenDocument;
use crate::ui::kit::{self, ButtonLabel as _, Hint as _};
use crate::ui::prefs;
use crate::ui::workspace::Workspace;

impl Workspace {
    pub(super) fn render_navigation_context(&self, cx: &mut Context<Self>) -> Div {
        let project = self.active_project_path();
        let path = project.display().to_string();
        let chat = self.active_chat_dir().map(std::path::Path::to_path_buf);
        h_flex()
            .min_w_0()
            .gap_0p5()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(
                kit::button("context-project")
                    .icon(Lucide::Folder)
                    .faded_label(folder_name(project))
                    .quiet(cx)
                    .small()
                    .min_w_0()
                    .max_w(prefs::scaled(220.))
                    .hint(&format!("Проект\n{path}"), cx)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_panel(SESSIONS_PANEL, window, cx);
                    })),
            )
            .when_some(chat, |row, dir| {
                let title = self.session_title(&dir, cx);
                row.child(Icon::new(Lucide::ChevronRight).xsmall()).child(
                    kit::button("context-chat")
                        .icon(Lucide::MessageSquare)
                        .faded_label(title.clone())
                        .quiet(cx)
                        .small()
                        .min_w_0()
                        .max_w(prefs::scaled(320.))
                        .hint(&format!("Вернуться к чату\n{title}"), cx)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_chat(
                                dir.clone(),
                                OpenDocument {
                                    preview: false,
                                    focus: true,
                                },
                                window,
                                cx,
                            );
                        })),
                )
            })
    }
}
