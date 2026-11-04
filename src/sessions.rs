//! Marks the user keeps on saved sessions on this device: a custom title,
//! pinning and the archive. The server knows nothing about them; deleting a
//! session goes to the server and only forgets the marks here.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::conversations::ConversationSummary;
use proteus_tabs::ChatKey;
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

/// Marks by canonical driver-neutral conversation id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionMarks(BTreeMap<String, SessionMark>);

// Read the previous preference format once, then persist canonical driver keys.
// Runtime lookup has a single namespace and never guesses a Codex id as a path.
impl<'de> Deserialize<'de> for SessionMarks {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut marks = BTreeMap::<String, SessionMark>::deserialize(deserializer)?;
        let paths: Vec<_> = marks
            .keys()
            .filter(|key| Path::new(key).is_absolute())
            .cloned()
            .collect();
        for path in paths {
            let mark = marks.remove(&path).unwrap();
            marks
                .entry(ChatKey::Proteus(path.into()).tab_id())
                .or_insert(mark);
        }
        Ok(Self(marks))
    }
}

fn key(conversation: &ChatKey) -> String {
    conversation.tab_id()
}

impl SessionMarks {
    pub fn get(&self, dir: &ChatKey) -> SessionMark {
        self.0.get(&key(dir)).cloned().unwrap_or_default()
    }

    fn change(&mut self, dir: &ChatKey, change: impl FnOnce(&mut SessionMark)) {
        let key = key(dir);
        let mark = self.0.entry(key.clone()).or_default();
        change(mark);
        if mark.is_empty() {
            self.0.remove(&key);
        }
    }

    /// Sets a custom title; blank input returns to the automatic one.
    pub fn rename(&mut self, dir: &ChatKey, title: &str) {
        let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = (!title.is_empty()).then(|| title.chars().take(TITLE_LIMIT).collect());
        self.change(dir, |mark| mark.title = title);
    }

    pub fn set_pinned(&mut self, dir: &ChatKey, pinned: bool) {
        self.change(dir, |mark| {
            mark.pinned = pinned;
            if pinned {
                mark.archived = false;
            }
        });
    }

    /// Archiving hides a session from the list and unpins it.
    pub fn set_archived(&mut self, dir: &ChatKey, archived: bool) {
        self.change(dir, |mark| {
            mark.archived = archived;
            if archived {
                mark.pinned = false;
            }
        });
    }

    pub fn forget(&mut self, dir: &ChatKey) {
        self.0.remove(&key(dir));
    }

    /// Title the user gave a session, if any.
    pub fn title(&self, dir: &ChatKey) -> Option<&str> {
        self.0.get(&key(dir))?.title.as_deref()
    }

    /// Build a summary-only forest. Pins and archive membership belong to
    /// each session independently; neither action moves its descendants.
    pub fn arrange<'a>(&self, sessions: &'a [ConversationSummary], view: &View) -> Listing<'a> {
        let search = view.search.trim().to_lowercase();
        let marks: Vec<_> = sessions.iter().map(|s| self.get(&s.key)).collect();
        let by_id: HashMap<_, _> = sessions
            .iter()
            .enumerate()
            .map(|(i, summary)| (&summary.key, i))
            .collect();
        let mut parents: Vec<_> = sessions
            .iter()
            .enumerate()
            .map(|(i, summary)| {
                let parent = *by_id.get(summary.parent.as_ref()?)?;
                // An explicit pin is its own root; archive never drags other
                // sessions with it. Cross-folder children stay with their parent.
                (parent != i && !marks[i].pinned && marks[i].archived == marks[parent].archived)
                    .then_some(parent)
            })
            .collect();
        break_cycles(&mut parents);
        let matches: Vec<_> = sessions
            .iter()
            .zip(&marks)
            .map(|(summary, mark)| search.is_empty() || title_holds(summary, mark, &search))
            .collect();
        let mut included: Vec<_> = matches
            .iter()
            .zip(&marks)
            .map(|(matches, mark)| {
                *matches && (!mark.archived || view.archived || !search.is_empty())
            })
            .collect();
        // Searching a child reveals its ancestry without loading transcripts
        // or including unrelated siblings.
        if !search.is_empty() {
            for i in 0..sessions.len() {
                if !included[i] {
                    continue;
                }
                let mut parent = parents[i];
                while let Some(i) = parent {
                    if included[i] {
                        break;
                    }
                    included[i] = true;
                    parent = parents[i];
                }
            }
        }
        let mut children = vec![Vec::new(); sessions.len()];
        let mut roots = Vec::new();
        for i in 0..sessions.len() {
            if !included[i] {
                continue;
            }
            match parents[i].filter(|parent| included[*parent]) {
                Some(parent) => children[parent].push(i),
                None => roots.push(i),
            }
        }
        let archived_count = marks
            .iter()
            .zip(&matches)
            .filter(|(mark, matches)| mark.archived && **matches)
            .count();
        let mut rows: Vec<Option<Row<'a>>> = sessions
            .iter()
            .zip(marks)
            .map(|(summary, mark)| {
                Some(Row {
                    summary,
                    mark,
                    children: Vec::new(),
                })
            })
            .collect();
        // Iterative postorder also handles unusually deep imported trees.
        let mut order = Vec::new();
        let mut pending = roots.clone();
        while let Some(i) = pending.pop() {
            order.push(i);
            pending.extend(&children[i]);
        }
        for i in order.into_iter().rev() {
            let descendants = children[i]
                .iter()
                .filter_map(|child| rows[*child].take())
                .collect();
            rows[i].as_mut().unwrap().children = descendants;
        }
        let mut listing = Listing {
            archived_count,
            ..Listing::default()
        };
        for root in roots {
            let row = rows[root].take().unwrap();
            if row.mark.archived {
                listing.archived.push(row);
            } else if row.mark.pinned {
                listing.pinned.push(row);
            } else {
                let path = row.summary.workspace_path.as_path();
                if let Some(project) = listing
                    .projects
                    .iter_mut()
                    .find(|project| project.path == path)
                {
                    project.rows.push(row);
                } else {
                    listing.projects.push(Project {
                        path,
                        rows: vec![row],
                    });
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
    pub summary: &'a ConversationSummary,
    pub mark: SessionMark,
    pub children: Vec<Row<'a>>,
}

impl Row<'_> {
    pub fn contains(&self, dir: &ChatKey) -> bool {
        let mut pending = vec![self];
        while let Some(row) = pending.pop() {
            if &row.summary.key == dir {
                return true;
            }
            pending.extend(&row.children);
        }
        false
    }

    fn updated_at_ms(&self) -> u64 {
        let mut latest = 0;
        let mut pending = vec![self];
        while let Some(row) = pending.pop() {
            latest = latest.max(row.summary.updated_at_ms.unwrap_or(0));
            pending.extend(&row.children);
        }
        latest
    }
}

fn title_holds(summary: &ConversationSummary, mark: &SessionMark, search: &str) -> bool {
    mark.title
        .as_deref()
        .or(summary.preview.as_deref())
        .is_some_and(|title| title.to_lowercase().contains(search))
}

/// Each node has at most one parent: a three-colour walk breaks one edge
/// per cycle in linear time and leaves every session accessible.
fn break_cycles(parents: &mut [Option<usize>]) {
    let mut colors = vec![0u8; parents.len()];
    for start in 0..parents.len() {
        if colors[start] != 0 {
            continue;
        }
        let mut path = Vec::new();
        let mut next = Some(start);
        while let Some(i) = next {
            if colors[i] == 1 {
                parents[i] = None;
                break;
            }
            if colors[i] == 2 {
                break;
            }
            colors[i] = 1;
            path.push(i);
            next = parents[i];
        }
        for i in path {
            colors[i] = 2;
        }
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
        self.rows.iter().map(Row::updated_at_ms).max().unwrap_or(0)
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
    use std::path::Path;

    use crate::conversations::ConversationSummary;
    use proteus_tabs::ChatKey;

    use super::{SessionMarks, View};

    fn key(path: &str) -> ChatKey {
        ChatKey::Proteus(path.into())
    }

    fn view(archived: bool, search: &str) -> View<'_> {
        View { archived, search }
    }

    fn summary(dir: &str) -> ConversationSummary {
        changed(dir, "/w", None, None)
    }

    fn changed(
        dir: &str,
        workspace: &str,
        updated_at_ms: Option<u64>,
        preview: Option<&str>,
    ) -> ConversationSummary {
        ConversationSummary {
            key: key(dir),
            proteus_session_id: None,
            parent: None,
            workspace_path: workspace.into(),
            message_count: 1,
            preview: preview.map(str::to_owned),
            updated_at_ms,
            activity: None,
        }
    }

    fn dirs<'r>(rows: impl IntoIterator<Item = &'r super::Row<'r>>) -> Vec<ChatKey> {
        rows.into_iter()
            .map(|row| row.summary.key.clone())
            .collect()
    }

    /// Rows in the order they are shown.
    fn shown(listing: &super::Listing<'_>) -> Vec<ChatKey> {
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
        let dir = &key("/s/a");
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
        marks.set_pinned(&key("/s/c"), true);
        marks.set_archived(&key("/s/b"), true);
        let listing = marks.arrange(&sessions, &view(false, ""));
        assert_eq!(dirs(&listing.pinned), [key("/s/c")]);
        assert_eq!(shown(&listing)[1..], [key("/s/a"), key("/s/d")]);
        assert!(listing.archived.is_empty());
        assert_eq!(listing.archived_count, 1);
        let listing = marks.arrange(&sessions, &view(true, ""));
        assert_eq!(dirs(&listing.archived), [key("/s/b")]);

        // Archiving unpins; pinning brings a session back from the archive.
        marks.set_archived(&key("/s/c"), true);
        assert!(!marks.get(&key("/s/c")).pinned);
        marks.set_pinned(&key("/s/b"), true);
        assert!(!marks.get(&key("/s/b")).archived);
        marks.forget(&key("/s/b"));
        assert_eq!(marks.get(&key("/s/b")), Default::default());
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
            ["/s/b1", "/s/a1", "/s/a2", "/s/c1"].map(key)
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
        marks.rename(&key("/s/c"), "Ночная СБОРКА");
        marks.set_archived(&key("/s/d"), true);
        let listing = marks.arrange(&sessions, &view(false, "  сборк "));
        assert_eq!(shown(&listing), ["/s/a", "/s/c", "/s/d"].map(key));
        assert_eq!(listing.archived_count, 1);
        assert!(shown(&marks.arrange(&sessions, &view(false, "нет такого"))).is_empty());
    }

    fn linked(id: u128, parent: Option<u128>, title: &str) -> ConversationSummary {
        let mut summary = changed(&format!("/s/{id}"), "/w", Some(id as u64), Some(title));
        summary.parent = parent.map(|id| key(&format!("/s/{id}")));
        summary
    }

    #[test]
    fn children_are_nested_recursively_and_search_keeps_only_their_ancestry() {
        let sessions = [
            linked(3, Some(2), "needle"),
            linked(4, Some(1), "sibling"),
            linked(2, Some(1), "worker"),
            linked(1, None, "main"),
        ];
        let marks = SessionMarks::default();
        let listing = marks.arrange(&sessions, &view(false, ""));
        let roots = &listing.projects[0].rows;
        assert_eq!(dirs(roots), [key("/s/1")]);
        assert_eq!(dirs(&roots[0].children), ["/s/4", "/s/2"].map(key));
        assert_eq!(dirs(&roots[0].children[1].children), [key("/s/3")]);
        assert!(roots[0].contains(&key("/s/3")));
        let listing = marks.arrange(&sessions, &view(false, "needle"));
        let root = &listing.projects[0].rows[0];
        assert_eq!(dirs(&root.children), [key("/s/2")]);
        assert_eq!(dirs(&root.children[0].children), [key("/s/3")]);
    }

    #[test]
    fn missing_parents_and_cycles_do_not_hide_sessions() {
        let sessions = [
            linked(1, Some(99), "orphan"),
            linked(2, Some(3), "cycle a"),
            linked(3, Some(2), "cycle b"),
            linked(4, Some(4), "self"),
        ];
        let listing = SessionMarks::default().arrange(&sessions, &view(false, ""));
        let roots = &listing.projects[0].rows;
        assert_eq!(roots.len(), 3);
        for id in 1..=4 {
            assert!(
                roots
                    .iter()
                    .any(|root| root.contains(&key(&format!("/s/{id}"))))
            );
        }
    }

    #[test]
    fn pin_and_archive_do_not_cascade_to_descendants() {
        let sessions = [
            linked(1, None, "main"),
            linked(2, Some(1), "worker"),
            linked(3, Some(2), "nested"),
        ];
        let mut marks = SessionMarks::default();
        marks.set_pinned(&key("/s/1"), true);
        let listing = marks.arrange(&sessions, &view(false, ""));
        assert_eq!(listing.pinned.len(), 1);
        assert!(listing.pinned[0].contains(&key("/s/3")));
        marks.set_pinned(&key("/s/2"), true);
        let listing = marks.arrange(&sessions, &view(false, ""));
        assert_eq!(listing.pinned.len(), 2);
        assert!(listing.pinned[0].children.is_empty());
        assert!(listing.pinned[1].contains(&key("/s/3")));
        marks.set_archived(&key("/s/2"), true);
        let listing = marks.arrange(&sessions, &view(true, ""));
        assert_eq!(dirs(&listing.archived), [key("/s/2")]);
        assert_eq!(dirs(&listing.projects[0].rows), [key("/s/3")]);
        assert_eq!(listing.archived_count, 1);
    }

    #[test]
    fn marks_round_trip_as_a_map() {
        let mut marks = SessionMarks::default();
        marks.set_pinned(&key("/s/a"), true);
        let json = serde_json::to_value(&marks).expect("json");
        assert_eq!(
            json,
            serde_json::json!({"chat:/s/a": {"title": null, "pinned": true, "archived": false}})
        );
        assert_eq!(serde_json::from_value::<SessionMarks>(json).unwrap(), marks);
        assert!(
            serde_json::from_value::<SessionMarks>(
                serde_json::json!({"chat:/s/a": {"star": true}})
            )
            .is_err()
        );
    }

    #[test]
    fn native_driver_keys_do_not_collide_and_codex_children_use_the_same_forest() {
        let proteus = summary("/s/shared");
        let codex = ConversationSummary {
            key: ChatKey::Codex("/s/shared".into()),
            preview: Some("Codex parent".into()),
            ..summary("/unused")
        };
        let child = ConversationSummary {
            key: ChatKey::Codex("child".into()),
            parent: Some(codex.key.clone()),
            preview: Some("needle".into()),
            ..summary("/unused")
        };
        let sessions = [proteus, codex, child];
        let mut marks = SessionMarks::default();
        marks.rename(&sessions[0].key, "Core title");
        marks.rename(&sessions[1].key, "Codex title");
        assert_eq!(marks.title(&sessions[0].key), Some("Core title"));
        assert_eq!(marks.title(&sessions[1].key), Some("Codex title"));
        let listing = marks.arrange(&sessions, &view(false, "needle"));
        let roots = &listing.projects[0].rows;
        assert_eq!(dirs(roots), [sessions[1].key.clone()]);
        assert_eq!(dirs(&roots[0].children), [sessions[2].key.clone()]);
        let json = serde_json::to_value(&marks).unwrap();
        assert!(json.get("chat:/s/shared").is_some());
        assert!(json.get("codex:/s/shared").is_some());
    }
}

#[cfg(test)]
mod preference_upgrade {
    use super::*;
    #[test]
    fn old_titles_pins_and_archive_are_preserved_in_canonical_namespace() {
        let marks: SessionMarks = serde_json::from_str(r#"{"/sessions/a":{"title":"Saved","pinned":true},"/sessions/b":{"archived":true},"codex:opaque":{"title":"Codex"}}"#).unwrap();
        assert_eq!(
            marks.title(&ChatKey::Proteus("/sessions/a".into())),
            Some("Saved")
        );
        assert!(marks.get(&ChatKey::Proteus("/sessions/a".into())).pinned);
        assert!(marks.get(&ChatKey::Proteus("/sessions/b".into())).archived);
        assert_eq!(marks.title(&ChatKey::Codex("opaque".into())), Some("Codex"));
        let saved = serde_json::to_value(&marks).unwrap();
        assert!(saved.get("/sessions/a").is_none());
        assert!(saved.get("chat:/sessions/a").is_some());
    }
}
