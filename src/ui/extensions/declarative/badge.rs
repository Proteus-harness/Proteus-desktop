//! Compact icon of an installed extension: its icon or the short text the
//! process puts at the top of the badge tree.

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::progress::ProgressCircle;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::badge::{Details, Press, pressable_badge, resting_badge};
use super::view::DeclarativeView;
use super::{Host, SurfaceKind, installed, runtime};
use crate::declarative::presentation::{Face, RingDisplay};
use crate::declarative::widget::Tone;
use crate::preferences::BadgePlace;
use crate::ui::kit::MetricText as _;
use crate::ui::{motion::Number, prefs};
use std::cell::Cell;

pub(super) struct DeclarativeBadge {
    pub(super) host: Host,
    pub(super) view: Entity<DeclarativeView>,
    pub(super) popup: crate::ui::motion::Popup<super::super::SurfaceView>,
    pub(super) last_popup: Option<(
        Rc<crate::declarative::widget::Node>,
        std::collections::HashMap<String, String>,
    )>,
    pub(super) popup_epoch: u64,
    pub(super) bounds: Rc<Cell<Bounds<Pixels>>>,
    pub(super) restore: Option<FocusHandle>,
    _view: Subscription,
}

pub(super) fn build(host: Host, window: &mut Window, cx: &mut App) -> AnyView {
    let runtime = runtime(&host, window, cx).expect("catalog lists installed extensions only");
    let view = cx.new(|cx| {
        DeclarativeView::new(runtime, host.clone(), SurfaceKind::Badge, None, window, cx)
    });
    cx.new(|cx| DeclarativeBadge {
        _view: cx.observe(&view, |_, _, cx| cx.notify()),
        host,
        view,
        popup: crate::ui::motion::Popup::default(),
        last_popup: None,
        popup_epoch: 0,
        bounds: Rc::default(),
        restore: None,
    })
    .into()
}

impl Render for DeclarativeBadge {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.host.extension();
        let Some(extension) = installed(id) else {
            self.close_popup(window, cx);
            self.popup.clear();
            return div().into_any_element();
        };
        let manifest = &extension.manifest;
        let icon = super::icon(manifest.icon.as_deref());
        let view = self.view.read(cx);
        let error = view.error(cx);
        let extension_ui = super::super::find(id);
        let initially_hidden = extension_ui
            .and_then(|extension| extension.badge.as_ref())
            .is_some_and(|badge| badge.initially_hidden);
        if should_hide(
            initially_hidden,
            view.presentation
                .as_ref()
                .map(|presentation| presentation.hidden),
            error.is_some(),
        ) {
            self.close_popup(window, cx);
            self.popup.clear();
            return div().into_any_element();
        }
        let epoch = view.popup_closed(cx);
        if epoch != self.popup_epoch {
            self.popup_epoch = epoch;
            self.close_popup(window, cx);
        }
        let view = self.view.read(cx);
        let mut details = Details::new(manifest.title.clone());
        if !manifest.description.is_empty() {
            details = details.line(manifest.description.clone());
        }
        let presentation = view.presentation.as_ref();
        let place = extension_ui.map_or(BadgePlace::Hidden, |extension| {
            super::super::badge_place(extension, cx)
        });
        let composer_selector = presentation.is_some_and(|presentation| {
            is_composer_selector(manifest.surfaces.popup.is_some(), place, &presentation.face)
        });
        if let Some(presentation) = presentation {
            details = Details::new(presentation.title.clone());
            for line in &presentation.lines {
                details = details.line(line.clone());
            }
        }
        if let Some(error) = error {
            details = details.line(format!("Ошибка: {error}"));
        }
        let rail = place.is_rail();
        let open = self.popup.is_open();
        let face: Rc<dyn Fn() -> AnyElement> = if let Some(presentation) = presentation {
            render_face(
                &presentation.face,
                FaceStyle {
                    id: ("badge-face", cx.entity_id()).into(),
                    icon,
                    title: details.title.clone(),
                    rail,
                    caret_open: (!composer_selector).then_some(open),
                },
                cx,
            )
        } else {
            match view.face(cx) {
                Some((text, color)) => Rc::new(move || {
                    h_flex()
                        .gap_1()
                        .child(Icon::new(icon).small())
                        .child(div().text_color(color).child(text.clone()))
                        .into_any_element()
                }),
                None => Rc::new(move || {
                    Icon::new(icon)
                        .with_size(icon_size(rail))
                        .into_any_element()
                }),
            }
        };
        let press = manifest.surfaces.popup.as_ref().map(|_| {
            let this = cx.entity().downgrade();
            Rc::new(move |window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |this, cx| this.toggle_popup(window, cx));
            }) as Press
        });
        let bounds = self.bounds.clone();
        let this = cx.entity().downgrade();
        let drag_start = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = this.update(cx, |this, cx| {
                this.close_popup(window, cx);
                // The anchor is moving: discard its inert exit too, so no
                // detached menu can follow the badge or return after Escape.
                this.popup.clear();
                cx.notify();
            });
        }) as Press;
        let popup = manifest
            .surfaces
            .popup
            .as_ref()
            .and_then(|spec| self.render_popup(spec.width, spec.height, window, cx));
        div()
            .relative()
            .on_prepaint(move |rect, _, _| bounds.set(rect))
            .child(pressable_badge(
                face,
                details,
                &self.host,
                composer_selector && open,
                press,
                Some(drag_start),
                cx,
            ))
            .children(popup)
            .into_any_element()
    }
}

pub(super) fn preview_face(extension_id: &str, id: ElementId, face: &Face, cx: &App) -> AnyElement {
    let Some(manifest) = super::super::find(extension_id) else {
        return div().into_any_element();
    };
    let Some(badge) = &manifest.badge else {
        return div().into_any_element();
    };
    if badge.narrow && !face.narrow() {
        return div().into_any_element();
    }
    let place = match super::super::badge_place(manifest, cx) {
        BadgePlace::Hidden => badge.place,
        place => place,
    };
    let has_popup = installed(extension_id)
        .is_some_and(|extension| extension.manifest.surfaces.popup.is_some());
    let rail = place.is_rail();
    let face = render_face(
        face,
        FaceStyle {
            id,
            icon: manifest.icon,
            title: manifest.title.into(),
            rail,
            caret_open: (!is_composer_selector(has_popup, place, face)).then_some(false),
        },
        cx,
    );
    resting_badge(rail, cx)
        .flex_none()
        .child(face())
        .into_any_element()
}

fn is_composer_selector(has_popup: bool, place: BadgePlace, face: &Face) -> bool {
    has_popup
        && matches!(place, BadgePlace::ComposerStart | BadgePlace::ComposerEnd)
        && matches!(face, Face::Text { caret: true, .. })
}

fn icon_size(rail: bool) -> gpui_kit::component::Size {
    if rail {
        gpui_kit::component::Size::Medium
    } else {
        gpui_kit::component::Size::Small
    }
}

struct FaceStyle {
    id: ElementId,
    icon: IconName,
    title: SharedString,
    rail: bool,
    caret_open: Option<bool>,
}

fn render_face(face: &Face, style: FaceStyle, cx: &App) -> Rc<dyn Fn() -> AnyElement> {
    let FaceStyle {
        id,
        icon,
        title,
        rail,
        caret_open,
    } = style;
    let icon_size = icon_size(rail);
    match face {
        Face::Icon { tone, count } => {
            let tone = super::render::icon_tone(*tone);
            let count = *count;
            Rc::new(move || {
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(prefs::scaled(22.))
                    .child(Icon::new(icon).with_size(icon_size).tone(tone))
                    .children(count.filter(|count| *count > 0).map(|count| {
                        div()
                            .absolute()
                            .right_0()
                            .bottom_0()
                            .compact_metric()
                            .child(if count > 99 {
                                "99+".to_string()
                            } else {
                                count.to_string()
                            })
                    }))
                    .into_any_element()
            })
        }
        Face::Text {
            text,
            tone,
            icon: show_icon,
            caret,
        } => {
            let show_icon = *show_icon;
            let caret_open = caret_open.filter(|_| *caret);
            // Default is ordinary readable text, as in Node::Text.
            // Only an explicitly muted face inherits the badge tint.
            let color =
                (!matches!(tone, Tone::Muted)).then(|| super::render::tone_color(*tone, cx));
            let text = text.clone();
            let chevron_color = cx.theme().muted_foreground;
            let animate_chevron = crate::ui::motion::moves(cx);
            Rc::new(move || {
                h_flex()
                    .gap_1()
                    .when_some(color, |face, color| face.text_color(color))
                    .children(show_icon.then(|| Icon::new(icon).small()))
                    .child(text.clone())
                    .children(caret_open.map(|open| {
                        crate::ui::motion::menu_chevron(
                            (id.clone(), "chevron"),
                            open,
                            chevron_color,
                            animate_chevron,
                        )
                    }))
                    .into_any_element()
            })
        }
        Face::Ring {
            percent,
            tone,
            display,
        } => {
            let (percent, display) = (*percent, *display);
            let color = if display == RingDisplay::Number {
                super::render::tone_color(*tone, cx)
            } else {
                crate::ui::theme::glyph_color(super::render::icon_tone(*tone), cx.theme())
            };
            Rc::new(move || metric_face(id.clone(), title.clone(), percent, color, display))
        }
    }
}

fn metric_face(
    id: ElementId,
    label: SharedString,
    percent: f32,
    color: Hsla,
    display: RingDisplay,
) -> AnyElement {
    let number = |format| {
        div()
            .line_height(relative(1.))
            .font_features(FontFeatures(Arc::new(vec![("tnum".into(), 1)])))
            .child(Number::new((id.clone(), "number"), percent, format))
    };
    if display == RingDisplay::Number {
        return number(|value| format!("{value:.0}%"))
            .text_color(color)
            .into_any_element();
    }
    let diameter = prefs::scaled(20.);
    div()
        .relative()
        .size(diameter)
        .child(
            ProgressCircle::new(id.clone())
                .value(percent)
                .color(color)
                .accessibility_label(label)
                .size(diameter),
        )
        .when(display == RingDisplay::RingNumber, |face| {
            face.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        number(|value| format!("{value:.0}"))
                            .text_size(prefs::scaled(8.))
                            .font_semibold(),
                    ),
            )
        })
        .into_any_element()
}

/// Absence is hidden only by explicit manifest policy. Errors always remain
/// reachable, including a failure after the package previously hid its badge.
fn should_hide(initially_hidden: bool, presentation_hidden: Option<bool>, error: bool) -> bool {
    !error && presentation_hidden.unwrap_or(initially_hidden)
}

#[cfg(test)]
mod tests {
    use super::should_hide;

    #[test]
    fn empty_context_badges_wait_for_content_but_errors_remain_visible() {
        assert!(!should_hide(false, None, false));
        assert!(should_hide(true, None, false));
        assert!(!should_hide(true, Some(false), false));
        assert!(should_hide(false, Some(true), false));
        assert!(!should_hide(true, None, true));
        assert!(!should_hide(true, Some(true), true));
    }
}
