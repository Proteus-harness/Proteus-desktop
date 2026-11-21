//! Read-only unified and apply_patch diffs. Unknown source positions stay
//! unknown; a patch without line ranges must never invent line numbers.

mod parse;
#[cfg(test)]
mod tests;

use serde::Deserialize;
use std::collections::HashMap;

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
                "deleted" => "Файл удалён",
                // FileInfo alone says nothing about the contents of the file.
                _ if self.patch.is_empty() => "Текст изменений не предоставлен",
                "added" | "untracked" => "Пустой файл",
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

fn merge_metadata(files: &mut Vec<File>, info: &[FileInfo]) {
    // Short metadata lists are cheaper to scan than to index. For larger
    // lists, borrow keys from the input instead of cloning parsed paths.
    let mut by_path = (info.len() > 128).then(HashMap::new);
    if let Some(index) = &mut by_path
        && !files.is_empty()
    {
        for meta in info {
            index.insert(meta.path.as_str(), None);
        }
        for (at, file) in files.iter().enumerate() {
            if let Some(slot) = index.get_mut(file.path.as_str()) {
                // Multiple blocks may share a path. Update the first one.
                slot.get_or_insert(at);
            }
        }
    }
    for meta in info {
        let existing = if let Some(index) = &mut by_path {
            let at = index
                .entry(meta.path.as_str())
                .or_default()
                .get_or_insert(files.len());
            files.get_mut(*at)
        } else {
            files.iter_mut().find(|file| file.path == meta.path)
        };
        if let Some(file) = existing {
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
}

/// Parse the original patch once. Supplemental metadata also keeps files
/// without a textual diff visible as ordinary file cards.
pub fn parse(patch: &str, info: &[FileInfo]) -> Vec<File> {
    let mut files = parse::files(patch);
    merge_metadata(&mut files, info);
    for file in &mut files {
        file.finish();
    }
    files
}

fn pairs(lines: &[Line]) -> Vec<Pair> {
    let mut result = Vec::new();
    // In one change run the nth removal pairs with the nth addition, even
    // when the two kinds are interleaved. Fill either side in output order.
    let (mut old, mut new) = (0, 0);
    for (at, line) in lines.iter().enumerate() {
        match line.kind {
            Kind::Removed => {
                if let Some(Pair::Lines(left, _)) = result.get_mut(old) {
                    *left = Some(at);
                } else {
                    result.push(Pair::Lines(Some(at), None));
                }
                old += 1;
                continue;
            }
            Kind::Added => {
                if let Some(Pair::Lines(_, right)) = result.get_mut(new) {
                    *right = Some(at);
                } else {
                    result.push(Pair::Lines(None, Some(at)));
                }
                new += 1;
                continue;
            }
            Kind::Context => result.push(Pair::Lines(Some(at), Some(at))),
            Kind::Note => result.push(Pair::Full(at)),
            Kind::Hunk => {}
        }
        old = result.len();
        new = old;
    }
    result
}
