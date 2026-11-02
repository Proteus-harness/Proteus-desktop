//! Project Ghostty's visible viewport into the existing GPUI run contract.
use crate::screen::{Base, Run, Screen, Style};
use anyhow::Result;
use ghostty_vt::Terminal;
use ghostty_vt::render::{CellIterator, RenderState, RowIterator};
use ghostty_vt::screen::CellWide;
use ghostty_vt::style::{RgbColor, Underline};

pub(super) fn snapshot(
    vt: &Terminal,
    render: &mut RenderState,
    rows: &mut RowIterator,
    cells: &mut CellIterator,
    base: Base,
) -> Result<Screen> {
    let frame = render.update(vt)?;
    let foreground = vt.fg_color()?.map(rgb).unwrap_or(base.foreground);
    let background = vt.bg_color()?.map(rgb).unwrap_or(base.background);
    let columns = frame.cols()? as usize;
    let mut output = vec![Vec::<Run>::new(); frame.rows()? as usize];
    let mut row_iter = rows.update(&frame)?;
    let mut text = String::new();
    let mut y = 0usize;
    while let Some(row) = row_iter.next() {
        let Some(out) = output.get_mut(y) else {
            break;
        };
        y += 1;
        let mut iter = cells.update(row)?;
        let mut x = 0usize;
        while let Some(cell) = iter.next() {
            let wide = cell.raw_cell()?.wide()?;
            if matches!(wide, CellWide::SpacerTail | CellWide::SpacerHead) {
                x += 1;
                continue;
            }
            let width = if wide == CellWide::Wide { 2 } else { 1 };
            let raw = cell.style()?;
            let mut foreground = cell.fg_color()?.map(rgb).unwrap_or(foreground);
            let mut background = cell
                .bg_color()?
                .map(rgb)
                .or_else(|| (background != base.background).then_some(background));
            if raw.faint {
                foreground = foreground.map(|n| (n as f32 * 0.66) as u8);
            }
            if raw.inverse {
                let bg = background.unwrap_or(base.background);
                background = Some(foreground);
                foreground = bg;
            }
            let style = Style {
                foreground,
                background,
                bold: raw.bold,
                italic: raw.italic,
                underline: raw.underline != Underline::None,
                strike: raw.strikethrough,
            };
            text.clear();
            cell.graphemes_utf8(&mut text)?;
            if text.is_empty() || raw.invisible {
                text.clear();
                text.extend(std::iter::repeat_n(' ', width));
            }
            match out.last_mut() {
                Some(run) if run.style == style && run.column + run.width == x => {
                    run.text.push_str(&text);
                    run.width += width;
                }
                _ => out.push(Run {
                    column: x,
                    width,
                    text: text.clone(),
                    style,
                }),
            }
            x += 1; // The following wide spacer occupies the second column.
        }
        trim(out);
    }
    let cursor = if frame.cursor_visible()? {
        frame
            .cursor_viewport()?
            .map(|p| (p.y as usize, p.x as usize))
    } else {
        None
    };
    let scroll = vt.scrollbar()?;
    frame.clean()?;
    Ok(Screen {
        columns,
        rows: output,
        cursor,
        scrolled: scroll
            .total
            .saturating_sub(scroll.offset)
            .saturating_sub(scroll.len) as usize,
    })
}

fn rgb(v: RgbColor) -> [u8; 3] {
    [v.r, v.g, v.b]
}
fn trim(row: &mut Vec<Run>) {
    while let Some(run) = row.last_mut() {
        if run.style.background.is_some() || run.style.underline || run.style.strike {
            break;
        }
        let kept = run.text.trim_end_matches(' ').len();
        if kept > 0 {
            let dropped = run.text.len() - kept;
            run.text.truncate(kept);
            run.width -= dropped;
            break;
        }
        row.pop();
    }
}
