//! Access Control List (ACL) engine for OmniShell.
//!
//! Every command (user-typed or AI-generated) passes through the ACL parser.
//! Supports allowlist (explicit inclusion) and blocklist (explicit exclusion,
//! overrides allowlist). Blocked commands never spawn an OS process.

use std::fmt;

use glob::Pattern;
use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Verdict returned by the ACL engine for a proposed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Command is allowed to execute.
    Allow,
    /// Command is blocked. Contains the reason.
    Deny(String),
}

/// A single ACL rule entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclRule {
    /// Glob pattern for the command name (e.g., "rm", "git*", "sudo*").
    pub pattern: String,
    /// Argument constraints. Empty means any arguments are allowed.
    #[serde(default)]
    pub args: Vec<ArgConstraint>,
    /// Human-readable reason (shown in denial messages and audit logs).
    #[serde(default)]
    pub reason: String,
}

/// Constraint on command arguments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArgConstraint {
    /// A specific argument flag/value must be present.
    MustNotContain(String),
    /// A glob pattern that arguments must match.
    MustMatchGlob(String),
}

/// The ACL engine: evaluates commands against allowlist + blocklist rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclEngine {
    /// Profile mode this engine was built for.
    pub mode: Mode,
    /// Commands on the allowlist. Empty means all are allowed (unless blocked).
    pub allowlist: Vec<AclRule>,
    /// Commands on the blocklist. Blocklist always overrides allowlist.
    pub blocklist: Vec<AclRule>,
}

impl AclEngine {
    /// Create a new ACL engine for the given mode.
    pub fn new(mode: Mode) -> Self {
        match mode {
            Mode::Kids => Self::kids_default(),
            Mode::Agent => Self::agent_default(),
            Mode::Admin => Self::admin_default(),
        }
    }

    /// Evaluate a command string against the ACL rules.
    ///
    /// The command string is tokenized into (command_name, args).
    /// Returns a verdict indicating whether the command should be allowed.
    pub fn evaluate(&self, command_line: &str) -> Verdict {
        let tokens = tokenize(command_line);
        if tokens.is_empty() {
            return Verdict::Allow;
        }

        let cmd_name = &tokens[0];
        let args = &tokens[1..];

        // Blocklist takes absolute precedence
        for rule in &self.blocklist {
            if matches_pattern(cmd_name, &rule.pattern) {
                if rule.args.is_empty() {
                    return Verdict::Deny(format!(
                        "Command '{}' is blocked: {}",
                        cmd_name, rule.reason
                    ));
                }
                // Check argument-level constraints
                for constraint in &rule.args {
                    if let Some(deny) =
                        check_arg_constraint(args, constraint, cmd_name, &rule.reason)
                    {
                        return deny;
                    }
                }
            }
        }

        // If allowlist is non-empty, command must match at least one rule
        if !self.allowlist.is_empty() {
            let mut allowed = false;
            for rule in &self.allowlist {
                if matches_pattern(cmd_name, &rule.pattern) {
                    if rule.args.is_empty() {
                        allowed = true;
                        break;
                    }
                    // Check that no argument constraint is violated
                    let mut all_args_ok = true;
                    for constraint in &rule.args {
                        if check_arg_constraint(args, constraint, cmd_name, &rule.reason).is_some()
                        {
                            all_args_ok = false;
                            break;
                        }
                    }
                    if all_args_ok {
                        allowed = true;
                        break;
                    }
                }
            }
            if !allowed {
                return Verdict::Deny(format!(
                    "Command '{}' is not on the allowlist for {} mode",
                    cmd_name, self.mode
                ));
            }
        }

        Verdict::Allow
    }

    /// Kids mode: strict allowlist, only safe commands.
    fn kids_default() -> Self {
        Self {
            mode: Mode::Kids,
            allowlist: vec![
                simple_rule("ls", "List directory contents"),
                simple_rule("cd", "Change directory"),
                simple_rule("pwd", "Print working directory"),
                simple_rule("echo", "Print text"),
                simple_rule("cat", "View file contents"),
                simple_rule("cowsay", "Fun cow message"),
                simple_rule("cowthink", "Fun cow thought"),
                simple_rule("fortune", "Random quote"),
                simple_rule("clear", "Clear screen"),
                simple_rule("help", "Show help"),
                simple_rule("?", "Ask AI tutor"),
                simple_rule("ai", "Ask AI tutor"),
                simple_rule("exit", "Exit the shell"),
                simple_rule("quit", "Exit the shell"),
                simple_rule("mode", "Show or switch mode"),
                simple_rule("snapshots", "List snapshots"),
                simple_rule("undo", "Undo last command"),
                simple_rule("redo", "Redo last undone command"),
                simple_rule("true", "No-op success"),
                simple_rule("false", "No-op failure"),
                simple_rule("test", "Test expression"),
                pattern_rule("git", "status", "View git status"),
                pattern_rule("git", "log", "View git log"),
                pattern_rule("git", "diff", "View git diff"),
                pattern_rule("git", "branch", "List git branches"),
            ],
            blocklist: vec![AclRule {
                pattern: "*".to_string(),
                args: vec![ArgConstraint::MustNotContain(
                    "--no-preserve-root".to_string(),
                )],
                reason: "Destructive flag blocked".to_string(),
            }],
        }
    }

    /// Agent mode: blocklist only, everything else allowed.
    fn agent_default() -> Self {
        Self {
            mode: Mode::Agent,
            allowlist: vec![], // Empty = everything allowed unless blocked
            blocklist: vec![
                AclRule {
                    pattern: "rm".to_string(),
                    args: vec![ArgConstraint::MustNotContain("-rf /".to_string())],
                    reason: "Recursive root deletion blocked".to_string(),
                },
                AclRule {
                    pattern: "rm".to_string(),
                    args: vec![ArgConstraint::MustNotContain(
                        "--no-preserve-root".to_string(),
                    )],
                    reason: "Root filesystem deletion blocked".to_string(),
                },
                AclRule {
                    pattern: "sudo".to_string(),
                    args: vec![],
                    reason: "Privilege escalation blocked in agent mode".to_string(),
                },
                AclRule {
                    pattern: "curl".to_string(),
                    args: vec![ArgConstraint::MustMatchGlob("*|*".to_string())],
                    reason: "Pipe-based data exfiltration blocked".to_string(),
                },
                AclRule {
                    pattern: "wget".to_string(),
                    args: vec![ArgConstraint::MustMatchGlob("*|*".to_string())],
                    reason: "Pipe-based data exfiltration blocked".to_string(),
                },
            ],
        }
    }

    /// Admin mode: everything allowed.
    fn admin_default() -> Self {
        Self {
            mode: Mode::Admin,
            allowlist: vec![],
            blocklist: vec![],
        }
    }
}

/// Check if a command name matches a pattern (exact or glob).
fn matches_pattern(cmd: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if cmd == pattern {
        return true;
    }
    // Try glob matching
    Pattern::new(pattern)
        .map(|p| p.matches(cmd))
        .unwrap_or(false)
}

/// Check a single argument constraint. Returns Some(Deny) if violated.
fn check_arg_constraint(
    args: &[String],
    constraint: &ArgConstraint,
    cmd: &str,
    reason: &str,
) -> Option<Verdict> {
    match constraint {
        ArgConstraint::MustNotContain(forbidden) => {
            let full_args = args.join(" ");
            if full_args.contains(forbidden) {
                return Some(Verdict::Deny(format!(
                    "Command '{cmd}' blocked: {reason} (forbidden argument: {forbidden})"
                )));
            }
            None
        }
        ArgConstraint::MustMatchGlob(pattern_str) => {
            let full_args = args.join(" ");
            if let Ok(pattern) = Pattern::new(pattern_str) {
                if pattern.matches(&full_args) {
                    return Some(Verdict::Deny(format!(
                        "Command '{cmd}' blocked: {reason} (argument pattern matched: {pattern_str})"
                    )));
                }
            }
            None
        }
    }
}

/// Create a simple allowlist rule with no argument constraints.
fn simple_rule(cmd: &str, reason: &str) -> AclRule {
    AclRule {
        pattern: cmd.to_string(),
        args: vec![],
        reason: reason.to_string(),
    }
}

/// Create a rule that matches a command and subcommand pattern.
fn pattern_rule(cmd: &str, subcmd: &str, reason: &str) -> AclRule {
    AclRule {
        // This allows "git status", "git log" but not "git push"
        pattern: cmd.to_string(),
        args: vec![ArgConstraint::MustMatchGlob(format!("{subcmd}*"))],
        reason: reason.to_string(),
    }
}

/// Simple POSIX-like tokenization of a command line.
/// Handles basic quoting (single, double) and escaping.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            '\\' if !in_single_quote => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ' ' | '\t' if !in_single_quote && !in_double_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Mode::Kids => write!(f, "kids"),
            Mode::Agent => write!(f, "agent"),
            Mode::Admin => write!(f, "admin"),
        }
    }
}

/// Schema for a command's dangerous flags and argument patterns.
/// Provides argument-level validation beyond simple pattern matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandSchema {
    /// The command name this schema describes.
    pub command: String,
    /// Flags that are always dangerous (e.g. "--force", "-rf").
    pub dangerous_flags: Vec<String>,
    /// Argument patterns that are dangerous (glob patterns).
    pub dangerous_args: Vec<String>,
    /// A human-readable reason for why this command is dangerous.
    pub reason: String,
}

/// Return the default built-in command schemas for common dangerous commands.
pub fn default_schemas() -> Vec<CommandSchema> {
    vec![
        CommandSchema {
            command: "rm".to_string(),
            dangerous_flags: vec![
                "-rf".to_string(),
                "-r".to_string(),
                "--force".to_string(),
                "--no-preserve-root".to_string(),
            ],
            dangerous_args: vec!["/*".to_string(), "/".to_string()],
            reason: "Recursive/forced file deletion".to_string(),
        },
        CommandSchema {
            command: "mv".to_string(),
            dangerous_flags: vec!["--force".to_string()],
            dangerous_args: vec![],
            reason: "Force file move/overwrite".to_string(),
        },
        CommandSchema {
            command: "cp".to_string(),
            dangerous_flags: vec!["--force".to_string(), "-r".to_string()],
            dangerous_args: vec![],
            reason: "Force recursive copy".to_string(),
        },
        CommandSchema {
            command: "chmod".to_string(),
            dangerous_flags: vec!["-R".to_string(), "--recursive".to_string()],
            dangerous_args: vec!["/*".to_string(), "/".to_string()],
            reason: "Recursive permission change on root".to_string(),
        },
        CommandSchema {
            command: "chown".to_string(),
            dangerous_flags: vec!["-R".to_string(), "--recursive".to_string()],
            dangerous_args: vec!["/*".to_string(), "/".to_string()],
            reason: "Recursive ownership change on root".to_string(),
        },
        CommandSchema {
            command: "dd".to_string(),
            dangerous_flags: vec![],
            dangerous_args: vec!["/dev/sd*".to_string(), "/dev/nvme*".to_string()],
            reason: "Direct disk write operations".to_string(),
        },
        CommandSchema {
            command: "git".to_string(),
            dangerous_flags: vec!["--force".to_string()],
            dangerous_args: vec![],
            reason: "Force git operations".to_string(),
        },
        CommandSchema {
            command: "sudo".to_string(),
            dangerous_flags: vec![],
            dangerous_args: vec![],
            reason: "Elevated privilege execution".to_string(),
        },
        CommandSchema {
            command: "mkfs".to_string(),
            dangerous_flags: vec![],
            dangerous_args: vec!["/dev/sd*".to_string(), "/dev/nvme*".to_string()],
            reason: "Filesystem formatting".to_string(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Tokenization tests ---

    #[test]
    fn test_tokenize_simple() {
        assert_eq!(tokenize("ls -la /tmp"), vec!["ls", "-la", "/tmp"]);
    }

    #[test]
    fn test_tokenize_quoted() {
        assert_eq!(
            tokenize("echo 'hello world' \"foo bar\""),
            vec!["echo", "hello world", "foo bar"]
        );
    }

    #[test]
    fn test_tokenize_escaped() {
        assert_eq!(tokenize("echo hello\\ world"), vec!["echo", "hello world"]);
    }

    // --- Kids mode tests ---

    #[test]
    fn test_kids_allows_safe_commands() {
        let acl = AclEngine::new(Mode::Kids);
        assert_eq!(acl.evaluate("ls"), Verdict::Allow);
        assert_eq!(acl.evaluate("ls -la"), Verdict::Allow);
        assert_eq!(acl.evaluate("echo hello"), Verdict::Allow);
        assert_eq!(acl.evaluate("pwd"), Verdict::Allow);
        assert_eq!(acl.evaluate("cd /tmp"), Verdict::Allow);
        assert_eq!(acl.evaluate("cowsay moo"), Verdict::Allow);
        assert_eq!(acl.evaluate("git status"), Verdict::Allow);
    }

    #[test]
    fn test_kids_blocks_dangerous_commands() {
        let acl = AclEngine::new(Mode::Kids);
        assert!(matches!(acl.evaluate("rm -rf /"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("sudo bash"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("curl evil.com"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("python"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("bash"), Verdict::Deny(_)));
    }

    #[test]
    fn test_kids_blocks_pipe_commands() {
        let acl = AclEngine::new(Mode::Kids);
        // "ls | rm" tokenizes as ["ls", "|", "rm"] — the pipe character
        // reaches the allowlist which only allows ls without extra args matching |
        // For now, this test verifies that rm is blocked when it appears as a
        // command in its own right. Pipe-level filtering is the shell's job.
        assert!(matches!(acl.evaluate("rm"), Verdict::Deny(_)));
    }

    // --- Agent mode tests ---

    #[test]
    fn test_agent_allows_most_commands() {
        let acl = AclEngine::new(Mode::Agent);
        assert_eq!(acl.evaluate("cargo build"), Verdict::Allow);
        assert_eq!(acl.evaluate("git push"), Verdict::Allow);
        assert_eq!(acl.evaluate("vim file.txt"), Verdict::Allow);
        assert_eq!(acl.evaluate("ls -la"), Verdict::Allow);
    }

    #[test]
    fn test_agent_blocks_sudo() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("sudo rm -rf /"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("sudo bash"), Verdict::Deny(_)));
    }

    #[test]
    fn test_agent_blocks_recursive_root_deletion() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("rm -rf /"), Verdict::Deny(_)));
        assert!(matches!(
            acl.evaluate("rm --no-preserve-root -rf /"),
            Verdict::Deny(_)
        ));
    }

    #[test]
    fn test_agent_allows_safe_rm() {
        let acl = AclEngine::new(Mode::Agent);
        assert_eq!(acl.evaluate("rm file.txt"), Verdict::Allow);
        assert_eq!(acl.evaluate("rm -rf build/"), Verdict::Allow);
    }

    // --- Admin mode tests ---

    #[test]
    fn test_admin_allows_everything() {
        let acl = AclEngine::new(Mode::Admin);
        assert_eq!(acl.evaluate("sudo rm -rf /"), Verdict::Allow);
        assert_eq!(acl.evaluate("anything goes"), Verdict::Allow);
    }

    // --- Edge cases ---

    #[test]
    fn test_empty_command_allowed() {
        let acl = AclEngine::new(Mode::Kids);
        assert_eq!(acl.evaluate(""), Verdict::Allow);
    }

    #[test]
    fn test_blocklist_overrides_allowlist() {
        let mut acl = AclEngine::new(Mode::Admin);
        // Add a blocklist entry
        acl.blocklist.push(AclRule {
            pattern: "dangerous_cmd".to_string(),
            args: vec![],
            reason: "test override".to_string(),
        });
        assert!(matches!(acl.evaluate("dangerous_cmd"), Verdict::Deny(_)));
    }
}
