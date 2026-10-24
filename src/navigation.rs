//! Bounded, headless navigation history shared by tabs and Settings pages.

/// Maximum number of navigation targets retained for one window.
pub(crate) const HISTORY_LIMIT: usize = 128;

#[derive(Clone, Debug)]
pub(crate) struct History<T: Clone + PartialEq> {
    entries: Vec<T>,
    cursor: Option<usize>,
}

impl<T: Clone + PartialEq> Default for History<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            cursor: None,
        }
    }
}

impl<T: Clone + PartialEq> History<T> {
    /// Record a target. Visiting the current target is a no-op; other visits
    /// branch from the cursor and discard the old forward path.
    pub(crate) fn visit(&mut self, target: T) -> bool {
        if self.cursor.and_then(|cursor| self.entries.get(cursor)) == Some(&target) {
            return false;
        }

        if let Some(cursor) = self.cursor {
            self.entries.truncate(cursor + 1);
        }
        self.entries.push(target);
        if self.entries.len() > HISTORY_LIMIT {
            self.entries.remove(0);
        }
        self.cursor = Some(self.entries.len() - 1);
        true
    }

    /// Move to the nearest available target before the current one.
    /// Unavailable targets are skipped. If none is available, the cursor is
    /// unchanged.
    pub(crate) fn back(&mut self, mut available: impl FnMut(&T) -> bool) -> Option<T> {
        let cursor = self.cursor?;
        for index in (0..cursor).rev() {
            if available(&self.entries[index]) {
                self.cursor = Some(index);
                return Some(self.entries[index].clone());
            }
        }
        None
    }

    /// Move to the nearest available target after the current one.
    /// Unavailable targets are skipped. If none is available, the cursor is
    /// unchanged.
    pub(crate) fn forward(&mut self, mut available: impl FnMut(&T) -> bool) -> Option<T> {
        let cursor = self.cursor?;
        for index in cursor + 1..self.entries.len() {
            if available(&self.entries[index]) {
                self.cursor = Some(index);
                return Some(self.entries[index].clone());
            }
        }
        None
    }

    #[cfg(test)]
    fn current(&self) -> Option<&T> {
        self.cursor.and_then(|cursor| self.entries.get(cursor))
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{HISTORY_LIMIT, History};

    #[test]
    fn traverses_back_back_forward() {
        let mut history = History::default();
        for target in ['A', 'B', 'C'] {
            history.visit(target);
        }

        assert_eq!(history.back(|_| true), Some('B'));
        assert_eq!(history.back(|_| true), Some('A'));
        assert_eq!(history.forward(|_| true), Some('B'));
        assert_eq!(history.current(), Some(&'B'));
    }

    #[test]
    fn a_visit_after_back_branches_and_discards_forward_history() {
        let mut history = History::default();
        for target in ['A', 'B', 'C'] {
            history.visit(target);
        }

        assert_eq!(history.back(|_| true), Some('B'));
        assert!(history.visit('D'));
        assert_eq!(history.forward(|_| true), None);
        assert_eq!(history.back(|_| true), Some('B'));
    }

    #[test]
    fn visiting_current_target_keeps_forward_history() {
        let mut history = History::default();
        for target in ['A', 'B', 'C'] {
            history.visit(target);
        }

        assert_eq!(history.back(|_| true), Some('B'));
        assert!(!history.visit('B'));
        assert_eq!(history.forward(|_| true), Some('C'));
    }

    #[test]
    fn unavailable_targets_are_skipped_and_failed_traversal_keeps_cursor() {
        let mut history = History::default();
        for target in ['A', 'B', 'C', 'D'] {
            history.visit(target);
        }

        assert_eq!(history.back(|target| *target != 'C'), Some('B'));
        assert_eq!(history.forward(|target| *target != 'C'), Some('D'));
        assert_eq!(history.current(), Some(&'D'));
        assert_eq!(history.back(|_| false), None);
        assert_eq!(history.current(), Some(&'D'));
    }

    #[test]
    fn history_does_not_exceed_its_capacity() {
        let mut history = History::default();
        for target in 0..HISTORY_LIMIT + 2 {
            history.visit(target);
        }

        assert_eq!(history.len(), HISTORY_LIMIT);
        assert_eq!(history.current(), Some(&(HISTORY_LIMIT + 1)));
        for _ in 1..HISTORY_LIMIT {
            assert!(history.back(|_| true).is_some());
        }
        assert_eq!(history.current(), Some(&2));
        assert_eq!(history.back(|_| true), None);
    }
}
