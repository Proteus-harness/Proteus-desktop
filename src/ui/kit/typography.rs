// Адаптировано из Zed: crates/ui/src/styles/typography.rs
// (semantic text roles expressed in rems).
//! Compact numbers inside icons and progress rings follow the same interface
//! scale as their containing geometry; regular captions keep native text sizes.

use gpui_kit::Styled;
use gpui_kit::base::StyledExt as _;

use crate::ui::prefs;

pub(crate) trait MetricText: Styled + Sized {
    fn compact_metric(self) -> Self {
        self.text_size(prefs::scaled(9.)).font_semibold()
    }
}

impl<E: Styled> MetricText for E {}
