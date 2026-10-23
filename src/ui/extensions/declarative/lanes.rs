//! A virtualized list with a lane column, such as a commit history; what
//! the rows mean belongs to the process.

use super::lanes_paint::{self, LANE, LEFT};
use super::rows::{Kind, Rows};
use crate::declarative::control::MenuItem;
use crate::declarative::lanes::Row;
use crate::ui::fade::faded;
use crate::ui::prefs;
use crate::ui::tip::Tip as _;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::rc::Rc;

const HEIGHT: f32 = 24.;

pub(super) type Lanes = Rows<LaneRows>;

#[derive(Default)]
pub(super) struct LaneRows {
    /// Lanes the widest row uses, so every row's column is as wide.
    lanes: usize,
}

impl Kind for LaneRows {
    type Row = Row;
    const NAME: &'static str = "extension-lanes";
    const MOVED: Option<&'static str> = Some("select");
    const SCROLL: ScrollStrategy = ScrollStrategy::Nearest;
    const RING_AFTER_KEYS: bool = false;

    fn id(row: &Row) -> &str {
        &row.id
    }

    fn selected(row: &Row) -> bool {
        row.selected
    }

    fn menu(row: &Row) -> &[MenuItem] {
        &row.menu
    }

    fn clicked(count: usize) -> &'static str {
        if count > 1 { "open" } else { "select" }
    }

    fn measure(&mut self, rows: &[Rc<Row>]) {
        self.lanes = rows.iter().map(|row| row.width()).max().unwrap_or(1);
    }

    fn draw(&self, row: &Row, index: usize, ring: bool, cx: &App) -> Stateful<Div> {
        let theme = cx.theme();
        let (hover, active, primary, focus) = (
            theme.list_hover,
            theme.list_active,
            theme.primary,
            theme.ring,
        );
        let muted = theme.muted_foreground;
        let selected = row.selected;
        let painted = Rc::new(row.clone());
        let graph = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| lanes_paint::paint(bounds, &painted, window),
        )
        .flex_none()
        .h_full()
        .w(px(LEFT * 2. + self.lanes as f32 * LANE));
        let refs = row.refs.iter().map(|chip| {
            let color = super::render::tone_color(chip.tone, cx);
            div()
                .flex_none()
                .px_1()
                .rounded(px(3.))
                .border_1()
                .border_color(color.opacity(0.4))
                .bg(color.opacity(0.08))
                .text_xs()
                .line_height(prefs::scaled(14.))
                .text_color(color)
                .child(chip.label.clone())
        });
        h_flex()
            .id(("lanes-row", index))
            .relative()
            .w_full()
            .h(prefs::scaled(HEIGHT))
            .pr_2()
            .gap_1p5()
            .text_sm()
            .cursor_pointer()
            .when(selected, |element| element.bg(primary.opacity(0.1)))
            .hover(move |style| {
                style.bg(if selected {
                    primary.opacity(0.14)
                } else {
                    hover
                })
            })
            .active(move |style| {
                style.bg(if selected {
                    primary.opacity(0.18)
                } else {
                    active
                })
            })
            .child(graph)
            .child(div().min_w_0().child(faded(row.label.clone())))
            .children(refs)
            .when_some(row.detail.clone(), |element, detail| {
                element.child(
                    div()
                        .flex_none()
                        .max_w(relative(0.3))
                        .text_xs()
                        .text_color(muted)
                        .child(faded(detail)),
                )
            })
            .child(div().flex_1())
            .when_some(row.time.clone(), |element, time| {
                element.child(div().flex_none().text_xs().text_color(muted).child(time))
            })
            .when(ring, |element| {
                element.child(div().absolute().inset_0().border_1().border_color(focus))
            })
            .when_some(row.hint.clone(), |element, hint| element.tip_text(hint))
    }
}
