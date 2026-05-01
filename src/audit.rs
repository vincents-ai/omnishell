//! Audit logging and telemetry for OmniShell.
//!
//! Provides structured audit logging of all command executions.
//! Uses `tracing` for structured spans and events.
//! Outputs JSON-formatted audit logs per profile.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// An audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp (epoch seconds).
    pub timestamp: u64,
    /// The command that was executed.
    pub command: String,
    /// Exit code.
    pub exit_code: i32,
    /// ACL verdict (allowed/denied).
    pub acl_verdict: String,
    /// Profile mode at time of execution.
    pub mode: Mode,
    /// Working directory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Whether a snapshot was created.
    pub snapshot_created: bool,
    /// User (from $USER or profile).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// Audit log configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    /// Whether audit logging is enabled.
    #[serde(default = "audit_default_true")]
    pub enabled: bool,
    /// Directory for audit log files.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_dir: Option<PathBuf>,
    /// Maximum log file size in bytes before rotation.
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
}

fn default_max_file_size() -> u64 {
    10 * 1024 * 1024 // 10 MB
}

fn audit_default_true() -> bool {
    true
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            log_dir: None,
            max_file_size: default_max_file_size(),
        }
    }
}

/// The audit logger.
pub struct AuditLogger {
    config: AuditConfig,
    log_path: PathBuf,
}

impl AuditLogger {
    /// Create a new audit logger for the given mode.
    pub fn new(mode: Mode, config: AuditConfig) -> Self {
        let log_dir = config
            .log_dir
            .clone()
            .unwrap_or_else(default_audit_dir);

        let filename = match mode {
            Mode::Kids => "audit_kids.jsonl",
            Mode::Agent => "audit_agent.jsonl",
            Mode::Admin => "audit_admin.jsonl",
        };

        Self {
            config,
            log_path: log_dir.join(filename),
        }
    }

    /// Check if audit logging is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Log an audit entry.
    pub fn log(&self, entry: AuditEntry) -> std::io::Result<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Also emit a tracing event
        tracing::info!(
            command = %entry.command,
            exit_code = entry.exit_code,
            verdict = %entry.acl_verdict,
            mode = ?entry.mode,
            duration_ms = entry.duration_ms,
            "command executed"
        );

        // Write to file
        self.write_to_disk(&entry)
    }

    /// Create an audit entry builder for a command.
    pub fn entry_for(command: &str, mode: Mode) -> AuditEntryBuilder {
        AuditEntryBuilder {
            command: command.to_string(),
            mode,
            timestamp: now_secs(),
            exit_code: 0,
            acl_verdict: "allowed".to_string(),
            working_dir: std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
            duration_ms: 0,
            snapshot_created: false,
            user: std::env::var("USER").ok(),
        }
    }

    /// Write an entry to the audit log file.
    fn write_to_disk(&self, entry: &AuditEntry) -> std::io::Result<()> {
        if let Some(parent) = self.log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string(entry)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_path)?;

        writeln!(file, "{json}")?;

        // TODO: file rotation based on max_file_size

        Ok(())
    }

    /// Get the log file path.
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }
}

/// Builder for audit entries.
pub struct AuditEntryBuilder {
    command: String,
    mode: Mode,
    timestamp: u64,
    exit_code: i32,
    acl_verdict: String,
    working_dir: Option<String>,
    duration_ms: u64,
    snapshot_created: bool,
    user: Option<String>,
}

impl AuditEntryBuilder {
    pub fn exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }

    pub fn acl_verdict(mut self, verdict: &str) -> Self {
        self.acl_verdict = verdict.to_string();
        self
    }

    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.duration_ms = ms;
        self
    }

    pub fn snapshot_created(mut self, created: bool) -> Self {
        self.snapshot_created = created;
        self
    }

    pub fn build(self) -> AuditEntry {
        AuditEntry {
            timestamp: self.timestamp,
            command: self.command,
            exit_code: self.exit_code,
            acl_verdict: self.acl_verdict,
            mode: self.mode,
            working_dir: self.working_dir,
            duration_ms: self.duration_ms,
            snapshot_created: self.snapshot_created,
            user: self.user,
        }
    }
}

fn default_audit_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("omnishell")
        .join("audit")
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
    fn test_audit_entry_builder() {
        let entry = AuditLogger::entry_for("ls -la", Mode::Admin)
            .exit_code(0)
            .acl_verdict("allowed")
            .duration_ms(42)
            .snapshot_created(false)
            .build();

        assert_eq!(entry.command, "ls -la");
        assert_eq!(entry.exit_code, 0);
        assert_eq!(entry.acl_verdict, "allowed");
        assert_eq!(entry.duration_ms, 42);
        assert!(!entry.snapshot_created);
    }

    #[test]
    fn test_audit_entry_serialization() {
        let entry = AuditLogger::entry_for("rm file", Mode::Kids)
            .exit_code(1)
            .acl_verdict("denied: blocked by kids mode")
            .build();

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"command\":\"rm file\""));
        assert!(json.contains("\"exit_code\":1"));
        assert!(json.contains("denied"));
    }

    #[test]
    fn test_audit_logger_writes_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let config = AuditConfig {
            enabled: true,
            log_dir: Some(dir.path().to_path_buf()),
            max_file_size: default_max_file_size(),
        };

        let logger = AuditLogger::new(Mode::Agent, config);
        let entry = AuditLogger::entry_for("cargo build", Mode::Agent)
            .exit_code(0)
            .build();

        logger.log(entry).unwrap();

        // Verify file was created and contains valid JSONL
        let content = std::fs::read_to_string(logger.log_path()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(content.trim()).unwrap();
        assert_eq!(parsed["command"], "cargo build");
        assert_eq!(parsed["mode"], "agent");
    }

    #[test]
    fn test_audit_logger_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let config = AuditConfig {
            enabled: false,
            log_dir: Some(dir.path().to_path_buf()),
            max_file_size: default_max_file_size(),
        };

        let logger = AuditLogger::new(Mode::Admin, config);
        let entry = AuditLogger::entry_for("ls", Mode::Admin).build();

        logger.log(entry).unwrap();

        // File should not be created when disabled
        assert!(!logger.log_path().exists());
    }

    #[test]
    fn test_audit_logger_appends() {
        let dir = tempfile::tempdir().unwrap();
        let config = AuditConfig {
            enabled: true,
            log_dir: Some(dir.path().to_path_buf()),
            max_file_size: default_max_file_size(),
        };

        let logger = AuditLogger::new(Mode::Admin, config);

        for i in 0..5 {
            let entry = AuditLogger::entry_for(&format!("cmd{i}"), Mode::Admin).build();
            logger.log(entry).unwrap();
        }

        let content = std::fs::read_to_string(logger.log_path()).unwrap();
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn test_audit_config_default() {
        let config = AuditConfig::default();
        assert!(config.enabled);
        assert!(config.log_dir.is_none());
        assert_eq!(config.max_file_size, 10 * 1024 * 1024);
    }
}
