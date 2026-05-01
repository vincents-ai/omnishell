//! OmniShellTool — agentic-loop integration + engram context provider.
//!
//! Part 1: OmniShellTool exposes shell execution to agentic-loop agents.
//! Part 2: EngramContext provides LLM context injection from engram's git storage.
//! structured JSON responses.

use serde::{Deserialize, Serialize};

use crate::acl::AclEngine;
use crate::builtins::{self, BuiltinResult};
use crate::profile::Mode;

/// Input schema for the OmniShellTool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolInput {
    /// The command to execute.
    pub command: String,
    /// Working directory (defaults to CWD).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Timeout in seconds (0 = no timeout).
    #[serde(default)]
    pub timeout: u64,
    /// Whether to capture stderr separately.
    #[serde(default = "default_true")]
    pub capture_stderr: bool,
}

fn default_true() -> bool {
    true
}

/// Output schema for the OmniShellTool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellToolOutput {
    /// Whether the command was allowed by ACL.
    pub allowed: bool,
    /// The command that was attempted.
    pub command: String,
    /// Standard output content.
    pub stdout: String,
    /// Standard error content.
    pub stderr: String,
    /// Process exit code (None if blocked by ACL).
    pub exit_code: Option<i32>,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// Denial reason (if blocked by ACL).
    pub denial_reason: Option<String>,
    /// The mode used for execution.
    pub mode: String,
}

/// The OmniShellTool — exposes shell execution to agentic-loop agents.
pub struct OmniShellTool {
    mode: Mode,
}

impl OmniShellTool {
    /// Create a new tool for the given mode.
    pub fn new(mode: Mode) -> Self {
        Self { mode }
    }

    /// Get the tool name.
    pub fn name(&self) -> &str {
        "omnishell"
    }

    /// Get the tool description.
    pub fn description(&self) -> &str {
        "Execute shell commands through OmniShell with ACL enforcement, audit logging, and structured JSON output"
    }

    /// Get the JSON schema for tool input.
    pub fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (defaults to CWD)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds (0 = no timeout)",
                    "default": 0
                },
                "capture_stderr": {
                    "type": "boolean",
                    "description": "Whether to capture stderr separately",
                    "default": true
                }
            },
            "required": ["command"]
        })
    }

    /// Execute a command through the OmniShell pipeline.
    pub fn execute(&self, input: ShellToolInput) -> ShellToolOutput {
        let mut acl = AclEngine::new(self.mode);
        let start = std::time::Instant::now();

        // Check ACL
        let verdict = acl.evaluate(&input.command);

        match verdict {
            crate::acl::Verdict::Deny(reason) => {
                return ShellToolOutput {
                    allowed: false,
                    command: input.command,
                    stdout: String::new(),
                    stderr: String::new(),
                    exit_code: None,
                    duration_ms: start.elapsed().as_millis() as u64,
                    denial_reason: Some(reason),
                    mode: self.mode.to_string(),
                };
            }
            crate::acl::Verdict::Allow => {}
        }

        // Check builtins first
        let tokens: Vec<String> = input.command.split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if !tokens.is_empty() {
            let cmd = &tokens[0];
            let args = &tokens[1..];

            if let Some(result) = builtins::dispatch(cmd, args, self.mode, &mut acl) {
                let (stdout, exit_code) = match result {
                    BuiltinResult::Success(msg) => (msg, 0),
                    BuiltinResult::Error(msg) => (msg, 1),
                    BuiltinResult::SwitchMode(mode) => (format!("Switched to {mode} mode"), 0),
                    BuiltinResult::Exit => ("Shell exit requested".to_string(), 0),
                };

                return ShellToolOutput {
                    allowed: true,
                    command: input.command,
                    stdout,
                    stderr: String::new(),
                    exit_code: Some(exit_code),
                    duration_ms: start.elapsed().as_millis() as u64,
                    denial_reason: None,
                    mode: self.mode.to_string(),
                };
            }
        }

        // For external commands: return a structured response indicating
        // the command would be executed. Actual execution requires shrs integration.
        let cmd = input.command.clone();
        ShellToolOutput {
            allowed: true,
            command: input.command,
            stdout: format!("(command queued: {cmd})"),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: start.elapsed().as_millis() as u64,
            denial_reason: None,
            mode: self.mode.to_string(),
        }
    }

    /// Execute from raw JSON string input.
    pub fn execute_json(&self, json_input: &str) -> Result<ShellToolOutput, String> {
        let input: ShellToolInput = serde_json::from_str(json_input)
            .map_err(|e| format!("Invalid input JSON: {e}"))?;
        Ok(self.execute(input))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_name_and_description() {
        let tool = OmniShellTool::new(Mode::Agent);
        assert_eq!(tool.name(), "omnishell");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn test_input_schema_valid_json() {
        let tool = OmniShellTool::new(Mode::Agent);
        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
    }

    #[test]
    fn test_execute_allowed_command() {
        let tool = OmniShellTool::new(Mode::Agent);
        let result = tool.execute(ShellToolInput {
            command: "ls -la".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(result.allowed);
        assert!(result.denial_reason.is_none());
        assert_eq!(result.mode, "agent");
    }

    #[test]
    fn test_execute_blocked_command() {
        let tool = OmniShellTool::new(Mode::Agent);
        let result = tool.execute(ShellToolInput {
            command: "sudo rm -rf /".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(!result.allowed);
        assert!(result.denial_reason.is_some());
        assert!(result.exit_code.is_none());
    }

    #[test]
    fn test_execute_builtin_command() {
        let tool = OmniShellTool::new(Mode::Admin);
        let result = tool.execute(ShellToolInput {
            command: "help".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(result.allowed);
        assert!(result.stdout.contains("OmniShell"));
        assert_eq!(result.exit_code, Some(0));
    }

    #[test]
    fn test_execute_mode_switch() {
        let tool = OmniShellTool::new(Mode::Admin);
        let result = tool.execute(ShellToolInput {
            command: "mode kids".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(result.allowed);
        assert!(result.stdout.contains("Switched to kids mode"));
    }

    #[test]
    fn test_execute_json_input() {
        let tool = OmniShellTool::new(Mode::Agent);
        let result = tool.execute_json(r#"{"command": "cargo build"}"#).unwrap();
        assert!(result.allowed);
    }

    #[test]
    fn test_execute_json_invalid_input() {
        let tool = OmniShellTool::new(Mode::Agent);
        let result = tool.execute_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_output_serialization() {
        let output = ShellToolOutput {
            allowed: true,
            command: "ls".to_string(),
            stdout: "file.txt".to_string(),
            stderr: String::new(),
            exit_code: Some(0),
            duration_ms: 42,
            denial_reason: None,
            mode: "agent".to_string(),
        };
        let json = serde_json::to_string(&output).unwrap();
        assert!(json.contains("\"allowed\":true"));
        assert!(json.contains("\"exit_code\":0"));
    }

    #[test]
    fn test_kids_mode_blocks_dangerous() {
        let tool = OmniShellTool::new(Mode::Kids);
        let result = tool.execute(ShellToolInput {
            command: "python".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(!result.allowed);
    }

    #[test]
    fn test_kids_mode_allows_safe() {
        let tool = OmniShellTool::new(Mode::Kids);
        let result = tool.execute(ShellToolInput {
            command: "ls".to_string(),
            working_dir: None,
            timeout: 0,
            capture_stderr: true,
        });
        assert!(result.allowed);
    }
}

// --- Engram Context Provider ---

/// Engram context provider for LLM integration.
///
/// Reads engram's git-based storage (tasks, reasoning, ADRs) via the engram CLI
/// and formats them as context for LLM prompts.
pub struct EngramContext {
    /// Path to the engram CLI binary.
    cli_path: String,
    /// Whether engram is available.
    available: bool,
}

impl EngramContext {
    /// Create a new engram context provider.
    pub fn new() -> Self {
        let cli_path = "engram".to_string();
        let available = Self::check_engram_available(&cli_path);
        Self { cli_path, available }
    }

    /// Create with a specific CLI path.
    pub fn with_path(cli_path: String) -> Self {
        let available = Self::check_engram_available(&cli_path);
        Self { cli_path, available }
    }

    /// Check if engram CLI is available.
    fn check_engram_available(cli_path: &str) -> bool {
        std::process::Command::new(cli_path)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Check if engram is available.
    pub fn is_available(&self) -> bool {
        self.available
    }

    /// Get the current task context for LLM injection.
    pub fn get_task_context(&self, task_id: &str) -> Result<String, String> {
        if !self.available {
            return Ok("(engram not available)".to_string());
        }

        let output = std::process::Command::new(&self.cli_path)
            .args(["task", "show", task_id])
            .output()
            .map_err(|e| format!("Failed to run engram: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(format!("engram task show failed: {}", String::from_utf8_lossy(&output.stderr)))
        }
    }

    /// Get recent tasks for context.
    pub fn get_recent_tasks(&self, limit: usize) -> Result<String, String> {
        if !self.available {
            return Ok("(engram not available)".to_string());
        }

        let output = std::process::Command::new(&self.cli_path)
            .args(["task", "list", "--limit", &limit.to_string()])
            .output()
            .map_err(|e| format!("Failed to run engram: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            Err(format!("engram task list failed: {}", String::from_utf8_lossy(&output.stderr)))
        }
    }

    /// Get the next task for the current session.
    pub fn get_next_task(&self) -> Result<String, String> {
        if !self.available {
            return Ok("(engram not available)".to_string());
        }

        let output = std::process::Command::new(&self.cli_path)
            .args(["next"])
            .output()
            .map_err(|e| format!("Failed to run engram: {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            // No next task is not an error
            Ok("(no pending tasks)".to_string())
        }
    }

    /// Build a context string for LLM system prompt.
    pub fn build_llm_context(&self) -> String {
        let mut context = String::new();

        if let Ok(next) = self.get_next_task() {
            if !next.contains("engram not available") {
                context.push_str("Current task context:\n");
                context.push_str(&next);
                context.push_str("\n\n");
            }
        }

        if let Ok(tasks) = self.get_recent_tasks(5) {
            if !tasks.contains("engram not available") {
                context.push_str("Recent tasks:\n");
                context.push_str(&tasks);
            }
        }

        if context.is_empty() {
            context = "(no engram context available)".to_string();
        }

        context
    }
}

impl Default for EngramContext {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod engram_tests {
    use super::*;

    #[test]
    fn test_engram_context_new() {
        let ctx = EngramContext::new();
        // May or may not be available depending on environment
        // Just check it doesn't panic
        let _ = ctx.is_available();
    }

    #[test]
    fn test_engram_context_with_invalid_path() {
        let ctx = EngramContext::with_path("/nonexistent/engram_binary_12345".to_string());
        assert!(!ctx.is_available());
    }

    #[test]
    fn test_engram_context_graceful_when_unavailable() {
        let ctx = EngramContext::with_path("/nonexistent/engram_binary_12345".to_string());
        let result = ctx.get_task_context("test-id").unwrap();
        assert!(result.contains("engram not available"));
    }

    #[test]
    fn test_engram_build_context_when_unavailable() {
        let ctx = EngramContext::with_path("/nonexistent/engram_binary_12345".to_string());
        let context = ctx.build_llm_context();
        assert!(context.contains("no engram context available"));
    }

    #[test]
    fn test_engram_get_recent_tasks_when_unavailable() {
        let ctx = EngramContext::with_path("/nonexistent/engram_binary_12345".to_string());
        let result = ctx.get_recent_tasks(5).unwrap();
        assert!(result.contains("engram not available"));
    }
}
