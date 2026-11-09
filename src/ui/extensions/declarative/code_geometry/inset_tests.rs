//! Pixel-budget regression independent of the mock text system's glyph widths.
use super::super::code_block;
use super::*;
use ::gpui::TestAppContext;
use gpui_kit::base::ElementExt as _;
use std::{cell::Cell, rc::Rc};

struct Fence {
    glyph_width: Pixels,
    numbered: bool,
    viewport: Rc<Cell<Pixels>>,
}

impl Render for Fence {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = code_block::fit_width(
            self.glyph_width + embedded_editor_insets(self.numbered, window),
            px(0.),
            window,
        );
        let viewport = self.viewport.clone();
        // These are the actual separately authored readonly-editor insets;
        // the child deliberately does not force its required glyph width.
        let input = div()
            .w_full()
            .px(gpui_kit::component::Size::XSmall.input_px())
            .child(div().w_full().h(px(20.)).on_prepaint(move |bounds, _, _| {
                viewport.set(bounds.size.width);
            }));
        let body = div()
            .w_full()
            .when(self.numbered, |body| body.pl_1p5())
            .child(input);
        div().w(width).child(code_block::shell(
            "inset-copy",
            "sh",
            |_, _| SharedString::default(),
            None,
            body.into_any_element(),
            cx,
        ))
    }
}

#[::core::prelude::v1::test]
fn fitted_shell_preserves_fractional_content_after_each_inset_is_snapped() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    for scale in [0.75, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2.] {
        let pixel = cx.update(|window, _| {
            window.set_rem_size(px(16. * scale));
            px(1. / window.scale_factor())
        });
        for numbered in [false, true] {
            for glyph_width in [px(196.01), px(196.79587)] {
                let viewport = Rc::new(Cell::new(px(0.)));
                let fence = cx.update(|_, cx| {
                    cx.new(|_| Fence {
                        glyph_width,
                        numbered,
                        viewport: viewport.clone(),
                    })
                });
                cx.draw(point(px(0.), px(0.)), size(px(800.), px(200.)), |_, _| {
                    fence.into_any_element()
                });
                assert!(
                    viewport.get() >= glyph_width && viewport.get() <= glyph_width + pixel,
                    "scale={scale}, numbered={numbered}, needed={glyph_width:?}, actual={:?}",
                    viewport.get(),
                );
            }
        }
    }
}
