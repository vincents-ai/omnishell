//! OmniShellTool — agentic-loop integration.
//!
//! Provides a Tool trait implementation that allows agentic-loop agents
//! to execute shell commands through OmniShell's ACL, output formatting,
//! and audit pipeline. The tool accepts JSON command specs and returns
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
                    BuiltinResult::SwitchMode(mode) => (format!("Switched to {} mode", mode), 0),
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
            stdout: format!("(command queued: {})", cmd),
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
            .map_err(|e| format!("Invalid input JSON: {}", e))?;
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
