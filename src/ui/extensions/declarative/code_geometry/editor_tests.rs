//! Natural-fit fences must not scroll solely because of editor presentation.
use super::super::code_block;
use super::*;
use ::gpui::TestAppContext;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

struct Fence {
    editor: Entity<EditorState>,
    source: SharedString,
    language: &'static str,
    measured: Rc<Cell<Pixels>>,
}

impl Render for Fence {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = Width::Editor(self.editor.clone(), true);
        self.measured
            .set(width.measure(&"measured-fence".into(), window, cx));
        let height = embedded_editor_height(
            code_block::preview_rows(&self.source),
            cx.theme().mono_font_size * 1.5,
            false,
            horizontal_track_height(cx),
        );
        let source = self.source.clone();
        div().w(px(400.)).min_w_0().child(
            code_block::shell(
                "fence-copy",
                self.language,
                move |_, _| source.clone(),
                None,
                code_block::readonly_editor(&self.editor, cx)
                    .pl_1p5()
                    .h(height)
                    .into_any_element(),
                cx,
            )
            .fit(width),
        )
    }
}

#[::core::prelude::v1::test]
fn natural_fit_code_fences_do_not_report_horizontal_overflow_for_short_source() {
    let mut app = TestAppContext::single();
    app.update(|cx| {
        gpui_kit::init(cx);
        crate::ui::prefs::init_in_memory(cx);
    });
    let cx = app.add_empty_window();
    for scale in [1., 1.1] {
        cx.update(|window, cx| {
            window.set_rem_size(px(16. * scale));
            gpui_kit::component::Theme::update(cx, |theme| {
                theme.font_size = px(16. * scale);
                theme.mono_font_size = px(13. * scale);
            });
        });
        let language = "sh";
        for (text, overflowing) in [
            (String::new(), false),
            // NoopTextSystem assigns an advance even to LF. Include a
            // nonterminated row so that spare mock glyph cannot hide a
            // fractional shortage in the native editor's viewport.
            ("echo hi".into(), false),
            ("echo hi\n".into(), false),
            ("0123456789".repeat(100), true),
        ] {
            let presented = Rc::new(RefCell::new(None));
            let measured = Rc::new(Cell::new(px(0.)));
            let (editor, fence) = cx.update(|window, cx| {
                let editor = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        .language(language)
                        .rows(1)
                        .line_number(true)
                        .min_line_number_digits(1)
                        .show_trailing_empty_line(false)
                        .folding(false)
                        .soft_wrap(false)
                        .scroll_beyond_last_line(Some(0));
                    state.set_readonly(true, cx);
                    state.set_scroll_bounce(false, cx);
                    state.set_value(text.clone(), window, cx);
                    let presented = presented.clone();
                    state.set_scrollbar_renderer(Some(Rc::new(move |context, window, cx| {
                        *presented.borrow_mut() = Some(context.clone());
                        editor_scrollbar(context, window, cx)
                    })));
                    state
                });
                let fence = cx.new(|_| Fence {
                    editor: editor.clone(),
                    source: text.clone().into(),
                    language,
                    measured: measured.clone(),
                });
                (editor, fence)
            });
            for _ in 0..2 {
                cx.draw(point(px(0.), px(0.)), size(px(500.), px(600.)), |_, _| {
                    fence.clone().into_any_element()
                });
            }
            let context: EditorScrollbarContext =
                presented.borrow().clone().expect("editor must prepaint");
            let input = cx.update(|_, cx| editor.read(cx).input_bounds());
            let range = context.horizontal_scroll_range();
            assert_eq!(
                range > px(0.),
                overflowing,
                "{language}, scale={scale}, bytes={}, natural_width={:?}, input={input:?}, viewport={:?}, content={:?}, range={range:?}",
                text.len(),
                measured.get(),
                context.viewport_bounds(),
                context.content_size(),
            );
        }
    }
}
