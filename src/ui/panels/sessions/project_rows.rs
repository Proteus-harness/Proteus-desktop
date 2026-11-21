//! Present project roots without materializing a second list of visible rows.
use crate::sessions::Row;
use proteus_tabs::ChatKey;

/// Chats a project folder shows before "show more".
pub(super) const PROJECT_ROWS: usize = 5;

/// Visit the main block in its original order. The folded counter does not
/// need row references; only the in-flight reveal needs the extra rows.
pub(super) fn visit<'r, 's>(
    rows: &'r [Row<'s>],
    current: Option<&ChatKey>,
    all: bool,
    more_moving: bool,
    mut show: impl FnMut(&'r Row<'s>),
) -> (usize, Vec<&'r Row<'s>>) {
    let mut hidden = 0;
    let mut extra = Vec::new();
    if all && !more_moving {
        rows.iter().for_each(show);
        return (hidden, extra);
    }
    for (index, row) in rows.iter().enumerate() {
        if index < PROJECT_ROWS || current.is_some_and(|key| row.contains(key)) {
            show(row);
        } else {
            if !all {
                hidden += 1;
            }
            if more_moving {
                extra.push(row);
            }
        }
    }
    (hidden, extra)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversations::ConversationSummary;
    use crate::sessions::{SessionMarks, View};

    #[test]
    fn active_nested_root_precedes_animation_extras() {
        let key = |i| ChatKey::Codex(format!("c{i}"));
        let summaries: Vec<_> = (0..9)
            .map(|i| ConversationSummary {
                key: key(i),
                proteus_session_id: None,
                parent: (i == 8).then(|| key(7)),
                workspace_path: "/synthetic/project".into(),
                message_count: 0,
                updated_at_ms: Some(100 - i),
                preview: None,
                activity: None,
            })
            .collect();
        let listing = SessionMarks::default().arrange(
            &summaries,
            &View {
                archived: false,
                search: "",
            },
        );
        let roots = &listing.projects[0].rows;
        let mut shown = Vec::new();
        let (hidden, extra) = visit(roots, Some(&key(8)), false, true, |row| {
            shown.push(row.summary.key.clone());
        });
        assert_eq!(shown, [0, 1, 2, 3, 4, 7].map(key));
        assert_eq!(hidden, 2);
        assert_eq!(
            extra.iter().map(|r| &r.summary.key).collect::<Vec<_>>(),
            [&key(5), &key(6)]
        );
    }
}
