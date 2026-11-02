//! Bounded JSONL framing, adapted from crates/codex-client/src/process.rs.

use std::io::{self, BufRead, Write};

use anyhow::{Context as _, Result, bail};

use super::{State, budget::Permit};

/// This is an encoded frame limit, not an estimate of a parsed widget tree's
/// heap. One enormous view fails explicitly instead of bypassing the budget.
pub(super) const MAX_FRAME: usize = 16 * 1024 * 1024;
pub(super) const MAX_LOG_LINE: usize = 8 * 1024;

pub(super) fn read_frame(
    reader: &mut impl BufRead,
    state: &State,
) -> Result<Option<(Vec<u8>, Permit)>> {
    // Waiting for an idle child's first bytes holds only BufReader's fixed 8KiB
    // buffer. Reserve room for a complete frame before growing the line: partial
    // frames from different children cannot deadlock each other at the byte cap.
    if reader.fill_buf()?.is_empty() {
        return Ok(None);
    }
    let mut permit = state.incoming.reserve(MAX_FRAME + 1, &state.closed)?;
    let mut output = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            // Preserve the previous reader's acceptance of a final JSON line
            // without a newline; it is still checked by the protocol parser.
            permit.shrink(output.len());
            return Ok(Some((output, permit)));
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content = newline.unwrap_or(available.len());
        if output.len().saturating_add(content) > MAX_FRAME {
            bail!("сообщение расширения превышает лимит 16 МиБ");
        }
        // `Vec`'s geometric growth would reserve up to twice the frame budget.
        if output.capacity() < output.len() + content {
            let desired = (output.capacity().max(8192) * 2).min(MAX_FRAME);
            let desired = desired.max(output.len() + content);
            output.reserve_exact(desired - output.len());
        }
        output.extend_from_slice(&available[..content]);
        reader.consume(newline.map_or(content, |index| index + 1));
        if newline.is_some() {
            permit.shrink(output.len() + 1);
            return Ok(Some((output, permit)));
        }
    }
}

/// Keep draining an overlong stderr line without retaining its unbounded tail.
pub(super) fn read_log(mut reader: impl BufRead, mut each: impl FnMut(String)) {
    let mut line = Vec::new();
    loop {
        let available = match reader.fill_buf() {
            Ok([]) | Err(_) => {
                if !line.is_empty() {
                    each(String::from_utf8_lossy(&line).into_owned());
                }
                return;
            }
            Ok(available) => available,
        };
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content = newline.unwrap_or(available.len());
        let retained = content.min(MAX_LOG_LINE - line.len());
        line.extend_from_slice(&available[..retained]);
        reader.consume(newline.map_or(content, |index| index + 1));
        if newline.is_some() {
            let text = String::from_utf8_lossy(&line);
            each(text.trim_end_matches('\r').to_owned());
            line.clear();
        }
    }
}

struct Count(usize);

impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .checked_add(bytes.len())
            .filter(|size| *size <= MAX_FRAME)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "сообщение превышает лимит 16 МиБ",
                )
            })?;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encoded_size(message: &super::Outgoing) -> Result<usize> {
    let mut count = Count(0);
    serde_json::to_writer(&mut count, message)
        .context("не удалось измерить сообщение расширения")?;
    Ok(count.0 + 1)
}
