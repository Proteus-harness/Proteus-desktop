//! Files touched by an `apply_patch` envelope and their diff lines.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchOperation {
    Add,
    Delete,
    Update,
    Move,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLine {
    Added,
    Removed,
    Hunk,
    Context,
}

impl DiffLine {
    pub fn of(line: &str) -> Self {
        if line.starts_with("@@") {
            Self::Hunk
        } else if line.starts_with('+') {
            Self::Added
        } else if line.starts_with('-') {
            Self::Removed
        } else {
            Self::Context
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PatchFile {
    pub path: String,
    pub operation: PatchOperation,
    pub additions: usize,
    pub deletions: usize,
    /// Hunk lines without the file header.
    pub body: String,
}

enum PatchLine<'a> {
    File(PatchOperation, &'a str),
    Move(&'a str),
    Body(&'a str),
}

/// Both display paths accept the same permissive envelope, borrowing its lines.
fn patch_lines(patch: &str) -> impl Iterator<Item = PatchLine<'_>> {
    patch.lines().filter_map(|line| {
        if line == "*** Begin Patch" || line == "*** End Patch" {
            None
        } else if let Some((operation, path)) = file_header(line) {
            Some(PatchLine::File(operation, path))
        } else if let Some(target) = line.strip_prefix("*** Move to: ") {
            Some(PatchLine::Move(target))
        } else {
            Some(PatchLine::Body(line))
        }
    })
}

/// Stats for a compact row, without copying paths, formatting moves or building
/// diff bodies. `None` means no file header, as in `headline`'s fallback.
pub fn patch_line_stats(patch: &str) -> Option<(usize, usize)> {
    let mut stats = None;
    for line in patch_lines(patch) {
        match line {
            PatchLine::File(..) => {
                stats.get_or_insert((0, 0));
            }
            PatchLine::Body(line) => {
                if let Some((added, removed)) = &mut stats {
                    match DiffLine::of(line) {
                        DiffLine::Added => *added += 1,
                        DiffLine::Removed => *removed += 1,
                        DiffLine::Hunk | DiffLine::Context => {}
                    }
                }
            }
            PatchLine::Move(_) => {}
        }
    }
    stats
}

pub fn patch_files(patch: &str) -> Vec<PatchFile> {
    let mut files: Vec<PatchFile> = Vec::new();
    for line in patch_lines(patch) {
        match line {
            PatchLine::File(operation, path) => files.push(PatchFile {
                path: path.to_owned(),
                operation,
                additions: 0,
                deletions: 0,
                body: String::new(),
            }),
            PatchLine::Move(target) => {
                if let Some(file) = files.last_mut() {
                    file.operation = PatchOperation::Move;
                    file.path = format!("{} → {target}", file.path);
                }
            }
            PatchLine::Body(line) => {
                let Some(file) = files.last_mut() else {
                    continue;
                };
                match DiffLine::of(line) {
                    DiffLine::Added => file.additions += 1,
                    DiffLine::Removed => file.deletions += 1,
                    DiffLine::Hunk | DiffLine::Context => {}
                }
                if !file.body.is_empty() {
                    file.body.push('\n');
                }
                file.body.push_str(line);
            }
        }
    }
    files
}

pub fn patch_stats(files: &[PatchFile]) -> (usize, usize) {
    files.iter().fold((0, 0), |(added, removed), file| {
        (added + file.additions, removed + file.deletions)
    })
}

fn file_header(line: &str) -> Option<(PatchOperation, &str)> {
    [
        ("*** Add File: ", PatchOperation::Add),
        ("*** Delete File: ", PatchOperation::Delete),
        ("*** Update File: ", PatchOperation::Update),
    ]
    .into_iter()
    .find_map(|(prefix, operation)| line.strip_prefix(prefix).map(|path| (operation, path)))
}
