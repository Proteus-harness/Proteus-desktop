//! Read-only unified and apply_patch diffs. Unknown source positions stay
//! unknown; a patch without line ranges must never invent line numbers.

mod parse;
#[cfg(test)]
mod tests;

use serde::Deserialize;

/// File information supplied alongside a patch, including files for which
/// the server cannot return text (binary, over the size limit, unavailable).
#[derive(Clone, Debug, Deserialize, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileInfo {
    pub path: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Context,
    Added,
    Removed,
    Hunk,
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    pub old: Option<usize>,
    pub new: Option<usize>,
    /// The source line has no terminating newline; metadata, not a diff row.
    pub no_newline: bool,
}

/// Two source-line indices, or a full-width hunk/notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pair {
    Lines(Option<usize>, Option<usize>),
    Full(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub lines: Vec<Line>,
    /// Visible unified rows. Hunk markers remain parser metadata only.
    pub unified: Vec<usize>,
    pub pairs: Vec<Pair>,
    pub added: usize,
    pub removed: usize,
    /// Exactly the input block, including Git metadata and newline markers.
    pub patch: String,
}

impl File {
    fn finish(&mut self) {
        if self.path.is_empty() {
            self.path = "Changes".into();
        }
        if self.lines.is_empty() {
            self.note(match self.status.as_str() {
                "added" | "untracked" => "Пустой файл",
                "deleted" => "Файл удалён",
                "renamed" => "Файл переименован без изменения содержимого",
                _ => "Содержимое не изменилось",
            });
        }
        self.added = self
            .lines
            .iter()
            .filter(|line| line.kind == Kind::Added)
            .count();
        self.removed = self
            .lines
            .iter()
            .filter(|line| line.kind == Kind::Removed)
            .count();
        self.pairs = pairs(&self.lines);
        self.unified = self
            .lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.kind != Kind::Hunk)
            .map(|(index, _)| index)
            .collect();
    }

    fn note(&mut self, text: impl Into<String>) {
        self.lines.push(Line {
            kind: Kind::Note,
            text: text.into(),
            old: None,
            new: None,
            no_newline: false,
        });
    }
}

/// Parse the original patch once. Supplemental metadata also keeps files
/// without a textual diff visible as ordinary file cards.
pub fn parse(patch: &str, info: &[FileInfo]) -> Vec<File> {
    let mut files = parse::files(patch);
    for meta in info {
        if let Some(file) = files.iter_mut().find(|file| file.path == meta.path) {
            if !meta.status.is_empty() {
                file.status.clone_from(&meta.status);
            }
            if let Some(message) = &meta.message
                && file.lines.is_empty()
            {
                file.note(message);
            }
        } else {
            let mut file = File {
                path: meta.path.clone(),
                status: meta.status.clone(),
                ..File::default()
            };
            if let Some(message) = &meta.message {
                file.note(message);
            }
            files.push(file);
        }
    }
    for file in &mut files {
        file.finish();
    }
    files
}

fn pairs(lines: &[Line]) -> Vec<Pair> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        match lines[at].kind {
            Kind::Context => {
                result.push(Pair::Lines(Some(at), Some(at)));
                at += 1;
            }
            Kind::Hunk => {
                at += 1;
            }
            Kind::Note => {
                result.push(Pair::Full(at));
                at += 1;
            }
            Kind::Added | Kind::Removed => {
                let mut old = Vec::new();
                let mut new = Vec::new();
                while at < lines.len() && matches!(lines[at].kind, Kind::Added | Kind::Removed) {
                    match lines[at].kind {
                        Kind::Added => new.push(at),
                        _ => old.push(at),
                    }
                    at += 1;
                }
                for index in 0..old.len().max(new.len()) {
                    result.push(Pair::Lines(
                        old.get(index).copied(),
                        new.get(index).copied(),
                    ));
                }
            }
        }
    }
    result
}
