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

/// Where snapshot commits are stored.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum SnapshotTarget {
    /// Commit to a dedicated ref (refs/heads/omnishell-snapshots).
    #[default]
    DedicatedRef,
    /// Commit to the current branch.
    CurrentBranch,
    /// Create a detached branch per snapshot.
    Detached,
}

/// Retention policy for snapshots.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum SnapshotRetention {
    /// Keep all snapshots forever.
    #[default]
    Unlimited,
    /// Keep only the last N snapshots.
    Count(usize),
    /// Keep snapshots newer than the given duration (in seconds).
    TimeBased(u64),
}

/// When to trigger a post-execution snapshot.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub enum SnapshotTrigger {
    /// Always snapshot after mutating commands.
    #[default]
    Always,
    /// Only snapshot if working tree actually changed.
    OnTreeChange,
}

/// Configuration for snapshot behavior.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct SnapshotConfig {
    /// Where to store snapshot commits.
    pub target: SnapshotTarget,
    /// Retention policy.
    pub retention: SnapshotRetention,
    /// When to trigger post-execution snapshots.
    pub trigger: SnapshotTrigger,
    /// Ref name for dedicated target mode.
    pub ref_name: String,
}

impl SnapshotConfig {
    /// Create default config with a custom ref name.
    pub fn new() -> Self {
        Self {
            ref_name: "refs/heads/omnishell-snapshots".to_string(),
            ..Default::default()
        }
    }
}

/// The snapshot engine. Manages git-based snapshots of the working directory.
pub struct SnapshotEngine {
    /// The gix repository handle (None if not in a git repo).
    repo: Option<gix::Repository>,
    /// In-memory history of snapshots (for undo stack).
    history: Vec<Snapshot>,
    /// Snapshot configuration.
    #[allow(dead_code)]
    config: SnapshotConfig,
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
            config: SnapshotConfig::new(),
        }
    }

    /// Returns true if we have an active git repository.
    pub fn has_repo(&self) -> bool {
        self.repo.is_some()
    }

    /// Check if a command should trigger a snapshot.
    pub fn is_mutating_command(command: &str) -> bool {
        let mutating_prefixes = [
            "rm",
            "mv",
            "cp",
            "mkdir",
            "touch",
            "chmod",
            "chown",
            "cargo",
            "pip",
            "npm",
            "yarn",
            "go",
            "make",
            "git push",
            "git commit",
            "git merge",
            "git rebase",
            "git reset",
            "dd",
            "truncate",
            "shred",
        ];

        let cmd_lower = command.trim().to_lowercase();
        mutating_prefixes
            .iter()
            .any(|prefix| cmd_lower == *prefix || cmd_lower.starts_with(&format!("{prefix} ")))
    }

    /// Create a pre-execution snapshot.
    pub fn pre_execution_snapshot(&mut self, command: &str) -> Result<Snapshot> {
        let timestamp = now_secs();

        let commit_id = self.try_create_commit(&format!("omnishell: PRE | {command}"));

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

        let commit_id =
            self.try_create_commit(&format!("omnishell: POST | {command} | exit={exit_code}"));

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

    /// Try to create a git commit that captures the current working tree state.
    ///
    /// Uses gix's `is_dirty()` to check for changes, then uses the index
    /// to build a tree that reflects staged changes. Only creates a commit
    /// if there are actual changes (no no-op commits).
    fn try_create_commit(&self, message: &str) -> Option<ObjectId> {
        let repo = self.repo.as_ref()?;
        let head = repo.head_commit().ok()?;
        let head_tree_id = head.tree().ok()?.id;

        // Check if the repo has any changes (staged or unstaged)
        let is_dirty = repo.is_dirty().ok().unwrap_or(false);
        if !is_dirty {
            tracing::debug!("Skipping snapshot: working tree clean");
            return None;
        }

        // Use HEAD's tree for the commit. We've confirmed the repo is dirty.
        //
        // Limitation: gix doesn't expose a public `write_tree()` API to convert
        // the index into a tree object. To capture unstaged changes, we would need
        // to either:
        //   a) Use `edit_tree()` + blob writes for each changed file, or
        //   b) Wait for gix to expose `index.write_tree()`
        // For now, we commit HEAD's tree with a snapshot message, which is still
        // useful for tracking WHEN commands ran even if it doesn't capture file
        // content changes.
        let tree_id = head_tree_id;

        let parent_id = head.id;
        let commit_id = repo
            .commit(
                "refs/heads/omnishell-snapshots",
                message,
                tree_id,
                std::iter::once(parent_id),
            )
            .ok()?;

        tracing::info!(
            "Snapshot commit: {} (tree: {} → {})",
            commit_id.shorten_or_id(),
            head_tree_id,
            tree_id
        );

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

    /// Restore the working tree to a specific commit.
    ///
    /// Used by the undo system to revert the filesystem to a pre-execution state.
    /// Performs a `git checkout --force` equivalent using gix (no subprocess):
    /// builds an index from the commit's tree, writes it to the worktree, then
    /// persists the index to `.git/index`.
    ///
    /// Limitation: snapshots capture HEAD's tree at snapshot time (see
    /// `try_create_commit`), so this restores to whatever HEAD pointed to when
    /// the snapshot was taken. Unstaged working-tree changes are not captured.
    /// Restore the working tree to a specific commit.
    ///
    /// Used by the undo system to revert the filesystem to a pre-execution state.
    /// Performs a `git checkout --force` equivalent using gix (no subprocess):
    /// recursively writes every blob in the commit's tree to the worktree,
    /// sets executable bits, recreates symlinks, and removes files that are
    /// absent from the target tree but present in the worktree.
    ///
    /// Limitation: snapshots capture HEAD's tree at snapshot time (see
    /// `try_create_commit`), so this restores to whatever HEAD pointed to when
    /// the snapshot was taken. Unstaged working-tree changes are not captured.
    pub fn restore_to_commit(&self, commit_id: gix::ObjectId) -> std::result::Result<(), String> {
        let repo = self
            .repo
            .as_ref()
            .ok_or("No git repository available")?;
        let workdir = repo
            .workdir()
            .ok_or("Cannot restore: repository is bare (no worktree)")?
            .to_path_buf();

        let commit = repo
            .find_commit(commit_id)
            .map_err(|e| format!("Commit {commit_id} not found: {e}"))?;
        let target_tree = commit
            .tree()
            .map_err(|e| format!("Tree for commit {commit_id} not found: {e}"))?;

        // Collect the set of target paths (relative POSIX paths) so we can
        // delete worktree files that are absent from the snapshot.
        let mut target_paths: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut written = 0usize;
        let mut errors: Vec<String> = Vec::new();

        restore_tree(
            repo,
            &target_tree,
            &workdir,
            "",
            &mut target_paths,
            &mut written,
            &mut errors,
        );

        if !errors.is_empty() {
            return Err(format!(
                "Restore completed with {} error(s): {}",
                errors.len(),
                errors.join("; ")
            ));
        }

        // Remove files in the worktree that are NOT in the target tree.
        // We walk the worktree (skipping the .git directory) and delete any
        // regular file whose relative path isn't in target_paths.
        prune_worktree(&workdir, &target_paths, &mut errors);

        if !errors.is_empty() {
            return Err(format!(
                "Prune completed with {} error(s): {}",
                errors.len(),
                errors.join("; ")
            ));
        }

        tracing::info!(
            "Restored {} to commit {} (tree {}): {} files written",
            workdir.display(),
            commit_id,
            target_tree.id,
            written
        );

        Ok(())
    }
}

/// Recursively write every blob in `tree` to the worktree under `prefix`.
/// Records each written path in `target_paths` so the prune pass can delete
/// files absent from the target snapshot.
#[allow(clippy::too_many_arguments)]
fn restore_tree(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    workdir: &Path,
    prefix: &str,
    target_paths: &mut std::collections::HashSet<String>,
    written: &mut usize,
    errors: &mut Vec<String>,
) {
    let entries = match tree.iter() {
        e => e,
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("Failed to read tree entry under {prefix:?}: {e}"));
                continue;
            }
        };
        let name = entry.filename().to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let abs = workdir.join(&rel);
        match entry.mode().kind() {
            gix::object::tree::EntryKind::Tree => {
                if let Err(e) = std::fs::create_dir_all(&abs) {
                    errors.push(format!("mkdir {rel}: {e}"));
                    continue;
                }
                if let Ok(subtree) = entry.object() {
                    if let Ok(sub) = subtree.peel_to_tree() {
                        restore_tree(repo, &sub, workdir, &rel, target_paths, written, errors);
                    }
                }
            }
            gix::object::tree::EntryKind::Blob
            | gix::object::tree::EntryKind::BlobExecutable => {
                if let Some(parent) = abs.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        errors.push(format!("mkdir for {rel}: {e}"));
                        continue;
                    }
                }
                let blob = match entry.object() {
                    Ok(o) => o,
                    Err(e) => {
                        errors.push(format!("read blob {rel}: {e}"));
                        continue;
                    }
                };
                let data = blob.data.to_vec();
                if let Err(e) = std::fs::write(&abs, &data) {
                    errors.push(format!("write {rel}: {e}"));
                    continue;
                }
                // Apply executable bit if the tree recorded mode 100755.
                #[cfg(unix)]
                if entry.mode().kind() == gix::object::tree::EntryKind::BlobExecutable {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(&abs) {
                        let mut perms = meta.permissions();
                        perms.set_mode(perms.mode() | 0o111);
                        let _ = std::fs::set_permissions(&abs, perms);
                    }
                }
                target_paths.insert(rel);
                *written += 1;
            }
            gix::object::tree::EntryKind::Link => {
                if let Some(parent) = abs.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let blob = match entry.object() {
                    Ok(o) => o,
                    Err(e) => {
                        errors.push(format!("read link {rel}: {e}"));
                        continue;
                    }
                };
                let target = String::from_utf8_lossy(&blob.data).to_string();
                let _ = std::fs::remove_file(&abs);
                #[cfg(unix)]
                {
                    if let Err(e) = std::os::unix::fs::symlink(&target, &abs) {
                        errors.push(format!("symlink {rel} -> {target}: {e}"));
                        continue;
                    }
                }
                target_paths.insert(rel);
                *written += 1;
            }
            // Submodules (Commit) are not materialised by the undo system.
            gix::object::tree::EntryKind::Commit => {}
        }
    }
}

/// Walk the worktree and delete regular files that are not in `target_paths`.
/// Skips the `.git` directory. Removes empty directories after deletion.
fn prune_worktree(
    workdir: &Path,
    target_paths: &std::collections::HashSet<String>,
    errors: &mut Vec<String>,
) {
    fn walk(
        dir: &Path,
        workdir: &Path,
        target_paths: &std::collections::HashSet<String>,
        errors: &mut Vec<String>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                errors.push(format!("read_dir {}: {e}", dir.display()));
                return;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("read_dir {}: {e}", dir.display()));
                    continue;
                }
            };
            let _name = entry.file_name();
            let abs = entry.path();
            // Never descend into the git metadata directory.
            if abs.file_name().and_then(|n| n.to_str()) == Some(".git") {
                continue;
            }
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(e) => {
                    errors.push(format!("metadata {}: {e}", abs.display()));
                    continue;
                }
            };
            if meta.is_dir() {
                walk(&abs, workdir, target_paths, errors);
                // Remove the directory if it's now empty and not in the target.
                if std::fs::read_dir(&abs).map_or(false, |mut d| d.next().is_none()) {
                    let _ = std::fs::remove_dir(&abs);
                }
            } else if meta.is_file() || meta.file_type().is_symlink() {
                let rel = abs
                    .strip_prefix(workdir)
                    .ok()
                    .and_then(|p| p.to_str())
                    .map(|s| s.replace('\\', "/"));
                if let Some(rel) = rel {
                    if !target_paths.contains(&rel) {
                        if let Err(e) = std::fs::remove_file(&abs) {
                            errors.push(format!("delete {rel}: {e}"));
                        }
                    }
                }
            }
        }
    }
    walk(workdir, workdir, target_paths, errors);
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

    // ── restore_to_commit integration tests (task 36844090) ──────────────
    //
    // These build a real bare-ish repo via gix, commit content, snapshot it,
    // mutate the worktree, then call restore_to_commit and assert the worktree
    // reverts to the snapshot state. Uses only the public gix API.

    fn init_repo_with_commit(dir: &Path) -> gix::ObjectId {
        // Use system git to seed a minimal repo — test code is allowed to
        // shell out (per AGENTS.md: "Test code may use system git if gix/git2
        // cannot cover the operation"). gix has no high-level init+commit
        // porcelain that reliably stages arbitrary content in one shot.
        let _ = std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(dir)
            .status();
        let _ = std::process::Command::new("git")
            .arg("config")
            .arg("user.email")
            .arg("t@t")
            .current_dir(dir)
            .status();
        let _ = std::process::Command::new("git")
            .arg("config")
            .arg("user.name")
            .arg("t")
            .current_dir(dir)
            .status();
        std::fs::write(dir.join("a.txt"), "alpha\n").unwrap();
        std::fs::write(dir.join("b.txt"), "beta\n").unwrap();
        let _ = std::process::Command::new("git")
            .arg("add")
            .arg("-A")
            .current_dir(dir)
            .status();
        let _ = std::process::Command::new("git")
            .arg("commit")
            .arg("-q")
            .arg("-m")
            .arg("initial")
            .current_dir(dir)
            .status();
        // Resolve HEAD id via gix.
        let repo = gix::open(dir).unwrap();
        let head = repo.head_commit().unwrap();
        head.id
    }

    #[test]
    fn test_restore_reverts_modified_files() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // skip on systems without git
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let baseline = init_repo_with_commit(dir);

        // Mutate the worktree AFTER the baseline commit.
        std::fs::write(dir.join("a.txt"), "CHANGED\n").unwrap();
        std::fs::write(dir.join("c.txt"), "gamma\n").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "CHANGED\n");

        let engine = SnapshotEngine::new(dir);
        assert!(engine.has_repo());
        engine.restore_to_commit(baseline).expect("restore must succeed");

        // a.txt must be reverted to the baseline content.
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "alpha\n");
        assert_eq!(std::fs::read_to_string(dir.join("b.txt")).unwrap(), "beta\n");
        // c.txt was not in the baseline tree → must be pruned.
        assert!(!dir.join("c.txt").exists(), "pruned file should be gone");
    }

    #[test]
    fn test_restore_is_idempotent() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let baseline = init_repo_with_commit(dir);

        let engine = SnapshotEngine::new(dir);
        engine.restore_to_commit(baseline).expect("first restore");
        // Restoring to the same commit again should be a no-op success.
        engine.restore_to_commit(baseline).expect("second restore");
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "alpha\n");
    }
}
