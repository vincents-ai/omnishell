//! Output formatting layer for OmniShell.
//!
//! Wraps command output in structured envelopes based on the active mode:
//! - Kids: Colored, friendly output with emojis
//! - Agent: JSON envelope with type, stdout, stderr, exit_code
//! - Admin: Plain passthrough (raw stdout/stderr)

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Structured output from a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutput {
    /// The command that was executed.
    pub command: String,
    /// Standard output content.
    pub stdout: String,
    /// Standard error content.
    pub stderr: String,
    /// Process exit code.
    pub exit_code: i32,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// A JSON envelope for agent mode output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonEnvelope {
    /// The type of result.
    #[serde(rename = "type")]
    pub result_type: String,
    /// The command that was executed.
    pub command: String,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// Process exit code.
    #[serde(rename = "exitCode")]
    pub exit_code: i32,
    /// Duration in milliseconds.
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
}

/// Format command output based on the active mode.
pub fn format_output(output: &CommandOutput, mode: Mode) -> String {
    match mode {
        Mode::Kids => format_kids(output),
        Mode::Agent => format_agent(output),
        Mode::Admin => format_admin(output),
    }
}

/// Kids mode: friendly, colored output.
fn format_kids(output: &CommandOutput) -> String {
    let mut result = String::new();

    if output.exit_code == 0 {
        result.push_str(&format!("✅ {}\n", output.stdout.trim()));
    } else {
        result.push_str(&format!(
            "❌ Oops! Something went wrong:\n{}\n",
            output.stderr.trim()
        ));
    }

    result
}

/// Agent mode: JSON envelope.
fn format_agent(output: &CommandOutput) -> String {
    let envelope = JsonEnvelope {
        result_type: if output.exit_code == 0 {
            "success"
        } else {
            "error"
        }
        .to_string(),
        command: output.command.clone(),
        stdout: output.stdout.clone(),
        stderr: output.stderr.clone(),
        exit_code: output.exit_code,
        duration_ms: output.duration_ms,
    };

    serde_json::to_string(&envelope).unwrap_or_else(|e| {
        format!(
            r#"{{"type":"error","command":"{}","stdout":"","stderr":"JSON serialization failed: {}","exitCode":1,"durationMs":0}}"#,
            output.command, e
        )
    })
}

/// Admin mode: plain passthrough.
fn format_admin(output: &CommandOutput) -> String {
    let mut result = String::new();
    if !output.stdout.is_empty() {
        result.push_str(&output.stdout);
    }
    if !output.stderr.is_empty() {
        result.push_str(&output.stderr);
    }
    result
}

/// Format an error message for the given mode.
pub fn format_error(message: &str, mode: Mode) -> String {
    match mode {
        Mode::Kids => format!("❌ {message}"),
        Mode::Agent => {
            let envelope = JsonEnvelope {
                result_type: "error".to_string(),
                command: String::new(),
                stdout: String::new(),
                stderr: message.to_string(),
                exit_code: 1,
                duration_ms: 0,
            };
            serde_json::to_string(&envelope)
                .unwrap_or_else(|_| format!(r#"{{"type":"error","stderr":"{message}"}}"#))
        }
        Mode::Admin => format!("error: {message}"),
    }
}

impl fmt::Display for CommandOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[exit={}] stdout={} bytes, stderr={} bytes",
            self.exit_code,
            self.stdout.len(),
            self.stderr.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output(exit_code: i32) -> CommandOutput {
        CommandOutput {
            command: "ls -la".to_string(),
            stdout: "file1.txt\nfile2.txt\n".to_string(),
            stderr: if exit_code != 0 {
                "permission denied".to_string()
            } else {
                String::new()
            },
            exit_code,
            duration_ms: 42,
        }
    }

    #[test]
    fn test_format_kids_success() {
        let output = make_output(0);
        let result = format_kids(&output);
        assert!(result.contains("✅"));
        assert!(result.contains("file1.txt"));
    }

    #[test]
    fn test_format_kids_error() {
        let output = make_output(1);
        let result = format_kids(&output);
        assert!(result.contains("❌"));
        assert!(result.contains("permission denied"));
    }

    #[test]
    fn test_format_agent_success() {
        let output = make_output(0);
        let result = format_agent(&output);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["type"], "success");
        assert_eq!(parsed["exitCode"], 0);
        assert_eq!(parsed["durationMs"], 42);
    }

    #[test]
    fn test_format_agent_error() {
        let output = make_output(1);
        let result = format_agent(&output);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["type"], "error");
        assert_eq!(parsed["exitCode"], 1);
        assert_eq!(parsed["stderr"], "permission denied");
    }

    #[test]
    fn test_format_admin() {
        let output = make_output(0);
        let result = format_admin(&output);
        assert_eq!(result, "file1.txt\nfile2.txt\n");
    }

    #[test]
    fn test_format_error_kids() {
        let result = format_error("test error", Mode::Kids);
        assert!(result.contains("❌"));
    }

    #[test]
    fn test_format_error_agent() {
        let result = format_error("test error", Mode::Agent);
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["type"], "error");
    }

    #[test]
    fn test_format_error_admin() {
        let result = format_error("test error", Mode::Admin);
        assert!(result.contains("error:"));
    }

    #[test]
    fn test_format_output_dispatches_correctly() {
        let output = make_output(0);

        let kids = format_output(&output, Mode::Kids);
        assert!(kids.contains("✅"));

        let agent = format_output(&output, Mode::Agent);
        let parsed: serde_json::Value = serde_json::from_str(&agent).unwrap();
        assert_eq!(parsed["type"], "success");

        let admin = format_output(&output, Mode::Admin);
        assert_eq!(admin, "file1.txt\nfile2.txt\n");
    }
}
