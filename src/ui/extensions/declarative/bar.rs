// Адаптировано из Zed: crates/git_ui/src/git_panel.rs (render_footer, render_previous_commit, PanelRepoFooter, panel_editor_container)
//! Strips at the bottom of a panel. A `bar` is a compact row like the
//! footers of Zed's git panel: the part before a `spacer` sits at the start
//! and gives way when short of room, the rest at the end, buttons and menus
//! are compact. A `panel_editor` is the editor area under them: a line on
//! top and the editor background behind a `flush` input and its bar.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::button::Fit;
use super::view::DeclarativeView;
use crate::declarative::widget::Node;

/// How a bar meets what is above it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    /// A line on top and `p_1p5`, as the last commit strip.
    Line,
    /// No line and `px_2 py_1p5`, as the repository footer under a list.
    Open,
    /// Inside the editor area, `p_1p5`; a faint line once the text fills
    /// the editor, as Zed's commit footer.
    Editor { long: bool },
}

impl DeclarativeView {
    pub(super) fn bar(
        &self,
        children: &[Node],
        border: bool,
        path: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let edge = if border { Edge::Line } else { Edge::Open };
        self.strip(children, edge, path, cx)
    }

    fn strip(&self, children: &[Node], edge: Edge, path: &str, cx: &mut Context<Self>) -> Div {
        let border = cx.theme().border;
        let split = children
            .iter()
            .position(|child| matches!(child, Node::Spacer {}))
            .unwrap_or(children.len());
        let part = |range: std::ops::Range<usize>, fit: Fit, cx: &mut Context<Self>| {
            range
                .map(|index| {
                    let child = &children[index];
                    let path = format!("{path}.{index}");
                    let fit = match child {
                        Node::Button { label, .. } if label.is_empty() => Fit::Compact,
                        Node::Menu { label, .. } if label.is_empty() => Fit::Compact,
                        _ => fit,
                    };
                    match child {
                        Node::Button { .. } => super::button::button(child, &path, fit, cx),
                        Node::Menu { .. } => super::button::menu(child, &path, fit, cx),
                        _ => self.node(child, &path, cx),
                    }
                })
                .collect::<Vec<_>>()
        };
        let start = part(0..split, Fit::Shrink, cx);
        let end = part(
            (split + 1).min(children.len())..children.len(),
            Fit::Compact,
            cx,
        );
        h_flex()
            .w_full()
            .flex_none()
            .min_w_0()
            .items_center()
            .justify_between()
            .map(|bar| match edge {
                Edge::Line => bar
                    .p_1p5()
                    .gap_1p5()
                    .border_t_1()
                    .border_color(border.opacity(0.8)),
                Edge::Open => bar.px_2().py_1p5().gap_1(),
                Edge::Editor { long } => bar.p_1p5().gap_1p5().when(long, |bar| {
                    bar.border_t_1().border_color(border.opacity(0.6))
                }),
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .map(|part| match edge {
                        Edge::Open => part.gap_px(),
                        _ => part.gap_0p5(),
                    })
                    .children(start),
            )
            .child(h_flex().flex_none().gap_0p5().children(end))
    }

    pub(super) fn panel_editor(
        &self,
        children: &[Node],
        path: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let background = theme
            .highlight_theme
            .style
            .editor_background
            .unwrap_or(theme.background);
        let border = theme.border;
        let long = children.iter().any(|child| match child {
            Node::Input {
                id,
                lines,
                flush: true,
                ..
            } => self
                .fields
                .inputs
                .get(id)
                .is_some_and(|input| input.state.value(cx).matches('\n').count() >= *lines),
            _ => false,
        });
        v_flex()
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(border)
            .bg(background)
            .children(children.iter().enumerate().map(|(index, child)| {
                let path = format!("{path}.{index}");
                match child {
                    Node::Bar { children, .. } => self
                        .strip(children, Edge::Editor { long }, &path, cx)
                        .into_any_element(),
                    _ => self.node(child, &path, cx),
                }
            }))
    }
}
