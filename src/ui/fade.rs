//! One-line labels that run out of room fade out at the end instead of
//! breaking off with an ellipsis. The fade lowers the alpha of the last
//! visible letters, so it suits any background, hovered or selected. A label
//! whose end matters more, such as a folder path, is cut at its start.

use std::{cell::Cell, rc::Rc};

use gpui_kit::*;

/// Width of the fade in font sizes.
const FADE: f32 = 1.;

/// One tooltip combines the title (according to actual clipping and the
/// title preference) with optional description/context. The caller supplies
/// an id inside the row/cell that owns it.
#[derive(IntoElement)]
pub(in crate::ui) struct Caption {
    id: ElementId,
    text: SharedString,
    anchor: Option<crate::ui::tip::TooltipAnchor>,
    start: bool,
    description: Option<SharedString>,
    context: Option<SharedString>,
    suffix: Option<AnyElement>,
}

pub(in crate::ui) fn caption(id: impl Into<ElementId>, text: impl Into<SharedString>) -> Caption {
    Caption {
        id: id.into(),
        text: text.into(),
        anchor: None,
        start: false,
        description: None,
        context: None,
        suffix: None,
    }
}

impl Caption {
    pub(in crate::ui) fn description(mut self, description: Option<SharedString>) -> Self {
        self.description = description.map(|text| {
            // Hover previews remain bounded even for user-supplied schemas.
            if text.chars().take(601).count() > 600 {
                format!("{}…", text.chars().take(600).collect::<String>()).into()
            } else {
                text
            }
        });
        self
    }

    pub(in crate::ui) fn context(mut self, context: Option<SharedString>) -> Self {
        self.context = context;
        self
    }

    pub(in crate::ui) fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.suffix = Some(suffix.into_any_element());
        self
    }

    /// Keep the end of a clipped path while preserving the same clip-only hint.
    pub(in crate::ui) fn fade_start(mut self) -> Self {
        self.start = true;
        self
    }

    /// The full tab/row/chip owns the tooltip position, while this caption
    /// still decides from its own visible width whether a hint is needed.
    pub(in crate::ui) fn anchor(mut self, anchor: crate::ui::tip::TooltipAnchor) -> Self {
        self.anchor = Some(anchor);
        self
    }
}

impl RenderOnce for Caption {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        use crate::ui::tip::Tip as _;
        let clipped = Rc::new(Cell::new(false));
        let text = self.text.clone();
        let label = if self.start {
            faded_start(self.text)
        } else {
            faded(self.text)
        }
        .report_clipping(clipped.clone());
        let content = match self.suffix {
            Some(suffix) => gpui_kit::component::h_flex()
                .min_w_0()
                .gap_1()
                .child(div().flex_1().min_w_0().child(label))
                .child(suffix)
                .into_any_element(),
            None => label.into_any_element(),
        };
        let resolve = move |cx: &App| {
            let hints = cx
                .try_global::<crate::ui::prefs::ClientPreferences>()
                .map(|prefs| prefs.value.hints)
                .unwrap_or_default();
            crate::ui::kit::caption_hint_text(
                hints,
                &text,
                clipped.get(),
                self.description.as_deref(),
                self.context.as_deref(),
            )
        };
        let label = div().id(self.id).relative().min_w_0().child(content);
        match self.anchor {
            Some(anchor) => label.tip_text_when_at(anchor, resolve),
            None => label.tip_text_when(resolve),
        }
    }
}

/// A cut multi-line preview fades into its actual background, using the
/// same treatment in tool output and long settings descriptions. Its
/// parent owns clipping and decides when the preview is incomplete.
pub(in crate::ui) fn bottom(background: Hsla, height: Rems) -> Div {
    div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(height)
        .bg(linear_gradient(
            180.,
            linear_color_stop(background.opacity(0.), 0.),
            linear_color_stop(background, 0.9),
        ))
}

/// A label clipped at its right edge, fading out there when it does not fit.
/// The font, colour and decorations come from the surrounding text style.
pub(in crate::ui) struct Faded {
    text: SharedString,
    clipped: Option<Rc<Cell<bool>>>,
    /// Cut at the start and keep the end visible.
    start: bool,
}

/// Line breaks become spaces: the label is a single line.
pub(in crate::ui) fn faded(text: impl Into<SharedString>) -> Faded {
    let text: SharedString = text.into();
    let text = if text.contains(['\n', '\r']) {
        text.replace(['\n', '\r'], " ").into()
    } else {
        text
    };
    Faded {
        text,
        start: false,
        clipped: None,
    }
}

/// A label clipped at its left edge, fading in there when it does not fit.
pub(in crate::ui) fn faded_start(text: impl Into<SharedString>) -> Faded {
    Faded {
        start: true,
        ..faded(text)
    }
}

impl Faded {
    /// Share the actual painted clipping with the enclosing control's single
    /// tooltip, without creating a nested hover trigger or retained entity.
    pub(in crate::ui) fn report_clipping(mut self, clipped: Rc<Cell<bool>>) -> Self {
        self.clipped = Some(clipped);
        self
    }
}

struct Metrics {
    font_size: Pixels,
    line_height: Pixels,
    style: TextStyle,
}

fn metrics(window: &Window) -> Metrics {
    let style = window.text_style();
    let font_size = style.font_size.to_pixels(window.rem_size());
    let line_height = window.pixel_snap(
        style
            .line_height
            .to_pixels(font_size.into(), window.rem_size()),
    );
    Metrics {
        font_size,
        line_height,
        style,
    }
}

impl IntoElement for Faded {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Faded {
    type RequestLayoutState = ();
    /// The shaped line, its height and how far it is shifted left.
    type PrepaintState = Option<(ShapedLine, Pixels, Pixels)>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        _: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let Metrics {
            font_size,
            line_height,
            style,
        } = metrics(window);
        let runs = [style.to_run(self.text.len())];
        let width = window
            .text_system()
            .shape_line(self.text.clone(), font_size, &runs, None)
            // Taffy snaps layout edges to pixels. A fractional intrinsic
            // width must round up, or a fitting label can lose its final
            // fraction of a pixel and incorrectly enter the fade branch.
            .width()
            .ceil();
        let mut layout = Style::default();
        layout.size.height = line_height.into();
        layout.min_size.width = px(0.).into();
        // Shrinks like clipped content: down to nothing, never past the text.
        let id = window.request_measured_layout(layout, move |known, available, _, _| {
            let width = known.width.unwrap_or(match available.width {
                AvailableSpace::Definite(room) => width.min(room),
                AvailableSpace::MinContent => px(0.),
                AvailableSpace::MaxContent => width,
            });
            size(width, line_height)
        });
        (id, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let Metrics {
            font_size,
            line_height,
            style,
        } = metrics(window);
        let text = &self.text;
        let system = window.text_system();
        let line = system.shape_line(text.clone(), font_size, &[style.to_run(text.len())], None);
        let room = bounds.size.width;
        let clipped = line.width() > room + px(0.5);
        if let Some(report) = &self.clipped {
            report.set(clipped);
        }
        if !clipped {
            return Some((line, line_height, px(0.)));
        }
        let ellipsis = cx
            .try_global::<crate::ui::prefs::ClientPreferences>()
            .is_some_and(|prefs| {
                prefs.value.label_truncation == crate::preferences::LabelTruncation::Ellipsis
            });
        if ellipsis {
            let mark: SharedString = "…".into();
            let mark_width = system
                .shape_line(mark, font_size, &[style.to_run("…".len())], None)
                .width();
            let available = (room - mark_width).max(px(0.));
            let clipped: SharedString = if self.start {
                let at = line
                    .index_for_x(line.width() - available)
                    .unwrap_or(text.len());
                let at = text.ceil_char_boundary(at);
                format!("…{}", &text[at..]).into()
            } else {
                let at = line.index_for_x(available).unwrap_or(0);
                let at = text.floor_char_boundary(at);
                format!("{}…", &text[..at]).into()
            };
            let line = system.shape_line(
                clipped.clone(),
                font_size,
                &[style.to_run(clipped.len())],
                None,
            );
            return Some((line, line_height, px(0.)));
        }
        let fade = (font_size * FADE).min(room * 0.25);
        if fade <= px(0.) {
            return None;
        }
        if self.start {
            let shift = line.width() - room;
            let mut runs = Vec::new();
            for (at, letter) in text.char_indices() {
                let end = at + letter.len_utf8();
                let middle = (line.x_for_index(at) + line.x_for_index(end)) / 2. - shift;
                let mut run = style.to_run(letter.len_utf8());
                if middle >= fade {
                    run.len = text.len() - at;
                    runs.push(run);
                    break;
                }
                run.color = run.color.opacity((middle / fade).max(0.));
                runs.push(run);
            }
            let line = system.shape_line(text.clone(), font_size, &runs, None);
            return Some((line, line_height, shift));
        }
        let start = line.index_for_x(room - fade).unwrap_or(0);
        let start = text.floor_char_boundary(start);
        let mut runs = Vec::new();
        if start > 0 {
            runs.push(style.to_run(start));
        }
        for (offset, letter) in text[start..].char_indices() {
            let at = start + offset;
            let end = at + letter.len_utf8();
            let middle = (line.x_for_index(at) + line.x_for_index(end)) / 2.;
            let mut run = style.to_run(letter.len_utf8());
            if middle >= room {
                run.len = text.len() - at;
                run.color = run.color.opacity(0.);
                runs.push(run);
                break;
            }
            run.color = run.color.opacity((room - middle) / fade);
            runs.push(run);
        }
        let line = system.shape_line(text.clone(), font_size, &runs, None);
        Some((line, line_height, px(0.)))
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some((line, line_height, shift)) = prepaint.take() else {
            return;
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            let _ = line.paint(
                bounds.origin - point(shift, px(0.)),
                line_height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        });
    }
}
