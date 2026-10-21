//! Marks the user keeps on saved sessions on this device: a custom title,
//! pinning and the archive. The server knows nothing about them; deleting a
//! session goes to the server and only forgets the marks here.

use std::collections::BTreeMap;
use std::path::Path;

use proteus_contracts::app_protocol::AppSessionSummary;
use serde::{Deserialize, Serialize};

/// Longest custom title kept; longer input is cut.
const TITLE_LIMIT: usize = 120;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionMark {
    pub title: Option<String>,
    pub pinned: bool,
    pub archived: bool,
}

impl SessionMark {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Marks by session directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionMarks(BTreeMap<String, SessionMark>);

fn key(dir: &Path) -> String {
    dir.to_string_lossy().into_owned()
}

impl SessionMarks {
    pub fn get(&self, dir: &Path) -> SessionMark {
        self.0.get(&key(dir)).cloned().unwrap_or_default()
    }

    fn change(&mut self, dir: &Path, change: impl FnOnce(&mut SessionMark)) {
        let key = key(dir);
        let mark = self.0.entry(key.clone()).or_default();
        change(mark);
        if mark.is_empty() {
            self.0.remove(&key);
        }
    }

    /// Sets a custom title; blank input returns to the automatic one.
    pub fn rename(&mut self, dir: &Path, title: &str) {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = (!title.is_empty()).then(|| title.chars().take(TITLE_LIMIT).collect());
        self.change(dir, |mark| mark.title = title);
    }

    pub fn set_pinned(&mut self, dir: &Path, pinned: bool) {
        self.change(dir, |mark| {
            mark.pinned = pinned;
            if pinned {
                mark.archived = false;
            }
        });
    }

    /// Archiving hides a session from the list and unpins it.
    pub fn set_archived(&mut self, dir: &Path, archived: bool) {
        self.change(dir, |mark| {
            mark.archived = archived;
            if archived {
                mark.pinned = false;
            }
        });
    }

    pub fn forget(&mut self, dir: &Path) {
        self.0.remove(&key(dir));
    }

    /// Title the user gave a session, if any.
    pub fn title(&self, dir: &Path) -> Option<&str> {
        self.0.get(&key(dir))?.title.as_deref()
    }

    /// The list as shown: pinned sessions first, then the rest in folders
    /// of their projects, the project that changed last on top and server
    /// order within a project; archived ones only when asked for or
    /// searched, after everything else. A search keeps the sessions whose
    /// title holds it, case aside.
    pub fn arrange<'a>(&self, sessions: &'a [AppSessionSummary], view: &View) -> Listing<'a> {
        let search = view.search.trim().to_lowercase();
        let mut listing = Listing::default();
        for summary in sessions {
            let mark = self.get(&summary.session_dir);
            let row = Row { summary, mark };
            if !search.is_empty() && !row.title_holds(&search) {
                continue;
            }
            if row.mark.archived {
                listing.archived_count += 1;
                if view.archived || !search.is_empty() {
                    listing.archived.push(row);
                }
            } else if row.mark.pinned {
                listing.pinned.push(row);
            } else {
                let path = summary.workspace_path.as_path();
                match listing
                    .projects
                    .iter_mut()
                    .find(|project| project.path == path)
                {
                    Some(project) => project.rows.push(row),
                    None => listing.projects.push(Project {
                        path,
                        rows: vec![row],
                    }),
                }
            }
        }
        listing
            .projects
            .sort_by_key(|project| std::cmp::Reverse(project.updated_at_ms()));
        listing
    }
}

/// What the list is asked to show.
pub struct View<'q> {
    pub archived: bool,
    pub search: &'q str,
}

pub struct Row<'a> {
    pub summary: &'a AppSessionSummary,
    pub mark: SessionMark,
}

impl Row<'_> {
    fn title_holds(&self, search: &str) -> bool {
        let title = self
            .mark
            .title
            .as_deref()
            .or(self.summary.preview.as_deref());
        title.is_some_and(|title| title.to_lowercase().contains(search))
    }
}

/// The sessions of one workspace folder.
pub struct Project<'a> {
    pub path: &'a Path,
    pub rows: Vec<Row<'a>>,
}

impl Project<'_> {
    /// When a session of the project last changed.
    fn updated_at_ms(&self) -> u64 {
        self.rows
            .iter()
            .filter_map(|row| row.summary.updated_at_ms)
            .max()
            .unwrap_or(0)
    }
}

#[derive(Default)]
pub struct Listing<'a> {
    pub pinned: Vec<Row<'a>>,
    /// Projects with sessions to show, the one that changed last first.
    pub projects: Vec<Project<'a>>,
    pub archived: Vec<Row<'a>>,
    /// Archived sessions, shown or not.
    pub archived_count: usize,
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use proteus_contracts::app_protocol::AppSessionSummary;

    use super::{SessionMarks, View};

    fn view(archived: bool, search: &str) -> View<'_> {
        View { archived, search }
    }

    fn summary(dir: &str) -> AppSessionSummary {
        changed(dir, "/w", None, None)
    }

    fn changed(
        dir: &str,
        workspace: &str,
        updated_at_ms: Option<u64>,
        preview: Option<&str>,
    ) -> AppSessionSummary {
        serde_json::from_value(serde_json::json!({
            "session_dir": dir,
            "session_id": "00000000-0000-0000-0000-000000000000",
            "workspace_path": workspace,
            "message_count": 1,
            "preview": preview,
            "updated_at_ms": updated_at_ms,
        }))
        .expect("summary")
    }

    fn dirs<'r>(rows: impl IntoIterator<Item = &'r super::Row<'r>>) -> Vec<PathBuf> {
        rows.into_iter()
            .map(|row| row.summary.session_dir.clone())
            .collect()
    }

    /// Rows in the order they are shown.
    fn shown(listing: &super::Listing<'_>) -> Vec<PathBuf> {
        let projects = listing.projects.iter().flat_map(|project| &project.rows);
        dirs(
            listing
                .pinned
                .iter()
                .chain(projects)
                .chain(&listing.archived),
        )
    }

    #[test]
    fn titles_are_trimmed_and_blank_ones_dropped() {
        let mut marks = SessionMarks::default();
        let dir = Path::new("/s/a");
        assert_eq!(marks.title(dir), None);
        marks.rename(dir, "  Сборка \n  релиза ");
        assert_eq!(marks.title(dir), Some("Сборка релиза"));
        marks.rename(dir, "   ");
        assert_eq!(marks.title(dir), None);
        assert_eq!(marks, SessionMarks::default(), "empty marks are dropped");
    }

    #[test]
    fn pinned_first_and_archive_hidden() {
        let sessions = [
            summary("/s/a"),
            summary("/s/b"),
            summary("/s/c"),
            summary("/s/d"),
        ];
        let mut marks = SessionMarks::default();
        marks.set_pinned(Path::new("/s/c"), true);
        marks.set_archived(Path::new("/s/b"), true);
        let listing = marks.arrange(&sessions, &view(false, ""));
        assert_eq!(dirs(&listing.pinned), [PathBuf::from("/s/c")]);
        assert_eq!(
            shown(&listing)[1..],
            [PathBuf::from("/s/a"), PathBuf::from("/s/d")]
        );
        assert!(listing.archived.is_empty());
        assert_eq!(listing.archived_count, 1);
        let listing = marks.arrange(&sessions, &view(true, ""));
        assert_eq!(dirs(&listing.archived), [PathBuf::from("/s/b")]);

        // Archiving unpins; pinning brings a session back from the archive.
        marks.set_archived(Path::new("/s/c"), true);
        assert!(!marks.get(Path::new("/s/c")).pinned);
        marks.set_pinned(Path::new("/s/b"), true);
        assert!(!marks.get(Path::new("/s/b")).archived);
        marks.forget(Path::new("/s/b"));
        assert_eq!(marks.get(Path::new("/s/b")), Default::default());
    }

    #[test]
    fn sessions_group_by_project_latest_first() {
        let sessions = [
            changed("/s/a1", "/a", Some(10), None),
            changed("/s/b1", "/b", Some(30), None),
            changed("/s/a2", "/a", Some(20), None),
            changed("/s/c1", "/c", None, None),
        ];
        let listing = SessionMarks::default().arrange(&sessions, &view(false, ""));
        let projects: Vec<_> = listing
            .projects
            .iter()
            .map(|project| project.path)
            .collect();
        assert_eq!(
            projects,
            [Path::new("/b"), Path::new("/a"), Path::new("/c")]
        );
        assert_eq!(
            shown(&listing),
            ["/s/b1", "/s/a1", "/s/a2", "/s/c1"].map(PathBuf::from)
        );
    }

    #[test]
    fn search_matches_titles_and_reaches_the_archive() {
        let sessions = [
            changed("/s/a", "/w", Some(0), Some("Почини сборку релиза")),
            changed("/s/b", "/w", Some(0), Some("Обзор тестов")),
            changed("/s/c", "/w", Some(0), None),
            changed("/s/d", "/w", Some(0), Some("Сборка")),
        ];
        let mut marks = SessionMarks::default();
        marks.rename(Path::new("/s/c"), "Ночная СБОРКА");
        marks.set_archived(Path::new("/s/d"), true);
        let listing = marks.arrange(&sessions, &view(false, "  сборк "));
        assert_eq!(shown(&listing), ["/s/a", "/s/c", "/s/d"].map(PathBuf::from));
        assert_eq!(listing.archived_count, 1);
        assert!(shown(&marks.arrange(&sessions, &view(false, "нет такого"))).is_empty());
    }

    #[test]
    fn marks_round_trip_as_a_map() {
        let mut marks = SessionMarks::default();
        marks.set_pinned(Path::new("/s/a"), true);
        let json = serde_json::to_value(&marks).expect("json");
        assert_eq!(
            json,
            serde_json::json!({"/s/a": {"title": null, "pinned": true, "archived": false}})
        );
        assert_eq!(serde_json::from_value::<SessionMarks>(json).unwrap(), marks);
        assert!(
            serde_json::from_value::<SessionMarks>(serde_json::json!({"/s/a": {"star": true}}))
                .is_err()
        );
    }
}
