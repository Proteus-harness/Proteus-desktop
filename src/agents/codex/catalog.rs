//! Summary pagination never resumes a thread or reads its message history.
use std::collections::HashSet;
use std::path::Path;

use super::{CodexService, Error, Summary};

#[derive(Default)]
pub(crate) struct CatalogCursor {
    loaded: bool,
    next: Option<String>,
    seen: HashSet<String>,
}

impl CatalogCursor {
    pub(crate) fn request(&self, refresh: bool) -> Option<Option<String>> {
        if refresh || !self.loaded {
            Some(None)
        } else {
            self.next.clone().map(Some)
        }
    }

    pub(crate) fn has_more(&self) -> bool {
        !self.loaded || self.next.is_some()
    }

    pub(crate) fn accept(
        &mut self,
        requested: Option<&str>,
        next: Option<String>,
    ) -> Result<(), Error> {
        if requested.is_none() {
            self.seen.clear();
        }
        if let Some(requested) = requested {
            self.seen.insert(requested.to_owned());
        }
        if next.as_ref().is_some_and(|next| self.seen.contains(next)) {
            self.next = None;
            self.loaded = true;
            return Err(Error::Protocol(
                "Codex thread catalog repeated a cursor".into(),
            ));
        }
        self.next = next;
        self.loaded = true;
        Ok(())
    }
}

impl CodexService {
    /// A project's newest main thread may be behind pages of newer children.
    /// Stop at the first page with a main thread; never load thread histories.
    pub(crate) fn list_until_main(&self, cwd: &Path) -> Result<Vec<Summary>, Error> {
        let mut cursor = CatalogCursor::default();
        let mut summaries = Vec::new();
        while let Some(requested) = cursor.request(false) {
            let page = self.list(cwd, requested.as_deref())?;
            cursor.accept(requested.as_deref(), page.next_cursor)?;
            let main = page
                .data
                .iter()
                .any(|summary| summary.parent_thread_id.is_none());
            summaries.extend(page.data);
            if main {
                break;
            }
        }
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_keep_opaque_cursor_and_reject_cycles_without_repeating_requests() {
        let mut cursor = CatalogCursor::default();
        assert_eq!(cursor.request(false), Some(None));
        cursor.accept(None, Some("after:50".into())).unwrap();
        assert_eq!(cursor.request(false), Some(Some("after:50".into())));
        cursor
            .accept(Some("after:50"), Some("after:100".into()))
            .unwrap();
        assert!(
            cursor
                .accept(Some("after:100"), Some("after:50".into()))
                .is_err()
        );
        assert!(!cursor.has_more());
        assert_eq!(cursor.request(false), None);
        assert_eq!(cursor.request(true), Some(None));
        cursor.accept(None, None).unwrap();
        assert_eq!(cursor.request(false), None);
    }
}
