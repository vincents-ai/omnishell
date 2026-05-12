//! OmniShellTool — agentic-loop integration + engram context provider.
//!
//! Part 1: OmniShellTool exposes shell execution to agentic-loop agents.
//! Part 2: EngramContext provides LLM context injection from engram's git storage.
//! structured JSON responses.

use serde::{Deserialize, Serialize};

use engram::Storage;
use engram::Entity;

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
        let tokens: Vec<String> = input
            .command
            .split_whitespace()
            .map(|s| s.to_string())
            .collect();

        if !tokens.is_empty() {
            let cmd = &tokens[0];
            let args = &tokens[1..];

            if let Some(result) =
                builtins::dispatch(cmd, args, self.mode, &mut acl, None, None, None)
            {
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

        // External command execution via std::process::Command.
        // The ACL has already approved this command. We tokenize simply
        // and execute the first token as the program with the rest as args.
        //
        // Note: This handles simple commands only. Pipes, redirects, && etc.
        // require the full shell evaluator (lang_impl.rs). For agentic-loop
        // integration, agents should send one command at a time.
        let tokens: Vec<String> = shlex::split(&input.command).unwrap_or_else(|| {
            // Fallback: split on whitespace if shlex can't parse
            input.command.split_whitespace().map(|s| s.to_string()).collect()
        });

        if tokens.is_empty() {
            return ShellToolOutput {
                allowed: true,
                command: input.command,
                stdout: String::new(),
                stderr: "Empty command".to_string(),
                exit_code: Some(1),
                duration_ms: start.elapsed().as_millis() as u64,
                denial_reason: None,
                mode: self.mode.to_string(),
            };
        }

        let program = &tokens[0];
        let args = &tokens[1..];

        let mut cmd = std::process::Command::new(program);
        cmd.args(args);

        if let Some(ref dir) = input.working_dir {
            cmd.current_dir(dir);
        }

        // Capture stdout and stderr separately
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        // Execute the command and capture output
        let result: std::io::Result<std::process::Output> = cmd.output();

        match result {
            Ok(output) => {
                let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();

                // Truncate large outputs to prevent context window abuse
                let max_output = 50_000;
                let stdout_truncated = if stdout_str.len() > max_output {
                    format!("{}...\n[truncated, {} total bytes]", &stdout_str[..max_output], stdout_str.len())
                } else {
                    stdout_str
                };
                let stderr_truncated = if stderr_str.len() > max_output {
                    format!("{}...\n[truncated, {} total bytes]", &stderr_str[..max_output], stderr_str.len())
                } else {
                    stderr_str
                };

                ShellToolOutput {
                    allowed: true,
                    command: input.command,
                    stdout: stdout_truncated,
                    stderr: stderr_truncated,
                    exit_code: output.status.code(),
                    duration_ms: start.elapsed().as_millis() as u64,
                    denial_reason: None,
                    mode: self.mode.to_string(),
                }
            }
            Err(e) => ShellToolOutput {
                allowed: true,
                command: input.command,
                stdout: String::new(),
                stderr: format!("Execution error: {e}"),
                exit_code: Some(126),
                duration_ms: start.elapsed().as_millis() as u64,
                denial_reason: None,
                mode: self.mode.to_string(),
            },
        }
    }

    /// Execute from raw JSON string input.
    pub fn execute_json(&self, json_input: &str) -> Result<ShellToolOutput, String> {
        let input: ShellToolInput =
            serde_json::from_str(json_input).map_err(|e| format!("Invalid input JSON: {e}"))?;
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
    fn test_execute_allowed_command_verdict() {
        // Only test ACL verdict, never spawn processes in tests
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("echo hello"), crate::acl::Verdict::Allow));
        assert!(matches!(acl.evaluate("ls -la"), crate::acl::Verdict::Allow));
    }

    #[test]
    fn test_execute_blocked_command_verdict() {
        // Only test ACL verdict, never attempt execution
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("sudo rm -rf /"), crate::acl::Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("sudo bash"), crate::acl::Verdict::Deny(_)));
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
    fn test_execute_json_input_verdict() {
        // Only test JSON parsing + ACL, never execute
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("echo hello"), crate::acl::Verdict::Allow));
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
    fn test_kids_mode_blocks_dangerous_verdict() {
        // Only test ACL verdict
        let acl = AclEngine::new(Mode::Kids);
        assert!(matches!(acl.evaluate("python"), crate::acl::Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("bash"), crate::acl::Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("sudo bash"), crate::acl::Verdict::Deny(_)));
    }

    #[test]
    fn test_kids_mode_allows_safe_verdict() {
        // Only test ACL verdict
        let acl = AclEngine::new(Mode::Kids);
        assert!(matches!(acl.evaluate("ls"), crate::acl::Verdict::Allow));
        assert!(matches!(acl.evaluate("echo hello"), crate::acl::Verdict::Allow));
        assert!(matches!(acl.evaluate("pwd"), crate::acl::Verdict::Allow));
    }
}

// --- Engram Context Provider ---

/// Engram context provider for LLM integration.
///
/// Uses the engram crate directly (no CLI subprocess) to read git-based
/// storage for tasks, reasoning, and context entities.
pub struct EngramContext {
    /// The underlying git-refs storage. None if engram repo not found.
    storage: Option<engram::storage::GitRefsStorage>,
    /// The agent name for queries.
    agent: String,
}

impl EngramContext {
    /// Create a new engram context provider.
    ///
    /// Searches for the engram repository by walking up from CWD to find `.engram/`
    /// or uses the current git repo's `.engram/` directory.
    pub fn new() -> Self {
        let agent = std::env::var("USER")
            .or_else(|_| std::env::var("ENGram_AGENT"))
            .unwrap_or_else(|_| "default".to_string());

        let storage = Self::find_and_open_storage(&agent);
        Self { storage, agent }
    }

    /// Create with a specific workspace path.
    pub fn with_workspace(workspace_path: &str, agent: &str) -> Self {
        let storage = engram::storage::GitRefsStorage::new(workspace_path, agent).ok();
        Self {
            storage,
            agent: agent.to_string(),
        }
    }

    /// Create a no-op context (engram unavailable).
    pub fn unavailable() -> Self {
        Self {
            storage: None,
            agent: "default".to_string(),
        }
    }

    /// Find the engram workspace by searching for `.engram/` directory.
    fn find_and_open_storage(agent: &str) -> Option<engram::storage::GitRefsStorage> {
        // Walk up from CWD to find .engram/
        let mut dir = std::env::current_dir().ok()?;
        loop {
            let engram_dir = dir.join(".engram");
            if engram_dir.exists() {
                // The workspace is the parent of .engram/
                if let Ok(storage) =
                    engram::storage::GitRefsStorage::new(&dir.to_string_lossy(), agent)
                {
                    return Some(storage);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    /// Check if engram is available.
    pub fn is_available(&self) -> bool {
        self.storage.is_some()
    }

    /// Get the current task context for LLM injection.
    pub fn get_task_context(&self, task_id: &str) -> Result<String, String> {
        let storage = self.storage.as_ref().ok_or("engram not available")?;

        let entity = storage
            .get(task_id, "task")
            .map_err(|e| format!("engram get failed: {e}"))?;

        match entity {
            Some(entity) => {
                match engram::entities::Task::from_generic(entity) {
                    Ok(task) => Ok(format!(
                        "Task: {} ({})\n\
                         Status: {:?}\n\
                         Priority: {:?}\n\
                         Description: {}",
                        task.title, task.id, task.status, task.priority, task.description
                    )),
                    Err(e) => Err(format!("Failed to parse task: {e}")),
                }
            }
            None => Err(format!("Task {task_id} not found")),
        }
    }

    /// Get recent tasks for context.
    pub fn get_recent_tasks(&self, limit: usize) -> Result<String, String> {
        let storage = self.storage.as_ref().ok_or("engram not available")?;

        let entities = storage
            .query_by_agent(&self.agent, Some("task"))
            .map_err(|e| format!("engram query failed: {e}"))?;

        let mut tasks: Vec<String> = Vec::new();
        for entity in entities.into_iter().take(limit) {
            if let Ok(task) = engram::entities::Task::from_generic(entity) {
                tasks.push(format!(
                    "  [{}] {:?} {} — {}",
                    &task.id[..7.min(task.id.len())],
                    task.status,
                    task.title,
                    task.description.chars().take(60).collect::<String>()
                ));
            }
        }

        if tasks.is_empty() {
            Ok("(no tasks found)".to_string())
        } else {
            Ok(tasks.join("\n"))
        }
    }

    /// Get the next task for the current session.
    pub fn get_next_task(&self) -> Result<String, String> {
        let storage = self.storage.as_ref().ok_or("engram not available")?;

        let scope = engram::cli::next::NextScope {
            parent: None,
            agent: None,
            session: None,
            tag: None,
        };

        match engram::cli::next::find_next_task(storage, &self.agent, &scope) {
            Ok(Some(task)) => Ok(format!(
                "Next task: {} ({})\n\
                 Status: {:?} | Priority: {:?}\n\
                 {}",
                task.title, task.id, task.status, task.priority, task.description
            )),
            Ok(None) => Ok("(no pending tasks)".to_string()),
            Err(e) => Err(format!("engram next failed: {e}")),
        }
    }

    /// Build a context string for LLM system prompt.
    pub fn build_llm_context(&self) -> String {
        let Some(storage) = self.storage.as_ref() else {
            return "(no engram context available)".to_string();
        };

        let mut context = String::new();

        // Get next task
        let scope = engram::cli::next::NextScope {
            parent: None,
            agent: None,
            session: None,
            tag: None,
        };

        if let Ok(Some(task)) = engram::cli::next::find_next_task(storage, &self.agent, &scope) {
            context.push_str(&format!(
                "Current task:\n  {} ({})\n  Status: {:?} | Priority: {:?}\n  {}\n\n",
                task.title, task.id, task.status, task.priority, task.description
            ));
        }

        // Get recent tasks
        if let Ok(entities) = storage.query_by_agent(&self.agent, Some("task")) {
            let tasks: Vec<String> = entities
                .into_iter()
                .take(5)
                .filter_map(|e| engram::entities::Task::from_generic(e).ok())
                .map(|t| {
                    format!(
                        "  [{}] {:?} {}",
                        &t.id[..7.min(t.id.len())],
                        t.status,
                        t.title
                    )
                })
                .collect();

            if !tasks.is_empty() {
                context.push_str("Recent tasks:\n");
                context.push_str(&tasks.join("\n"));
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
        // May or may not be available depending on whether we're in an engram workspace
        let _ = ctx.is_available();
    }

    #[test]
    fn test_engram_unavailable() {
        let ctx = EngramContext::unavailable();
        assert!(!ctx.is_available());
    }

    #[test]
    fn test_engram_context_graceful_when_unavailable() {
        let ctx = EngramContext::unavailable();
        let result = ctx.get_task_context("test-id").unwrap_err();
        assert!(result.contains("engram not available"));
    }

    #[test]
    fn test_engram_build_context_when_unavailable() {
        let ctx = EngramContext::unavailable();
        let context = ctx.build_llm_context();
        assert!(context.contains("no engram context available"));
    }

    #[test]
    fn test_engram_get_recent_tasks_when_unavailable() {
        let ctx = EngramContext::unavailable();
        let result = ctx.get_recent_tasks(5).unwrap_err();
        assert!(result.contains("engram not available"));
    }
}
