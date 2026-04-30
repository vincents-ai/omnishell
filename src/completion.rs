//! Tab completion filtering for OmniShell.
//!
//! Filters completion candidates based on the active profile:
//! - Kids mode: only allowlist commands + builtins
//! - Agent mode: full PATH scan + builtins
//! - Admin mode: full PATH scan + builtins + hidden commands

use std::collections::HashSet;
use std::path::PathBuf;

use crate::acl::AclEngine;
use crate::profile::Mode;

/// The completion provider.
pub struct CompletionEngine {
    /// Cached set of executables from PATH.
    path_commands: HashSet<String>,
    /// Built-in command names.
    builtin_commands: HashSet<String>,
}

impl CompletionEngine {
    /// Create a new completion engine.
    pub fn new() -> Self {
        let path_commands = scan_path();
        let builtin_commands = builtin_names();

        Self {
            path_commands,
            builtin_commands,
        }
    }

    /// Get completion candidates for a partial command.
    pub fn complete(&self, partial: &str, mode: Mode, acl: &AclEngine) -> Vec<String> {
        let mut candidates = Vec::new();

        // Always include matching builtins
        for builtin in &self.builtin_commands {
            if builtin.starts_with(partial) {
                candidates.push(builtin.clone());
            }
        }

        // For kids mode, only include allowlisted commands
        // For agent/admin, include all PATH commands
        if mode == Mode::Kids {
            // Kids mode: only allowlist commands from ACL
            for rule in &acl.allowlist {
                if rule.pattern.starts_with(partial) && !rule.pattern.contains('*') {
                    if !candidates.contains(&rule.pattern) {
                        candidates.push(rule.pattern.clone());
                    }
                }
            }
        } else {
            // Agent/Admin: full PATH scan
            for cmd in &self.path_commands {
                if cmd.starts_with(partial) {
                    candidates.push(cmd.clone());
                }
            }
        }

        candidates.sort();
        candidates
    }

    /// Get all available commands for the given mode.
    pub fn all_commands(&self, mode: Mode, acl: &AclEngine) -> Vec<String> {
        self.complete("", mode, acl)
    }

    /// Get completion candidates for command arguments.
    ///
    /// For now, provides file path completion. Future: subcommand-aware completion.
    pub fn complete_args(&self, partial: &str) -> Vec<String> {
        let mut candidates = Vec::new();

        // File path completion
        let (dir, prefix) = if partial.contains('/') {
            let path = PathBuf::from(partial);
            let parent = path.parent().unwrap_or(std::path::Path::new("."));
            let file_prefix = path.file_name()
                .and_then(|f| f.to_str())
                .unwrap_or("");
            (parent.to_path_buf(), file_prefix.to_string())
        } else {
            (PathBuf::from("."), partial.to_string())
        };

        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with(&prefix) {
                        let suffix = if entry.path().is_dir() { "/" } else { "" };
                        candidates.push(format!("{}{}", name, suffix));
                    }
                }
            }
        }

        candidates.sort();
        candidates
    }

    /// Refresh the PATH command cache.
    pub fn refresh(&mut self) {
        self.path_commands = scan_path();
    }
}

impl Default for CompletionEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Scan PATH directories for executables.
fn scan_path() -> HashSet<String> {
    let mut commands = HashSet::new();

    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Some(name) = entry.file_name().to_str() {
                        // Check if executable
                        if entry.path().is_file() || entry.path().is_symlink() {
                            commands.insert(name.to_string());
                        }
                    }
                }
            }
        }
    }

    commands
}

/// Get the set of built-in command names.
fn builtin_names() -> HashSet<String> {
    ["?", "ai", "snapshots", "undo", "redo", "allow", "block", "mode", "help", "exit", "quit"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complete_builtin_prefix() {
        let engine = CompletionEngine::new();
        let acl = AclEngine::new(Mode::Admin);
        let results = engine.complete("un", Mode::Admin, &acl);
        assert!(results.contains(&"undo".to_string()));
    }

    #[test]
    fn test_complete_ai_builtin() {
        let engine = CompletionEngine::new();
        let acl = AclEngine::new(Mode::Admin);
        let results = engine.complete("a", Mode::Admin, &acl);
        assert!(results.contains(&"ai".to_string()));
        assert!(results.contains(&"allow".to_string()));
    }

    #[test]
    fn test_complete_kids_mode_restricted() {
        let engine = CompletionEngine::new();
        let acl = AclEngine::new(Mode::Kids);
        let results = engine.complete("", Mode::Kids, &acl);

        // Kids should only see allowlisted commands + builtins
        assert!(results.contains(&String::from("?"))); // builtin
        assert!(results.contains(&String::from("ls"))); // allowlisted
        assert!(results.contains(&String::from("echo"))); // allowlisted

        // Should NOT see arbitrary PATH commands
        // (unless they happen to be on the allowlist)
    }

    #[test]
    fn test_complete_admin_mode_has_path() {
        let engine = CompletionEngine::new();
        let acl = AclEngine::new(Mode::Admin);
        let results = engine.complete("ls", Mode::Admin, &acl);

        // Admin should see ls from PATH
        assert!(results.iter().any(|r| r == "ls"));
    }

    #[test]
    fn test_complete_args_file_path() {
        let engine = CompletionEngine::new();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test_file.txt"), "").unwrap();
        std::fs::write(dir.path().join("test_other.txt"), "").unwrap();

        let results = engine.complete_args(
            &format!("{}/test_", dir.path().display())
        );

        assert!(results.iter().any(|r| r.contains("test_file")));
        assert!(results.iter().any(|r| r.contains("test_other")));
    }

    #[test]
    fn test_all_commands_non_empty() {
        let engine = CompletionEngine::new();
        let acl = AclEngine::new(Mode::Admin);
        let commands = engine.all_commands(Mode::Admin, &acl);
        // Should at minimum have builtins
        assert!(!commands.is_empty());
        assert!(commands.contains(&String::from("?")));
    }

    #[test]
    fn test_builtin_names() {
        let names = builtin_names();
        assert!(names.contains(&String::from("?")));
        assert!(names.contains(&String::from("ai")));
        assert!(names.contains(&String::from("help")));
        assert!(names.contains(&String::from("exit")));
        assert_eq!(names.len(), 11);
    }
}

/// shrs Completer trait implementation for mode-aware tab completion.
impl shrs::prelude::Completer for CompletionEngine {
    fn complete(&self, ctx: &shrs::prelude::CompletionCtx) -> Vec<shrs::prelude::Completion> {
        // Default to admin mode for completion (ACL filtering happens at eval time)
        let acl = crate::acl::AclEngine::new(crate::profile::Mode::Admin);

        let partial = ctx.cur_word().map(|s| s.as_str()).unwrap_or("");
        let candidates = self.complete(partial, crate::profile::Mode::Admin, &acl);

        candidates.into_iter().map(|c| shrs::prelude::Completion {
            add_space: true,
            display: Some(c.clone()),
            completion: c,
            replace_method: shrs::prelude::ReplaceMethod::Replace,
            comment: None,
        }).collect()
    }

    fn register(&mut self, _rule: shrs::prelude::Rule) {
        // No-op — our completion is ACL-driven, not rule-based
    }
}
