//! Output formatting layer for OmniShell.
//!
//! Wraps command output in structured envelopes based on the active mode:
//! - Kids: Colored, friendly output with emojis
//! - Agent: JSON envelope with type, stdout, stderr, exit_code
//! - Admin: Plain passthrough (raw stdout/stderr)

use std::fmt;

#[cfg(unix)]
use std::fs::OpenOptions;
#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::path::PathBuf;

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

/// Emit one agent-mode JSON envelope on stdout.
///
/// This is the single output path for agent mode: the envelope is written to
/// stdout, while anything that is not the envelope (startup banners, ACL
/// diagnostics, progress) stays on stderr.
pub fn emit_agent(command: &str, stdout: &str, stderr: &str, exit_code: i32, duration_ms: u64) {
    let output = CommandOutput {
        command: command.to_string(),
        stdout: stdout.to_string(),
        stderr: stderr.to_string(),
        exit_code,
        duration_ms,
    };
    println!("{}", format_output(&output, Mode::Agent));
}

/// Redirect this process's stdout/stderr into temporary files for the duration
/// of one agent-mode command, so the command's real output can be placed in the
/// JSON envelope instead of being written directly to the terminal.
///
/// The saved descriptors are restored by [`AgentCapture::finish`] or, if the
/// command panics, by `Drop`.
#[cfg(unix)]
pub struct AgentCapture {
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    saved_stdout: i32,
    saved_stderr: i32,
    active: bool,
}

#[cfg(unix)]
impl AgentCapture {
    pub fn start() -> std::io::Result<Self> {
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();

        let unique = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let stdout_path = std::env::temp_dir().join(format!("omnishell-{unique}.stdout"));
        let stderr_path = std::env::temp_dir().join(format!("omnishell-{unique}.stderr"));

        let stdout_file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&stdout_path)?;
        let stderr_file = match OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&stderr_path)
        {
            Ok(file) => file,
            Err(e) => {
                let _ = std::fs::remove_file(&stdout_path);
                return Err(e);
            }
        };

        let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_stdout < 0 {
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
            return Err(std::io::Error::last_os_error());
        }
        let saved_stderr = unsafe { libc::dup(libc::STDERR_FILENO) };
        if saved_stderr < 0 {
            unsafe { libc::close(saved_stdout) };
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
            return Err(std::io::Error::last_os_error());
        }

        if unsafe { libc::dup2(stdout_file.as_raw_fd(), libc::STDOUT_FILENO) } < 0
            || unsafe { libc::dup2(stderr_file.as_raw_fd(), libc::STDERR_FILENO) } < 0
        {
            unsafe {
                libc::dup2(saved_stdout, libc::STDOUT_FILENO);
                libc::dup2(saved_stderr, libc::STDERR_FILENO);
                libc::close(saved_stdout);
                libc::close(saved_stderr);
            }
            let _ = std::fs::remove_file(&stdout_path);
            let _ = std::fs::remove_file(&stderr_path);
            return Err(std::io::Error::last_os_error());
        }

        // The File handles can be dropped now; the duplicated descriptors are
        // what the process and any child commands write through.
        drop(stdout_file);
        drop(stderr_file);

        Ok(Self {
            stdout_path,
            stderr_path,
            saved_stdout,
            saved_stderr,
            active: true,
        })
    }

    /// Restore stdout/stderr and return everything the command wrote.
    pub fn finish(mut self) -> (String, String) {
        self.restore();
        let stdout = std::fs::read(&self.stdout_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let stderr = std::fs::read(&self.stderr_path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let _ = std::fs::remove_file(&self.stdout_path);
        let _ = std::fs::remove_file(&self.stderr_path);
        (stdout, stderr)
    }

    fn restore(&mut self) {
        if !self.active {
            return;
        }
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        unsafe {
            libc::dup2(self.saved_stdout, libc::STDOUT_FILENO);
            libc::dup2(self.saved_stderr, libc::STDERR_FILENO);
            libc::close(self.saved_stdout);
            libc::close(self.saved_stderr);
        }
        self.active = false;
    }
}

#[cfg(unix)]
impl Drop for AgentCapture {
    fn drop(&mut self) {
        self.restore();
        if !self.active {
            let _ = std::fs::remove_file(&self.stdout_path);
            let _ = std::fs::remove_file(&self.stderr_path);
        }
    }
}

/// Non-Unix fallback: no descriptor capture. OmniShell does not currently
/// build on Windows; this keeps the type available for cross-checking.
#[cfg(not(unix))]
pub struct AgentCapture;

#[cfg(not(unix))]
impl AgentCapture {
    pub fn start() -> std::io::Result<Self> {
        Ok(Self)
    }

    pub fn finish(self) -> (String, String) {
        (String::new(), String::new())
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
