//! How tool calls read in the thread: the tool name, the subject it acts on
//! and the remaining arguments. Tool names are matched only here, so a
//! replaced toolset degrades to the generic headline instead of breaking.

mod patch;
#[cfg(test)]
mod tests;

use serde_json::Value;

#[cfg(test)]
use patch::PatchOperation;
pub use patch::{PatchFile, patch_files, patch_line_stats, patch_stats};

pub const APPLY_PATCH: &str = "apply_patch";
pub const UPDATE_PLAN: &str = "update_plan";

/// Conventional argument names that usually carry the subject of an unknown tool.
const SUBJECT_KEYS: [&str; 8] = [
    "cmd",
    "command",
    "path",
    "file_path",
    "paths",
    "pattern",
    "query",
    "url",
];
const SUBJECT_LIMIT: usize = 300;
const ARG_LIMIT: usize = 160;
const MAX_ARGS: usize = 6;
/// Arguments that tune how a tool runs and say nothing about what it does.
const TUNING_ARGS: &[&str] = &["yield_time_ms", "max_output_tokens"];

/// The argument a known tool acts on; `""` means the tool has no subject.
fn known_subject(name: &str) -> Option<&'static str> {
    Some(match name {
        "exec_command" => "cmd",
        "shell" => "command",
        "write_stdin" => "chars",
        "read_file" | "write_file" | "edit_file" | "list_dir" | "git_diff" | "view_image" => "path",
        "read_many_files" => "paths",
        "find_files" | "grep" => "pattern",
        "web_search" => "query",
        "request_permissions" => "justification",
        "git_status" | "git_log" | APPLY_PATCH | UPDATE_PLAN => "",
        _ => return None,
    })
}

/// The headline is labelled with the tool's own name: it is what the model
/// called and what logs, approvals and settings show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headline {
    pub label: String,
    pub subject: Option<String>,
    /// Lines added and removed by a patch.
    pub stats: Option<(usize, usize)>,
    /// Argument already shown as the subject; details list only the rest.
    pub subject_key: Option<String>,
}

pub fn headline(name: &str, args: &Value) -> Headline {
    let known = known_subject(name);
    let label = name.to_owned();
    if name == APPLY_PATCH
        && let Some(patch) = patch_text(args)
    {
        let files = patch_files(patch);
        if !files.is_empty() {
            return Headline {
                label,
                subject: Some(patch_subject(&files)),
                stats: Some(patch_stats(&files)),
                subject_key: None,
            };
        }
    }
    if name == UPDATE_PLAN
        && let Some(subject) = plan_summary(args)
    {
        return Headline {
            label,
            subject: Some(subject),
            stats: None,
            subject_key: None,
        };
    }
    let key = match known {
        Some("") => None,
        Some(key) => Some(key.to_owned()),
        None => generic_subject_key(args),
    };
    let subject = key
        .as_deref()
        .and_then(|key| args.get(key).and_then(|value| subject_text(key, value)));
    Headline {
        label,
        subject_key: subject.as_ref().and(key),
        subject,
        stats: None,
    }
}

/// A known path-list subject is summarized in the header. Show its members only
/// in the expanded card, without treating arbitrary arrays as filenames.
pub fn path_list_preview(headline: &Headline, args: &Value) -> Option<String> {
    let key = headline.subject_key.as_deref()?;
    if key != "paths" || known_subject(&headline.label) != Some(key) {
        return None;
    }
    let paths = args.get(key)?.as_array()?;
    // The stock batch reader documents at most 20 paths. Do not expand
    // unbounded arrays just because supplied arguments use the same shape.
    if !(2..=20).contains(&paths.len()) {
        return None;
    }
    // Give every path its own line within a 2000-byte preview; reserve two
    // quotes, a three-byte ellipsis and a newline per path.
    let content_bytes = 2000 / paths.len() - 6;
    let mut rows = Vec::with_capacity(paths.len());
    for item in paths {
        let path = item.as_str()?;
        // Quote each filename separately: embedded line breaks and quotes
        // must not look like additional files. Bound work before escaping.
        let mut bytes = 0;
        let cut = path.char_indices().enumerate().find_map(|(i, (at, ch))| {
            // A char's Debug escape conservatively bounds its contribution
            // to the quoted string. Never cut through an escape or scalar.
            bytes += ch.escape_debug().map(char::len_utf8).sum::<usize>();
            (i == ARG_LIMIT || bytes > content_bytes).then_some(at)
        });
        rows.push(match cut {
            Some(end) => format!("{:?}…", &path[..end]),
            None => format!("{path:?}"),
        });
    }
    Some(rows.join("\n"))
}

/// Arguments not already shown in the headline, as short `key: value` pairs.
pub fn other_args(args: &Value, subject_key: Option<&str>) -> Vec<(String, String)> {
    let Some(map) = args.as_object() else {
        return Vec::new();
    };
    // Sorted so live rows and snapshot rows list arguments in the same order.
    let mut args: Vec<_> = map
        .iter()
        .filter(|(key, value)| {
            !value.is_null()
                && Some(key.as_str()) != subject_key
                && !TUNING_ARGS.contains(&key.as_str())
        })
        .collect();
    args.sort_by(|a, b| a.0.cmp(b.0));
    args.into_iter()
        .take(MAX_ARGS)
        .map(|(key, value)| (key.clone(), arg_preview(value)))
        .collect()
}

pub fn patch_text(args: &Value) -> Option<&str> {
    args.get("patch")
        .or_else(|| args.get("input"))
        .and_then(Value::as_str)
        .filter(|patch| !patch.trim().is_empty())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub step: String,
    pub status: StepStatus,
}

fn plan_entries(args: &Value) -> impl Iterator<Item = (&str, StepStatus)> {
    args.get("plan")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let step = entry.get("step").and_then(Value::as_str)?;
            let status = match entry.get("status").and_then(Value::as_str)? {
                "completed" => StepStatus::Completed,
                "in_progress" => StepStatus::InProgress,
                _ => StepStatus::Pending,
            };
            Some((step, status))
        })
}

pub fn plan_steps(args: &Value) -> Vec<PlanStep> {
    plan_entries(args)
        .map(|(step, status)| PlanStep {
            step: step.to_owned(),
            status,
        })
        .collect()
}

fn plan_summary(args: &Value) -> Option<String> {
    let mut count = 0usize;
    let mut completed = 0usize;
    let mut current = None;
    for (step, status) in plan_entries(args) {
        count += 1;
        match status {
            StepStatus::Completed => completed += 1,
            StepStatus::InProgress if current.is_none() => current = Some(step),
            _ => {}
        }
    }
    if count == 0 {
        return None;
    }

    let mut summary = format!("{completed}/{count}");
    if let Some(step) = current {
        summary.push_str(" · ");
        // The caption is limited in Unicode scalars; never copy the
        // unshown suffix of a large step into an intermediate string.
        let remaining = SUBJECT_LIMIT.saturating_sub(summary.chars().count());
        let end = step
            .char_indices()
            .nth(remaining)
            .map_or(step.len(), |(i, _)| i);
        summary.push_str(&step[..end]);
    }
    Some(summary)
}

fn patch_subject(files: &[PatchFile]) -> String {
    match files {
        [only] => only.path.clone(),
        [first, rest @ ..] => format!("{} +{}", first.path, rest.len()),
        [] => String::new(),
    }
}

fn generic_subject_key(args: &Value) -> Option<String> {
    let map = args.as_object()?;
    SUBJECT_KEYS
        .iter()
        .find(|key| {
            map.get(**key)
                .is_some_and(|value| subject_text(key, value).is_some())
        })
        .map(|key| (*key).to_owned())
        .or_else(|| {
            map.iter()
                .find(|(_, value)| {
                    value.as_str().is_some_and(|text| {
                        !text.trim().is_empty()
                            && !text.contains('\n')
                            && text.chars().count() <= 120
                    })
                })
                .map(|(key, _)| key.clone())
        })
}

fn subject_text(key: &str, value: &Value) -> Option<String> {
    let text = match value {
        Value::String(text) => return single_line(text, SUBJECT_LIMIT),
        Value::Array(items) => {
            let mut parts = items.iter().filter_map(Value::as_str);
            let first = parts.next()?;
            if let Some(second) = parts.next() {
                return Some(multi_subject(key, first, second, parts));
            }
            return single_line(first, SUBJECT_LIMIT);
        }
        Value::Number(number) => number.to_string(),
        _ => return None,
    };
    Some(limit(text, SUBJECT_LIMIT))
}

/// The multi-value branch keeps raw strings and separators, not single_line's
/// normalized whitespace. Only the prefix can reach the caption.
fn multi_subject<'a>(
    key: &str,
    first: &'a str,
    second: &'a str,
    rest: impl Iterator<Item = &'a str>,
) -> String {
    let mut remaining = SUBJECT_LIMIT;
    let first = take_chars(first, &mut remaining);
    if remaining == 0 {
        return first.to_owned();
    }
    if key.ends_with('s') {
        // Don't walk the other paths if their count cannot be shown.
        const SUFFIX_PREFIX: &str = " +";
        if remaining <= SUFFIX_PREFIX.len() {
            let mut result = String::with_capacity(first.len() + remaining);
            result.push_str(first);
            result.push_str(&SUFFIX_PREFIX[..remaining]);
            return result;
        }
        use std::fmt::Write as _;
        let others = 1 + rest.count();
        let digits = others.ilog10() as usize + 1;
        let mut result = String::with_capacity(first.len() + SUFFIX_PREFIX.len() + digits);
        result.push_str(first);
        write!(&mut result, "{SUFFIX_PREFIX}{others}").expect("writing to String cannot fail");
        // The count and its separator are ASCII, so this is a byte boundary.
        result.truncate(first.len() + remaining);
        return result;
    }

    remaining -= 1; // The first joining space.
    let second = take_chars(second, &mut remaining);
    let mut result = String::with_capacity(first.len() + 1 + second.len());
    result.push_str(first);
    result.push(' ');
    result.push_str(second);

    let mut rest = rest;
    while remaining != 0 {
        let Some(part) = rest.next() else {
            break;
        };
        remaining -= 1;
        let part = take_chars(part, &mut remaining);
        result.reserve(1 + part.len());
        result.push(' ');
        result.push_str(part);
    }
    result
}

/// Several lines become one, joined by `↵`; the row fades out what does
/// not fit.
fn single_line(text: &str, max_chars: usize) -> Option<String> {
    const SEPARATOR: &str = " ↵ ";
    let mut remaining = max_chars;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = take_chars(lines.next()?, &mut remaining);
    if remaining == 0 {
        return Some(first.to_owned());
    }
    let Some(next) = lines.next() else {
        return Some(first.to_owned());
    };
    // The bounded input size estimates a starting capacity. Leave room for
    // two separators so a short multiline caption need not grow repeatedly.
    let capacity = first.len().max(text.len().min(max_chars)) + 2 * SEPARATOR.len();
    let mut result = String::with_capacity(capacity);
    result.push_str(first);
    for line in std::iter::once(next).chain(lines) {
        let separator = take_chars(SEPARATOR, &mut remaining);
        let line = take_chars(line, &mut remaining);
        result.reserve(separator.len() + line.len());
        result.push_str(separator);
        result.push_str(line);
        if remaining == 0 {
            break;
        }
    }
    Some(result)
}

/// Borrow at most the remaining scalar budget and advance that budget.
fn take_chars<'a>(text: &'a str, remaining: &mut usize) -> &'a str {
    if text.len() <= *remaining {
        *remaining -= text.chars().count();
        return text;
    }
    for (at, _) in text.char_indices() {
        if *remaining == 0 {
            return &text[..at];
        }
        *remaining -= 1;
    }
    text
}

fn arg_preview(value: &Value) -> String {
    match value {
        Value::String(text) => single_line(text, ARG_LIMIT).unwrap_or_default(),
        Value::Array(items) => format!("[{}]", items.len()),
        Value::Object(map) => format!("{{{}}}", map.len()),
        other => other.to_string(),
    }
}

fn limit(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => text[..cut].to_owned(),
        None => text,
    }
}
