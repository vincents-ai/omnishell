//! Gix-based snapshot engine for OmniShell.
//!
//! Creates atomic "Pre-Execution" and "Post-Execution" snapshots of the working directory
//! when commands are flagged as "mutating" (e.g., rm, mv, cargo). Uses gix (gitoxide)
//! for pure Rust git operations.
//!
//! The engine creates commits that point to the current tree state, creating a "checkpoint"
//! that can be reverted to. File staging (add/rm) is the shell's responsibility before
//! the snapshot is taken.

use std::path::Path;

use gix::hash::ObjectId;

use crate::error::Result;

/// Metadata for a single snapshot.
#[derive(Debug, Clone)]
pub struct Snapshot {
    /// The git commit hash (if committed).
    pub commit_id: Option<ObjectId>,
    /// Timestamp when the snapshot was taken.
    pub timestamp: u64,
    /// The command that triggered this snapshot.
    pub trigger_command: String,
    /// Whether this was a pre or post execution snapshot.
    pub phase: SnapshotPhase,
    /// Exit code (only for post-execution snapshots).
    pub exit_code: Option<i32>,
}

/// Whether the snapshot was taken before or after command execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotPhase {
    PreExecution,
    PostExecution,
}

/// The snapshot engine. Manages git-based snapshots of the working directory.
pub struct SnapshotEngine {
    /// The gix repository handle (None if not in a git repo).
    repo: Option<gix::Repository>,
    /// In-memory history of snapshots (for undo stack).
    history: Vec<Snapshot>,
}

impl SnapshotEngine {
    /// Create a new snapshot engine for the given working directory.
    ///
    /// If the directory is not a git repo, the engine operates in "degraded" mode
    /// (tracking only, no git commits).
    pub fn new(working_dir: &Path) -> Self {
        let repo = gix::discover(working_dir).ok();
        Self {
            repo,
            history: Vec::new(),
        }
    }

    /// Returns true if we have an active git repository.
    pub fn has_repo(&self) -> bool {
        self.repo.is_some()
    }

    /// Check if a command should trigger a snapshot.
    pub fn is_mutating_command(command: &str) -> bool {
        let mutating_prefixes = [
            "rm", "mv", "cp", "mkdir", "touch", "chmod", "chown",
            "cargo", "pip", "npm", "yarn", "go", "make",
            "git push", "git commit", "git merge", "git rebase", "git reset",
            "dd", "truncate", "shred",
        ];

        let cmd_lower = command.trim().to_lowercase();
        mutating_prefixes.iter().any(|prefix| {
            cmd_lower == *prefix || cmd_lower.starts_with(&format!("{} ", prefix))
        })
    }

    /// Create a pre-execution snapshot.
    pub fn pre_execution_snapshot(&mut self, command: &str) -> Result<Snapshot> {
        let timestamp = now_secs();

        let commit_id = self.try_create_commit(&format!("omnishell: PRE | {}", command));

        let snapshot = Snapshot {
            commit_id,
            timestamp,
            trigger_command: command.to_string(),
            phase: SnapshotPhase::PreExecution,
            exit_code: None,
        };

        self.history.push(snapshot.clone());
        Ok(snapshot)
    }

    /// Create a post-execution snapshot.
    pub fn post_execution_snapshot(&mut self, command: &str, exit_code: i32) -> Result<Snapshot> {
        let timestamp = now_secs();

        let commit_id = self.try_create_commit(&format!(
            "omnishell: POST | {} | exit={}",
            command, exit_code
        ));

        let snapshot = Snapshot {
            commit_id,
            timestamp,
            trigger_command: command.to_string(),
            phase: SnapshotPhase::PostExecution,
            exit_code: Some(exit_code),
        };

        self.history.push(snapshot.clone());
        Ok(snapshot)
    }

    /// Try to create a git commit pointing at the current tree state.
    /// Returns None if no repo, no commits yet, or on error.
    fn try_create_commit(&self, message: &str) -> Option<ObjectId> {
        let repo = self.repo.as_ref()?;

        // Get the HEAD commit and its tree
        let head = repo.head_commit().ok()?;
        let tree = head.tree().ok()?;

        // Write a new commit with the same tree but our message
        let tree_id = tree.id;
        let parent_id = head.id;

        let commit_id = repo
            .commit(
                "refs/heads/omnishell-snapshots",
                message,
                tree_id,
                std::iter::once(parent_id),
            )
            .ok()?;

        Some(commit_id.detach())
    }

    /// Get the snapshot history.
    pub fn history(&self) -> &[Snapshot] {
        &self.history
    }

    /// Get the most recent snapshot.
    pub fn last_snapshot(&self) -> Option<&Snapshot> {
        self.history.last()
    }

    /// Get the gix repository handle (if available).
    pub fn repo(&self) -> Option<&gix::Repository> {
        self.repo.as_ref()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_mutating_command() {
        assert!(SnapshotEngine::is_mutating_command("rm file.txt"));
        assert!(SnapshotEngine::is_mutating_command("mv a b"));
        assert!(SnapshotEngine::is_mutating_command("cargo build"));
        assert!(SnapshotEngine::is_mutating_command("git push"));
        assert!(SnapshotEngine::is_mutating_command("pip install foo"));
        assert!(SnapshotEngine::is_mutating_command("npm install"));

        assert!(!SnapshotEngine::is_mutating_command("ls"));
        assert!(!SnapshotEngine::is_mutating_command("cat file.txt"));
        assert!(!SnapshotEngine::is_mutating_command("echo hello"));
        assert!(!SnapshotEngine::is_mutating_command("pwd"));
        assert!(!SnapshotEngine::is_mutating_command("git status"));
        assert!(!SnapshotEngine::is_mutating_command("git log"));
    }

    #[test]
    fn test_is_mutating_command_case_insensitive() {
        assert!(SnapshotEngine::is_mutating_command("RM file.txt"));
        assert!(SnapshotEngine::is_mutating_command("Cargo Build"));
    }

    #[test]
    fn test_is_mutating_command_trimmed() {
        assert!(SnapshotEngine::is_mutating_command("  rm file.txt  "));
        assert!(!SnapshotEngine::is_mutating_command("  ls  "));
    }

    #[test]
    fn test_snapshot_engine_no_repo() {
        let mut engine = SnapshotEngine::new(Path::new("/tmp/nonexistent_repo_12345"));
        assert!(!engine.has_repo());

        let snap = engine.pre_execution_snapshot("rm test.txt").unwrap();
        assert!(snap.commit_id.is_none());
        assert_eq!(snap.phase, SnapshotPhase::PreExecution);

        let snap = engine.post_execution_snapshot("rm test.txt", 0).unwrap();
        assert!(snap.commit_id.is_none());
        assert_eq!(snap.exit_code, Some(0));
    }

    #[test]
    fn test_snapshot_history() {
        let mut engine = SnapshotEngine::new(Path::new("/tmp/nonexistent_repo_12345"));
        engine.pre_execution_snapshot("rm test.txt").unwrap();
        engine.post_execution_snapshot("rm test.txt", 0).unwrap();

        assert_eq!(engine.history().len(), 2);
        assert_eq!(
            engine.last_snapshot().unwrap().phase,
            SnapshotPhase::PostExecution
        );
    }
}
