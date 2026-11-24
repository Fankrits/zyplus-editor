//! Undo/redo history shared by the plain and rich editors.
//!
//! Each step stores the whole text before the edit, so the cost of a step is the
//! file size. Capping only the number of steps lets a large file hold hundreds of
//! megabytes, so the history is bounded by bytes as well.

use std::collections::VecDeque;
use std::ops::Range;

/// The text and selection to restore.
pub type Snapshot = (String, Range<usize>);

/// Most steps kept, however small the file.
pub const MAX_STEPS: usize = 250;
/// Most text kept across the undo and redo stacks. The newest undo step is always
/// kept, so one undo works even when a single snapshot is larger than this.
pub const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Default)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    bytes: usize,
}

impl History {
    /// Records the state before a new edit. A new edit drops everything that could be redone.
    pub fn record(&mut self, snapshot: Snapshot) {
        self.drop_redo();
        self.push_undo(snapshot);
        self.trim();
    }

    /// Steps back: returns the snapshot to restore, keeping `current` for redo.
    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo.pop_back()?;
        self.bytes -= previous.0.len();
        self.bytes += current.0.len();
        self.redo.push(current);
        self.trim();
        Some(previous)
    }

    /// Steps forward: returns the snapshot to restore, keeping `current` for undo.
    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo.pop()?;
        self.bytes -= next.0.len();
        self.push_undo(current);
        self.trim();
        Some(next)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bytes = 0;
    }

    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    #[cfg(test)]
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    fn push_undo(&mut self, snapshot: Snapshot) {
        self.bytes += snapshot.0.len();
        self.undo.push_back(snapshot);
    }

    fn drop_redo(&mut self) {
        let dropped: usize = self.redo.iter().map(|s| s.0.len()).sum();
        self.bytes -= dropped;
        self.redo.clear();
    }

    /// Drops the oldest undo steps while over a cap. The newest step stays.
    fn trim(&mut self) {
        while self.undo.len() > 1 && (self.undo.len() > MAX_STEPS || self.bytes > MAX_BYTES) {
            if let Some(oldest) = self.undo.pop_front() {
                self.bytes -= oldest.0.len();
            }
        }
        // Redo steps are the newest history of an undone run; drop the deepest
        // (oldest-undone) ones if undoing alone filled the budget.
        while self.bytes > MAX_BYTES && self.redo.len() > 1 {
            let dropped = self.redo.remove(0);
            self.bytes -= dropped.0.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(len: usize) -> Snapshot {
        ("x".repeat(len), 0..0)
    }

    #[test]
    fn step_cap_drops_the_oldest() {
        let mut h = History::default();
        for _ in 0..(MAX_STEPS + 10) {
            h.record(snap(1));
        }
        assert_eq!(h.undo_len(), MAX_STEPS);
        assert_eq!(h.bytes(), MAX_STEPS);
    }

    #[test]
    fn byte_cap_drops_the_oldest_but_keeps_the_newest_step() {
        let mut h = History::default();
        for _ in 0..100 {
            h.record(snap(MAX_BYTES / 4));
        }
        assert!(h.bytes() <= MAX_BYTES, "{}", h.bytes());
        assert!((1..=4).contains(&h.undo_len()), "{}", h.undo_len());
    }

    #[test]
    fn a_snapshot_over_the_budget_still_allows_one_undo() {
        let mut h = History::default();
        h.record(snap(MAX_BYTES + 1));
        assert_eq!(h.undo_len(), 1);
        assert!(h.undo(snap(2)).is_some());
    }

    #[test]
    fn undo_then_redo_round_trips_and_a_new_edit_clears_redo() {
        let mut h = History::default();
        h.record(("a".into(), 0..1));
        let back = h.undo(("b".into(), 0..1)).unwrap();
        assert_eq!(back.0, "a");
        assert_eq!((h.undo_len(), h.redo_len()), (0, 1));
        let forward = h.redo(("a".into(), 0..1)).unwrap();
        assert_eq!(forward.0, "b");
        assert_eq!((h.undo_len(), h.redo_len()), (1, 0));
        h.undo(("b".into(), 0..0)).unwrap();
        h.record(("c".into(), 0..0));
        assert!(h.redo(("d".into(), 0..0)).is_none());
    }

    #[test]
    fn the_byte_count_matches_what_is_stored_through_every_operation() {
        let mut h = History::default();
        let stored = |h: &History| {
            h.undo.iter().map(|s| s.0.len()).sum::<usize>()
                + h.redo.iter().map(|s| s.0.len()).sum::<usize>()
        };
        for i in 1..30 {
            h.record(snap(i * 3));
            assert_eq!(h.bytes(), stored(&h));
        }
        for _ in 0..10 {
            h.undo(snap(7)).unwrap();
            assert_eq!(h.bytes(), stored(&h));
        }
        for _ in 0..4 {
            h.redo(snap(5)).unwrap();
            assert_eq!(h.bytes(), stored(&h));
        }
        h.record(snap(9));
        assert_eq!(h.bytes(), stored(&h));
        h.clear();
        assert_eq!((h.bytes(), h.undo_len(), h.redo_len()), (0, 0, 0));
    }

    #[test]
    fn undoing_a_huge_file_does_not_blow_the_budget_with_redo_snapshots() {
        let mut h = History::default();
        for _ in 0..8 {
            h.record(snap(MAX_BYTES / 3));
        }
        for _ in 0..6 {
            let _ = h.undo(snap(MAX_BYTES / 3));
        }
        assert!(h.bytes() <= MAX_BYTES + MAX_BYTES / 3, "{}", h.bytes());
    }
}
