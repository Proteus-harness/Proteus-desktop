use super::{File, Kind, Line};

pub(super) fn files(patch: &str) -> Vec<File> {
    let mut files = Vec::new();
    let (mut old, mut new) = (None, None);
    let (mut hunk, mut envelope) = (false, false);
    for raw in patch.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\r', '\n']);
        if matches!(
            line,
            "*** Begin Patch" | "*** End Patch" | "*** End of File"
        ) {
            continue;
        }
        let start = if let Some(header) = line.strip_prefix("diff --git ") {
            envelope = false;
            Some(File {
                path: git_paths(header).1,
                ..File::default()
            })
        } else {
            [
                ("*** Update File: ", "modified"),
                ("*** Add File: ", "added"),
                ("*** Delete File: ", "deleted"),
            ]
            .into_iter()
            .find_map(|(prefix, status)| {
                line.strip_prefix(prefix).map(|path| {
                    envelope = true;
                    File {
                        path: path.into(),
                        status: status.into(),
                        ..File::default()
                    }
                })
            })
        };
        if let Some(mut file) = start {
            file.patch.push_str(raw);
            new = (file.status == "added").then_some(1);
            old = None;
            hunk = envelope;
            files.push(file);
            continue;
        }
        if files.is_empty() {
            if line.trim().is_empty() {
                continue;
            }
            files.push(File::default());
        }
        let file = files.last_mut().expect("a file was started");
        file.patch.push_str(raw);
        if let Some(target) = line.strip_prefix("*** Move to: ") {
            file.old_path = Some(std::mem::replace(&mut file.path, target.into()));
            file.status = "renamed".into();
        } else if line.starts_with("@@") {
            let positions = positions(line);
            (old, new) = positions.map_or((None, None), |(a, b)| (Some(a), Some(b)));
            hunk = true;
            file.lines.push(Line {
                kind: Kind::Hunk,
                text: line.into(),
                old: None,
                new: None,
            });
        } else if !hunk && line.starts_with("--- ") {
            let path = path(&line[4..]);
            if path != "/dev/null" {
                file.old_path = Some(path);
            } else {
                file.status = "added".into();
            }
        } else if !hunk && line.starts_with("+++ ") {
            let path = path(&line[4..]);
            if path != "/dev/null" {
                file.path = path;
            } else {
                file.status = "deleted".into();
                file.path = file.old_path.clone().unwrap_or_default();
            }
        } else if !hunk && line.starts_with("rename from ") {
            file.old_path = Some(unquote(&line[12..]));
            file.status = "renamed".into();
        } else if !hunk && line.starts_with("rename to ") {
            file.path = unquote(&line[10..]);
            file.status = "renamed".into();
        } else if !hunk && line.starts_with("new file mode ") {
            file.status = "added".into();
        } else if !hunk && line.starts_with("deleted file mode ") {
            file.status = "deleted".into();
        } else if line.starts_with("Binary files ") || line == "GIT binary patch" {
            file.note("Двоичный файл");
        } else if line.starts_with("\\ No newline at end of file") {
            file.note("Нет перевода строки в конце файла");
        } else if hunk {
            let (kind, text) = match line.as_bytes().first() {
                Some(b'+') => (Kind::Added, &line[1..]),
                Some(b'-') => (Kind::Removed, &line[1..]),
                Some(b' ') => (Kind::Context, &line[1..]),
                _ => {
                    file.note(line);
                    continue;
                }
            };
            file.lines.push(Line {
                kind,
                text: text.into(),
                old: (kind != Kind::Added).then_some(old).flatten(),
                new: (kind != Kind::Removed).then_some(new).flatten(),
            });
            if kind != Kind::Added {
                old = old.and_then(|n| n.checked_add(1));
            }
            if kind != Kind::Removed {
                new = new.and_then(|n| n.checked_add(1));
            }
        } else if !(line.starts_with("index ") || line.starts_with("similarity index ")) {
            file.note(line);
        }
    }
    for file in &mut files {
        if file.old_path.as_ref() == Some(&file.path) {
            file.old_path = None;
        }
    }
    files
}

fn positions(line: &str) -> Option<(usize, usize)> {
    let mut parts = line.strip_prefix("@@ ")?.split_whitespace();
    let old = parts
        .next()?
        .strip_prefix('-')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    let new = parts
        .next()?
        .strip_prefix('+')?
        .split(',')
        .next()?
        .parse()
        .ok()?;
    (parts.next()? == "@@").then_some((old, new))
}

fn path(header: &str) -> String {
    let value = unquote(header.split('\t').next().unwrap_or(header));
    let value = value
        .strip_prefix("a/")
        .or_else(|| value.strip_prefix("b/"))
        .unwrap_or(&value);
    value.strip_prefix("./").unwrap_or(value).to_owned()
}

/// Git quotes whitespace and non-ASCII bytes with C escapes, including
/// three-digit octal UTF-8 bytes. This is not JSON string escaping.
fn unquote(text: &str) -> String {
    let Some(text) = text.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return text.into();
    };
    let mut out = Vec::new();
    let mut chars = text.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(byte) = chars.next() else {
            break;
        };
        if (b'0'..=b'7').contains(&byte) {
            let mut value = (byte - b'0') as u16;
            for _ in 0..2 {
                if let Some(next) = chars.peek().copied().filter(|b| (b'0'..=b'7').contains(b)) {
                    chars.next();
                    value = value * 8 + (next - b'0') as u16;
                } else {
                    break;
                }
            }
            out.push(value as u8);
        } else {
            out.push(match byte {
                b'n' => b'\n',
                b't' => b'\t',
                b'r' => b'\r',
                other => other,
            });
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn git_paths(header: &str) -> (String, String) {
    // Unquoted names may contain spaces. The ` b/` separator is reliable
    // for Git's binary/rename headers; +++/--- later refine textual paths.
    if let Some(rest) = header.strip_prefix('"') {
        let mut escaped = false;
        for (i, ch) in rest.char_indices() {
            if ch == '"' && !escaped {
                return (path(&header[..i + 2]), path(header[i + 2..].trim_start()));
            }
            escaped = ch == '\\' && !escaped;
        }
    }
    header.split_once(" b/").map_or_else(
        || (String::new(), path(header)),
        |(a, b)| (path(a), b.into()),
    )
}
