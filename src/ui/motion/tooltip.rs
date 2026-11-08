//! Tooltip travel follows the same motion preference as pages and disclosures.
//! Reduced motion keeps a short fade; Off paints the final frame immediately.

use gpui_kit::base::TooltipTransition;
use gpui_kit::base::animation::{EffectTransition, ease_in_out_cubic};
use gpui_kit::{AnyElement, Div, ElementId, IntoElement, Pixels, px};

use super::{BASE, FAST, ease_enter};

#[derive(Debug, PartialEq)]
enum Effect {
    Fade,
    Enter,
    Slide(Pixels),
}

fn effect(transition: &TooltipTransition, plays: bool, moves: bool) -> Option<Effect> {
    if !plays {
        return None;
    }
    if !moves {
        return Some(Effect::Fade);
    }
    match transition {
        TooltipTransition::Enter { .. } => Some(Effect::Enter),
        TooltipTransition::Switch {
            previous, current, ..
        } if (current.origin.y - previous.origin.y).abs() < px(10.) => {
            Some(Effect::Slide(previous.center().x - current.center().x))
        }
        TooltipTransition::Switch { .. } => None,
    }
}

pub(in crate::ui) fn present(element: Div, transition: TooltipTransition) -> AnyElement {
    let Some(effect) = effect(&transition, super::plays(), super::moves()) else {
        return element.into_any_element();
    };
    let epoch = match transition {
        TooltipTransition::Enter { epoch } | TooltipTransition::Switch { epoch, .. } => epoch,
    };
    let animation = match effect {
        Effect::Fade => EffectTransition::new(FAST).ease(ease_enter).fade(0., 1.),
        Effect::Enter => EffectTransition::new(FAST)
            .ease(ease_enter)
            .slide_y(px(4.), px(0.))
            .fade(0., 1.),
        Effect::Slide(shift) => EffectTransition::new(BASE)
            .ease(ease_in_out_cubic)
            .slide_x(shift, px(0.)),
    };
    animation
        .apply(
            element,
            ElementId::NamedInteger("tip-motion".into(), epoch as u64),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Bounds, point, size};

    fn switch(y: f32) -> TooltipTransition {
        TooltipTransition::Switch {
            epoch: 1,
            previous: Bounds::new(point(px(20.), px(10.)), size(px(100.), px(30.))),
            current: Bounds::new(point(px(60.), px(y)), size(px(100.), px(30.))),
        }
    }

    #[::core::prelude::v1::test]
    fn reduced_motion_fades_both_tooltip_transitions_without_travel() {
        for transition in [
            TooltipTransition::Enter { epoch: 1 },
            switch(10.),
            switch(80.),
        ] {
            assert_eq!(effect(&transition, true, false), Some(Effect::Fade));
            assert_eq!(effect(&transition, false, false), None);
        }
    }

    #[::core::prelude::v1::test]
    fn full_motion_only_slides_between_neighbors_on_the_same_row() {
        assert_eq!(
            effect(&switch(10.), true, true),
            Some(Effect::Slide(px(-40.)))
        );
        assert_eq!(effect(&switch(80.), true, true), None);
        assert_eq!(
            effect(&TooltipTransition::Enter { epoch: 1 }, true, true),
            Some(Effect::Enter)
        );
    }
}
