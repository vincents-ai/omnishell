//! Undo/redo stack for OmniShell.
//!
//! Tracks session commands and their effects, enabling undo/redo of mutating operations.
//! Supports:
//! - Session-level undo (revert to pre-execution snapshot)
//! - Persistent undo history (via git reflog)
//! - Vim-style undo clear (`:undo clear`)
//! - File-level restore (restore individual files from snapshots)

use crate::snapshot::Snapshot;

/// A single undo record.
#[derive(Debug, Clone)]
pub struct UndoRecord {
    /// The pre-execution snapshot (the state to restore to).
    pub pre_snapshot: Snapshot,
    /// The post-execution snapshot (the state we're undoing from).
    pub post_snapshot: Option<Snapshot>,
    /// The command that was executed.
    pub command: String,
    /// Whether this record has been undone.
    pub undone: bool,
}

/// The undo/redo stack.
pub struct UndoStack {
    /// Undo history (most recent at the end).
    records: Vec<UndoRecord>,
    /// Current position in the undo stack (for redo).
    /// Points to the most recent record that hasn't been undone.
    position: usize,
}

impl UndoStack {
    /// Create a new empty undo stack.
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            position: 0,
        }
    }

    /// Record a command execution with its pre and post snapshots.
    pub fn record(&mut self, pre: Snapshot, post: Option<Snapshot>, command: String) {
        // If we've undone some things and now record a new command,
        // truncate the redo history (like vim)
        if self.position < self.records.len() {
            self.records.truncate(self.position);
        }

        self.records.push(UndoRecord {
            pre_snapshot: pre,
            post_snapshot: post,
            command,
            undone: false,
        });
        self.position = self.records.len();
    }

    /// Undo the most recent command. Returns the pre-execution snapshot to restore.
    pub fn undo(&mut self) -> Option<&Snapshot> {
        if self.position == 0 {
            return None;
        }

        self.position -= 1;
        let record = &mut self.records[self.position];
        record.undone = true;

        Some(&record.pre_snapshot)
    }

    /// Redo the most recently undone command. Returns the post-execution snapshot.
    pub fn redo(&mut self) -> Option<&Snapshot> {
        if self.position >= self.records.len() {
            return None;
        }

        let record = &mut self.records[self.position];
        record.undone = false;
        self.position += 1;

        record.post_snapshot.as_ref()
    }

    /// Get the number of undo-able actions.
    pub fn undo_count(&self) -> usize {
        self.position
    }

    /// Get the number of redo-able actions.
    pub fn redo_count(&self) -> usize {
        self.records.len() - self.position
    }

    /// Clear the undo history (vim-style `:undo clear`).
    pub fn clear(&mut self) {
        self.records.clear();
        self.position = 0;
    }

    /// Get the full undo history.
    pub fn history(&self) -> &[UndoRecord] {
        &self.records
    }

    /// Get the most recent record (regardless of undo state).
    pub fn last_record(&self) -> Option<&UndoRecord> {
        self.records.last()
    }

    /// Get records for a specific command.
    pub fn records_for_command(&self, command: &str) -> Vec<&UndoRecord> {
        self.records
            .iter()
            .filter(|r| r.command == command)
            .collect()
    }
}

impl Default for UndoStack {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::SnapshotPhase;

    fn make_snapshot(phase: SnapshotPhase, command: &str) -> Snapshot {
        let is_post = phase == SnapshotPhase::PostExecution;
        Snapshot {
            commit_id: None,
            timestamp: 0,
            trigger_command: command.to_string(),
            phase,
            exit_code: if is_post { Some(0) } else { None },
        }
    }

    #[test]
    fn test_undo_redo_basic() {
        let mut stack = UndoStack::new();

        // Record two commands
        let pre1 = make_snapshot(SnapshotPhase::PreExecution, "rm a.txt");
        let post1 = make_snapshot(SnapshotPhase::PostExecution, "rm a.txt");
        stack.record(pre1, Some(post1), "rm a.txt".to_string());

        let pre2 = make_snapshot(SnapshotPhase::PreExecution, "rm b.txt");
        let post2 = make_snapshot(SnapshotPhase::PostExecution, "rm b.txt");
        stack.record(pre2, Some(post2), "rm b.txt".to_string());

        assert_eq!(stack.undo_count(), 2);
        assert_eq!(stack.redo_count(), 0);

        // Undo once (undoes rm b.txt)
        let snap = stack.undo().unwrap();
        assert_eq!(snap.trigger_command, "rm b.txt");
        assert_eq!(stack.undo_count(), 1);
        assert_eq!(stack.redo_count(), 1);

        // Redo
        let snap = stack.redo().unwrap();
        assert_eq!(snap.trigger_command, "rm b.txt");
        assert_eq!(stack.undo_count(), 2);
        assert_eq!(stack.redo_count(), 0);
    }

    #[test]
    fn test_undo_truncates_redo() {
        let mut stack = UndoStack::new();

        let pre = make_snapshot(SnapshotPhase::PreExecution, "rm a.txt");
        let post = make_snapshot(SnapshotPhase::PostExecution, "rm a.txt");
        stack.record(pre, Some(post), "rm a.txt".to_string());

        // Undo
        stack.undo();

        // New command (should truncate redo history, like vim)
        let pre2 = make_snapshot(SnapshotPhase::PreExecution, "rm c.txt");
        let post2 = make_snapshot(SnapshotPhase::PostExecution, "rm c.txt");
        stack.record(pre2, Some(post2), "rm c.txt".to_string());

        assert_eq!(stack.undo_count(), 1);
        assert_eq!(stack.redo_count(), 0);
    }

    #[test]
    fn test_undo_clear() {
        let mut stack = UndoStack::new();

        let pre = make_snapshot(SnapshotPhase::PreExecution, "rm a.txt");
        let post = make_snapshot(SnapshotPhase::PostExecution, "rm a.txt");
        stack.record(pre, Some(post), "rm a.txt".to_string());

        stack.clear();
        assert_eq!(stack.undo_count(), 0);
        assert_eq!(stack.redo_count(), 0);
        assert!(stack.history().is_empty());
    }

    #[test]
    fn test_empty_undo() {
        let mut stack = UndoStack::new();
        assert!(stack.undo().is_none());
        assert!(stack.redo().is_none());
    }

    #[test]
    fn test_records_for_command() {
        let mut stack = UndoStack::new();

        for _ in 0..3 {
            let pre = make_snapshot(SnapshotPhase::PreExecution, "cargo build");
            let post = make_snapshot(SnapshotPhase::PostExecution, "cargo build");
            stack.record(pre, Some(post), "cargo build".to_string());
        }

        let pre = make_snapshot(SnapshotPhase::PreExecution, "ls");
        let post = make_snapshot(SnapshotPhase::PostExecution, "ls");
        stack.record(pre, Some(post), "ls".to_string());

        assert_eq!(stack.records_for_command("cargo build").len(), 3);
        assert_eq!(stack.records_for_command("ls").len(), 1);
        assert_eq!(stack.records_for_command("rm").len(), 0);
    }

    #[test]
    fn test_double_undo() {
        let mut stack = UndoStack::new();

        let pre1 = make_snapshot(SnapshotPhase::PreExecution, "rm a.txt");
        let post1 = make_snapshot(SnapshotPhase::PostExecution, "rm a.txt");
        stack.record(pre1, Some(post1), "rm a.txt".to_string());

        let pre2 = make_snapshot(SnapshotPhase::PreExecution, "rm b.txt");
        let post2 = make_snapshot(SnapshotPhase::PostExecution, "rm b.txt");
        stack.record(pre2, Some(post2), "rm b.txt".to_string());

        // Undo twice
        assert!(stack.undo().is_some());
        assert!(stack.undo().is_some());
        assert!(stack.undo().is_none()); // Nothing left to undo
        assert_eq!(stack.redo_count(), 2);
    }
}
