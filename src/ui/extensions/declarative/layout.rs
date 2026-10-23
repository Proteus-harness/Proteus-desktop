// Адаптировано из Zed: crates/git_ui/src/git_panel.rs (render_tab_bar)
//! Layout containers: `fill`, which gives the remaining height to its list,
//! editor or terminal, and `tabs` with a panel tab bar.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::view::DeclarativeView;
use crate::declarative::widget::{Node, Tab};
use crate::ui::fade::faded;
use crate::ui::prefs;

/// Height of the tab bar.
const BAR: f32 = 32.;

/// Nodes that take the rest of a `fill`; the others keep their height.
fn greedy(node: &Node) -> bool {
    matches!(
        node,
        Node::Tree { .. }
            | Node::Lanes { .. }
            | Node::Code { height: None, .. }
            | Node::Terminal { .. }
            | Node::Split { .. }
            | Node::Graph { .. }
            | Node::Fill { .. }
            | Node::Tabs { .. }
    )
}

impl DeclarativeView {
    /// A column whose list, editor or terminal takes the rest of the
    /// height, so the controls above and below it stay in place.
    pub(super) fn fill(&self, children: &[Node], path: &str, cx: &mut Context<Self>) -> Div {
        let last = children.len().saturating_sub(1);
        v_flex().children(children.iter().enumerate().map(|(index, child)| {
            let path = format!("{path}.{index}");
            let element = match child {
                Node::Tabs { tabs, action } => self
                    .tabs(tabs, action.as_deref(), &path, true, cx)
                    .flex_1()
                    .min_h_0()
                    .into_any_element(),
                _ => self.node(child, &path, cx),
            };
            if greedy(child) {
                return element;
            }
            // The fill root has no padding so lists and editors reach the
            // edges; controls around them keep the usual inset.
            let padded = matches!(
                child,
                Node::Stack { .. }
                    | Node::Text { .. }
                    | Node::Select { .. }
                    | Node::Card { .. }
                    | Node::Button { .. }
                    | Node::Menu { .. }
                    | Node::Checkbox { .. }
            );
            div()
                .flex_none()
                .min_w_0()
                .when(padded, |cell| cell.px_2().pt_2())
                .when(padded && index == last, |cell| cell.pb_2())
                .child(element)
                .into_any_element()
        }))
    }

    /// Equal tabs over the body of the selected one; inside a `fill` the
    /// body is laid out as a fill too.
    pub(super) fn tabs(
        &self,
        tabs: &[Tab],
        action: Option<&str>,
        path: &str,
        filling: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        let selected = self
            .tabs
            .get(path)
            .copied()
            .unwrap_or(0)
            .min(tabs.len().saturating_sub(1));
        let theme = cx.theme();
        let (muted, border, hover, inactive) = (
            theme.muted_foreground,
            theme.border,
            theme.list_hover,
            theme.muted.opacity(0.5),
        );
        let bar = h_flex()
            .flex_none()
            .w_full()
            .h(prefs::scaled(BAR))
            .children(tabs.iter().enumerate().map(|(index, tab)| {
                let (key, action) = (path.to_string(), action.map(str::to_string));
                let active = index == selected;
                h_flex()
                    .id(ElementId::Name(format!("decl-tab-{path}-{index}").into()))
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px_2()
                    .justify_center()
                    .text_sm()
                    .cursor_pointer()
                    .when(index > 0, |tab| tab.border_l_1())
                    .border_color(border.opacity(0.6))
                    .when(!active, |tab| {
                        tab.border_b_1().bg(inactive).text_color(muted)
                    })
                    .hover(move |tab| tab.bg(hover))
                    .child(faded(tab.label.clone()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tabs.insert(key.clone(), index);
                        if !active && let Some(action) = &action {
                            this.act(action.clone(), Some(index.into()), cx);
                        }
                        cx.notify();
                    }))
            }));
        let body_path = format!("{path}.{selected}");
        let body = tabs.get(selected).map(|tab| {
            if filling {
                self.fill(&tab.children, &body_path, cx).flex_1().min_h_0()
            } else {
                v_flex()
                    .gap_1p5()
                    .children(self.children(&tab.children, &body_path, cx))
            }
        });
        v_flex()
            .when(!filling, |tabs| tabs.gap_2())
            .child(bar)
            .children(body)
    }
}
