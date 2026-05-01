//! Shell history management for OmniShell.
//!
//! Per-profile command history stored in XDG data directory.
//! Features:
//! - Separate history files per profile (kids, agent, admin)
//! - Configurable max entries
//! - Deduplication of consecutive identical commands
//! - Timestamp recording
//! - Search / reverse search

use std::collections::VecDeque;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// A single history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The command that was executed.
    pub command: String,
    /// Timestamp (epoch seconds).
    pub timestamp: u64,
    /// Exit code.
    pub exit_code: i32,
    /// Working directory at time of execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

/// History configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Maximum number of entries to keep.
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// Whether to save history to disk.
    #[serde(default = "default_true")]
    pub persistent: bool,
    /// Whether to deduplicate consecutive identical commands.
    #[serde(default = "default_true")]
    pub deduplicate: bool,
}

fn default_max_entries() -> usize {
    10_000
}

fn default_true() -> bool {
    true
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_entries: default_max_entries(),
            persistent: true,
            deduplicate: true,
        }
    }
}

/// The shell history manager.
pub struct History {
    entries: VecDeque<HistoryEntry>,
    config: HistoryConfig,
    /// Path to the history file.
    file_path: PathBuf,
    /// Current search position (for reverse-i-search).
    search_pos: Option<usize>,
}

impl History {
    /// Create a new history manager for the given profile and mode.
    pub fn new(mode: Mode, config: HistoryConfig) -> Self {
        let file_path = history_file_path(mode);
        let mut history = Self {
            entries: VecDeque::with_capacity(config.max_entries),
            config,
            file_path,
            search_pos: None,
        };

        if history.config.persistent {
            let _ = history.load_from_disk();
        }

        history
    }

    /// Add a command to history.
    pub fn push(&mut self, command: &str, exit_code: i32, working_dir: Option<&Path>) {
        // Skip empty commands
        if command.trim().is_empty() {
            return;
        }

        // Deduplicate consecutive identical commands
        if self.config.deduplicate {
            if let Some(last) = self.entries.back() {
                if last.command == command {
                    return;
                }
            }
        }

        let entry = HistoryEntry {
            command: command.to_string(),
            timestamp: now_secs(),
            exit_code,
            working_dir: working_dir.map(|p| p.to_string_lossy().to_string()),
        };

        // Enforce max entries
        if self.entries.len() >= self.config.max_entries {
            self.entries.pop_front();
        }

        self.entries.push_back(entry);
        self.search_pos = None;

        // Auto-save
        if self.config.persistent {
            let _ = self.save_to_disk();
        }
    }

    /// Get the N most recent entries.
    pub fn recent(&self, n: usize) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter().rev().take(n)
    }

    /// Get all entries.
    pub fn entries(&self) -> &VecDeque<HistoryEntry> {
        &self.entries
    }

    /// Search history for commands matching a pattern.
    pub fn search(&self, pattern: &str) -> Vec<&HistoryEntry> {
        let pattern_lower = pattern.to_lowercase();
        self.entries
            .iter()
            .rev()
            .filter(|e| e.command.to_lowercase().contains(&pattern_lower))
            .collect()
    }

    /// Get the previous entry in reverse search.
    pub fn search_prev(&mut self) -> Option<&HistoryEntry> {
        match self.search_pos {
            Some(pos) if pos + 1 < self.entries.len() => {
                self.search_pos = Some(pos + 1);
                Some(&self.entries[self.entries.len() - pos - 2])
            }
            Some(_) => None,
            None => {
                if !self.entries.is_empty() {
                    self.search_pos = Some(0);
                    Some(&self.entries[self.entries.len() - 1])
                } else {
                    None
                }
            }
        }
    }

    /// Reset search position.
    pub fn reset_search(&mut self) {
        self.search_pos = None;
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.search_pos = None;
        if self.config.persistent {
            let _ = self.save_to_disk();
        }
    }

    /// Get the number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if history is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Load history from disk.
    fn load_from_disk(&mut self) -> std::io::Result<()> {
        if !self.file_path.exists() {
            return Ok(());
        }

        let file = std::fs::File::open(&self.file_path)?;
        let reader = std::io::BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if let Ok(entry) = serde_json::from_str::<HistoryEntry>(&line) {
                if self.entries.len() >= self.config.max_entries {
                    self.entries.pop_front();
                }
                self.entries.push_back(entry);
            }
        }

        Ok(())
    }

    /// Save history to disk (JSONL format — one entry per line).
    fn save_to_disk(&self) -> std::io::Result<()> {
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = std::fs::File::create(&self.file_path)?;
        for entry in &self.entries {
            let json = serde_json::to_string(entry)?;
            writeln!(file, "{json}")?;
        }

        Ok(())
    }
}

/// Get the history file path for a mode.
impl Default for History {
    fn default() -> Self {
        Self::new(Mode::Admin, HistoryConfig::default())
    }
}

fn history_file_path(mode: Mode) -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("omnishell");

    let filename = match mode {
        Mode::Kids => "history_kids.jsonl",
        Mode::Agent => "history_agent.jsonl",
        Mode::Admin => "history_admin.jsonl",
    };

    data_dir.join(filename)
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
    fn test_push_and_recent() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("ls", 0, None);
        history.push("cd /tmp", 0, None);
        history.push("rm file", 1, None);

        let recent: Vec<_> = history.recent(2).collect();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].command, "rm file");
        assert_eq!(recent[1].command, "cd /tmp");
    }

    #[test]
    fn test_deduplicate_consecutive() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            deduplicate: true,
            ..Default::default()
        });

        history.push("ls", 0, None);
        history.push("ls", 0, None);
        history.push("ls", 0, None);

        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_no_deduplicate_different_commands() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            deduplicate: true,
            ..Default::default()
        });

        history.push("ls", 0, None);
        history.push("pwd", 0, None);
        history.push("ls", 0, None);

        assert_eq!(history.len(), 3);
    }

    #[test]
    fn test_search() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("cargo build", 0, None);
        history.push("cargo test", 0, None);
        history.push("ls", 0, None);

        let results = history.search("cargo");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_case_insensitive() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("Cargo Build", 0, None);
        let results = history.search("cargo");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_clear() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("ls", 0, None);
        history.push("pwd", 0, None);
        assert_eq!(history.len(), 2);

        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn test_max_entries_enforcement() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            max_entries: 3,
            ..Default::default()
        });

        history.push("cmd1", 0, None);
        history.push("cmd2", 0, None);
        history.push("cmd3", 0, None);
        history.push("cmd4", 0, None);

        assert_eq!(history.len(), 3);
        // First entry should be cmd2 (cmd1 evicted)
        assert_eq!(history.entries.front().unwrap().command, "cmd2");
    }

    #[test]
    fn test_empty_command_skipped() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("", 0, None);
        history.push("   ", 0, None);
        assert!(history.is_empty());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test_history.jsonl");

        let mut history = History {
            entries: VecDeque::new(),
            config: HistoryConfig {
                persistent: true,
                max_entries: 100,
                deduplicate: true,
            },
            file_path: file_path.clone(),
            search_pos: None,
        };

        history.push("cargo build", 0, Some(Path::new("/home/user/project")));
        history.push("cargo test", 0, None);

        // Save
        history.save_to_disk().unwrap();

        // Load into fresh history
        let mut history2 = History {
            entries: VecDeque::new(),
            config: HistoryConfig {
                persistent: false,
                max_entries: 100,
                deduplicate: false,
            },
            file_path: file_path.clone(),
            search_pos: None,
        };
        history2.load_from_disk().unwrap();

        assert_eq!(history2.len(), 2);
        assert_eq!(history2.entries[0].command, "cargo build");
        assert_eq!(history2.entries[0].working_dir, Some("/home/user/project".to_string()));
    }

    #[test]
    fn test_working_dir_recorded() {
        let mut history = History::new(Mode::Admin, HistoryConfig {
            persistent: false,
            ..Default::default()
        });

        history.push("ls", 0, Some(Path::new("/home/user")));
        assert_eq!(history.entries[0].working_dir, Some("/home/user".to_string()));
    }
}
