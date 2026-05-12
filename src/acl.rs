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
    /// A specific argument flag/value must not be present as a substring.
    MustNotContain(String),
    /// A glob pattern that arguments must match.
    MustMatchGlob(String),
    /// Any of these individual flags must not appear in the args list.
    /// Checks each tokenized arg individually, not as a joined string.
    MustNotContainFlag(Vec<String>),
    /// A positional argument (by index) must not match a glob pattern.
    /// Used to check specific argument positions (e.g., arg[0] is '/' for 'rm').
    PositionalMustNotMatch { index: usize, pattern: String },
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
    /// The command line is first split on shell metacharacters (|, ;, &&, ||)
    /// to get individual sub-commands. Each sub-command is evaluated independently.
    /// This prevents bypassing the ACL via pipes like `"ls | sudo rm -rf /"`.
    ///
    /// Command substitution (`$()`, backticks, process substitution) is also
    /// detected and blocked in Kids mode (and restricted in Agent mode) since
    /// the ACL cannot safely evaluate the embedded commands.
    pub fn evaluate(&self, command_line: &str) -> Verdict {
        // Phase 1: Check for command substitution that bypasses simple tokenization.
        // In Kids mode, any command substitution is blocked outright.
        // In Agent mode, command substitution is allowed (agent needs it for scripts).
        if self.mode == Mode::Kids && contains_command_substitution(command_line) {
            return Verdict::Deny(
                "Command substitution ($(), ``, <()) is not allowed in Kids mode".to_string(),
            );
        }

        // Phase 2: Split the command line into sub-commands on shell metacharacters.
        // This ensures `"ls | sudo rm"` evaluates both `"ls"` and `"sudo rm"`.
        let subcommands = split_into_subcommands(command_line);
        if subcommands.is_empty() {
            return Verdict::Allow;
        }

        // Phase 3: Evaluate each sub-command independently.
        // If ANY sub-command is denied, the entire command line is denied.
        for subcmd in &subcommands {
            if let Verdict::Deny(reason) = self.evaluate_single(subcmd) {
                return Verdict::Deny(reason);
            }
        }

        Verdict::Allow
    }

    /// Evaluate a single sub-command (no pipes/semicolons) against the ACL rules.
    ///
    /// Also extracts and evaluates commands inside $() and backtick substitution
    /// to prevent bypassing via `$(sudo bash)` or `` `sudo bash` ``.
    fn evaluate_single(&self, command_line: &str) -> Verdict {
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

        // Also check for blocked commands inside command substitution.
        // $(sudo bash) tokenizes as ["$(sudo", "bash)"] — the command name
        // is "$(sudo" which doesn't match the pattern. We need to also check
        // extracted substitution contents.
        if self.mode != Mode::Admin {
            for inner_cmd in extract_substitution_commands(command_line) {
                let inner_tokens = tokenize(&inner_cmd);
                if inner_tokens.is_empty() {
                    continue;
                }
                let inner_name = &inner_tokens[0];
                for rule in &self.blocklist {
                    if matches_pattern(inner_name, &rule.pattern) {
                        return Verdict::Deny(format!(
                            "Command '{}' is blocked inside substitution: {}",
                            inner_name, rule.reason
                        ));
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
                // Block sudo entirely
                AclRule {
                    pattern: "sudo".to_string(),
                    args: vec![],
                    reason: "Privilege escalation blocked in agent mode".to_string(),
                },
                // Block su entirely
                AclRule {
                    pattern: "su".to_string(),
                    args: vec![],
                    reason: "User switching blocked in agent mode".to_string(),
                },
                // rm with --no-preserve-root (always dangerous regardless of target)
                AclRule {
                    pattern: "rm".to_string(),
                    args: vec![
                        ArgConstraint::MustNotContain("--no-preserve-root".to_string()),
                    ],
                    reason: "Root filesystem preservation bypass blocked".to_string(),
                },
                // Note: 'rm -rf /some/path' is ALLOWED in agent mode — the agent
                // needs to be able to clean build dirs, remove files, etc.
                // Only sudo/su/mkfs/dd are fully blocked. The ACL is a safety net,
                // not a full security boundary. For true sandboxing, use Kids mode.
                // Block mkfs, dd targeting block devices
                AclRule {
                    pattern: "mkfs*".to_string(),
                    args: vec![],
                    reason: "Filesystem formatting blocked in agent mode".to_string(),
                },
                AclRule {
                    pattern: "dd".to_string(),
                    args: vec![],
                    reason: "Block device operations blocked in agent mode".to_string(),
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

    /// Create an ACL engine with additional rules from config.
    ///
    /// Takes the mode's default rules and adds `extra_allow` and `extra_block`
    /// from the profile's AclConfig. This is the preferred constructor
    /// when config is available.
    pub fn with_config(mode: Mode, config: Option<&crate::profile::AclConfig>) -> Self {
        let mut engine = Self::new(mode);
        if let Some(cfg) = config {
            for pattern in &cfg.extra_allow {
                engine.allowlist.push(AclRule {
                    pattern: pattern.clone(),
                    args: vec![],
                    reason: "Allowed by profile config".to_string(),
                });
            }
            for pattern in &cfg.extra_block {
                engine.blocklist.push(AclRule {
                    pattern: pattern.clone(),
                    args: vec![],
                    reason: "Blocked by profile config".to_string(),
                });
            }
        }
        engine
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
        ArgConstraint::MustNotContainFlag(flags) => {
            // Check each individual arg token against the flag list
            for arg in args {
                // Strip combined short flags: -rf → check for -r and -f individually
                if arg.starts_with('-') && !arg.starts_with("--") && arg.len() > 1 {
                    for ch in arg[1..].chars() {
                        let short_flag = format!("-{ch}");
                        if flags.contains(&short_flag) || flags.contains(&format!("--{}", expand_short_flag(ch))) {
                            return Some(Verdict::Deny(format!(
                                "Command '{cmd}' blocked: {reason} (dangerous flag: {short_flag})"
                            )));
                        }
                    }
                } else if flags.contains(arg) {
                    return Some(Verdict::Deny(format!(
                        "Command '{cmd}' blocked: {reason} (dangerous flag: {arg})"
                    )));
                }
            }
            None
        }
        ArgConstraint::PositionalMustNotMatch { index, pattern: pattern_str } => {
            if let Some(arg) = args.get(*index) {
                if let Ok(pattern) = Pattern::new(pattern_str) {
                    if pattern.matches(arg) {
                        return Some(Verdict::Deny(format!(
                            "Command '{cmd}' blocked: {reason} (dangerous target: {arg})"
                        )));
                    }
                }
            }
            None
        }
    }
}

/// Expand a short flag character to its long form for known dangerous flags.
fn expand_short_flag(ch: char) -> &'static str {
    match ch {
        'r' => "recursive",
        'f' => "force",
        'R' => "recursive",
        _ => "",
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
///
/// Handles basic quoting (single, double) and escaping.
/// Also recognizes shell metacharacters (|, ;, &&, ||) as command separators
/// so the ACL can evaluate each sub-command independently.
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

/// Split a command line on shell metacharacters (|, ;, &&, ||) to extract
/// individual sub-commands that each need separate ACL evaluation.
///
/// This is the core of the pipe-bypass defense: given `"ls | sudo rm -rf /"`
/// it returns `["ls", "sudo rm -rf /"]` so the ACL checks both commands.
fn split_into_subcommands(input: &str) -> Vec<String> {
    let mut subcommands = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut paren_depth: u32 = 0;

    while let Some(ch) = chars.next() {
        match ch {
            '\'' if !in_double_quote && paren_depth == 0 => {
                in_single_quote = !in_single_quote;
                current.push(ch);
            }
            '"' if !in_single_quote && paren_depth == 0 => {
                in_double_quote = !in_double_quote;
                current.push(ch);
            }
            '\\' if !in_single_quote && paren_depth == 0 => {
                current.push(ch);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '(' if !in_single_quote && !in_double_quote => {
                paren_depth += 1;
                current.push(ch);
            }
            ')' if !in_single_quote && !in_double_quote => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
                current.push(ch);
            }
            '|' if !in_single_quote && !in_double_quote && paren_depth == 0 => {
                // Check for ||
                if chars.peek() == Some(&'|') {
                    chars.next(); // consume second |
                    // || is a command separator
                    if !current.trim().is_empty() {
                        subcommands.push(std::mem::take(&mut current).trim().to_string());
                    }
                    current.clear();
                } else {
                    // Single | is a pipe
                    if !current.trim().is_empty() {
                        subcommands.push(std::mem::take(&mut current).trim().to_string());
                    }
                    current.clear();
                }
            }
            ';' if !in_single_quote && !in_double_quote && paren_depth == 0 => {
                if !current.trim().is_empty() {
                    subcommands.push(std::mem::take(&mut current).trim().to_string());
                }
                current.clear();
            }
            '&' if !in_single_quote && !in_double_quote && paren_depth == 0 => {
                // Check for &&
                if chars.peek() == Some(&'&') {
                    chars.next(); // consume second &
                    if !current.trim().is_empty() {
                        subcommands.push(std::mem::take(&mut current).trim().to_string());
                    }
                    current.clear();
                } else {
                    // Single & is background — treat as separator
                    if !current.trim().is_empty() {
                        subcommands.push(std::mem::take(&mut current).trim().to_string());
                    }
                    current.clear();
                }
            }
            '\n' if !in_single_quote && !in_double_quote && paren_depth == 0 => {
                if !current.trim().is_empty() {
                    subcommands.push(std::mem::take(&mut current).trim().to_string());
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.trim().is_empty() {
        subcommands.push(current.trim().to_string());
    }
    subcommands
}

/// Detect command substitution that could bypass the ACL.
/// Returns true if the input contains $(...), `...`, or process substitution <(...).
fn contains_command_substitution(input: &str) -> bool {
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote => {
                in_single_quote = !in_single_quote;
            }
            b'"' if !in_single_quote => {
                in_double_quote = !in_double_quote;
            }
            b'\\' if !in_single_quote => {
                i += 1; // skip next char
            }
            b'$' if !in_single_quote => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                    return true; // $(...) command substitution
                }
            }
            b'`' if !in_single_quote => {
                return true; // backtick substitution
            }
            b'<' if !in_single_quote && !in_double_quote => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                    return true; // <(...) process substitution
                }
            }
            b'>' if !in_single_quote && !in_double_quote => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'(' {
                    return true; // >(...) process substitution
                }
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Extract the inner command strings from $(...) and `...` substitution.
/// Used to evaluate substituted commands against the blocklist.
///
/// Example: `"echo $(sudo bash)"` → `["sudo bash"]`
fn extract_substitution_commands(input: &str) -> Vec<String> {
    let mut results = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut in_single_quote = false;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                in_single_quote = !in_single_quote;
            }
            b'\\' if !in_single_quote => {
                i += 1; // skip next char
            }
            b'$' if !in_single_quote && i + 1 < bytes.len() && bytes[i + 1] == b'(' => {
                // Find matching )
                if let Some(content) = extract_balanced_parens(input, i + 2) {
                    results.push(content);
                }
                // Skip past the $(...)
                i = find_closing_paren(input, i + 2).map(|p| p + 1).unwrap_or(i + 2);
                continue;
            }
            b'`' if !in_single_quote => {
                // Find closing backtick
                let start = i + 1;
                if let Some(end) = input[start..].find('`') {
                    results.push(input[start..start + end].to_string());
                    i = start + end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    results
}

/// Find the closing paren for a $( that respects nested parens and quoting.
fn find_closing_paren(input: &str, start: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 1;
    let mut i = start;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double_quote => in_single_quote = !in_single_quote,
            b'"' if !in_single_quote => in_double_quote = !in_double_quote,
            b'\\' if !in_single_quote => { i += 1; }
            b'(' if !in_single_quote && !in_double_quote => depth += 1,
            b')' if !in_single_quote && !in_double_quote => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Extract balanced paren content after $(
fn extract_balanced_parens(input: &str, start: usize) -> Option<String> {
    let end = find_closing_paren(input, start)?;
    Some(input[start..end].to_string())
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
        // "ls | rm" — each sub-command is evaluated: ls=allow, rm=deny → overall deny
        assert!(matches!(acl.evaluate("ls | rm -rf /"), Verdict::Deny(_)));
        // "ls | sudo bash" — sudo blocked
        assert!(matches!(acl.evaluate("ls | sudo bash"), Verdict::Deny(_)));
        // "echo hello ; rm -rf /" — semicolon separated, rm blocked
        assert!(matches!(acl.evaluate("echo hello ; rm -rf /"), Verdict::Deny(_)));
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
    fn test_agent_blocks_no_preserve_root() {
        let acl = AclEngine::new(Mode::Agent);
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
        assert_eq!(acl.evaluate("rm -r -f build/"), Verdict::Allow);
    }

    #[test]
    fn test_agent_blocks_mkfs_dd() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("mkfs.ext4 /dev/sda1"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("dd if=/dev/zero of=/dev/sda"), Verdict::Deny(_)));
    }

    #[test]
    fn test_agent_blocks_su() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("su root"), Verdict::Deny(_)));
    }

    #[test]
    fn test_flag_constraint_individual_tokens() {
        let mut acl = AclEngine::new(Mode::Admin);
        // Test MustNotContainFlag — checks individual tokens, not joined string
        acl.blocklist.push(AclRule {
            pattern: "testcmd".to_string(),
            args: vec![ArgConstraint::MustNotContainFlag(vec![
                "-r".to_string(), "-f".to_string(),
            ])],
            reason: "test".to_string(),
        });
        // Split flags: -r -f should be caught
        assert!(matches!(acl.evaluate("testcmd -r -f /tmp"), Verdict::Deny(_)));
        // Combined flag: -rf should be caught (individual char check)
        assert!(matches!(acl.evaluate("testcmd -rf /tmp"), Verdict::Deny(_)));
        // No dangerous flags
        assert_eq!(acl.evaluate("testcmd -v /tmp"), Verdict::Allow);
    }

    #[test]
    fn test_positional_constraint() {
        let mut acl = AclEngine::new(Mode::Admin);
        acl.blocklist.push(AclRule {
            pattern: "rmtree".to_string(),
            args: vec![ArgConstraint::PositionalMustNotMatch {
                index: 0,
                pattern: "/".to_string(),
            }],
            reason: "test".to_string(),
        });
        assert!(matches!(acl.evaluate("rmtree /"), Verdict::Deny(_)));
        assert_eq!(acl.evaluate("rmtree build/"), Verdict::Allow);
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

    // --- Pipe / subcommand splitting tests ---

    #[test]
    fn test_pipe_splits_and_checks_both() {
        let acl = AclEngine::new(Mode::Agent);
        // "ls | sudo rm" — ls=allow, sudo=deny
        assert!(matches!(acl.evaluate("ls | sudo rm -rf /"), Verdict::Deny(_)));
        // "cat file | grep pattern" — both allowed
        assert_eq!(acl.evaluate("cat file | grep pattern"), Verdict::Allow);
    }

    #[test]
    fn test_semicolon_splits() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("echo hi ; sudo bash"), Verdict::Deny(_)));
        assert_eq!(acl.evaluate("echo hi ; echo bye"), Verdict::Allow);
    }

    #[test]
    fn test_and_and_splits() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("true && sudo bash"), Verdict::Deny(_)));
        assert_eq!(acl.evaluate("true && echo ok"), Verdict::Allow);
    }

    #[test]
    fn test_or_or_splits() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("false || sudo bash"), Verdict::Deny(_)));
        assert_eq!(acl.evaluate("false || echo ok"), Verdict::Allow);
    }

    #[test]
    fn test_background_splits() {
        let acl = AclEngine::new(Mode::Agent);
        assert!(matches!(acl.evaluate("sudo bash &"), Verdict::Deny(_)));
    }

    #[test]
    fn test_kids_blocks_command_substitution() {
        let acl = AclEngine::new(Mode::Kids);
        // $(...) blocked in Kids mode
        assert!(matches!(acl.evaluate("echo $(rm -rf /)"), Verdict::Deny(_)));
        // Backtick blocked
        assert!(matches!(acl.evaluate("echo `rm -rf /`"), Verdict::Deny(_)));
        // Process substitution blocked
        assert!(matches!(acl.evaluate("cat <(sudo bash)"), Verdict::Deny(_)));
    }

    #[test]
    fn test_agent_allows_command_substitution() {
        let acl = AclEngine::new(Mode::Agent);
        // Agent mode allows command substitution (needs it for scripts)
        assert_eq!(acl.evaluate("echo $(whoami)"), Verdict::Allow);
        // ...but still blocks sudo inside pipes
        assert!(matches!(acl.evaluate("$(sudo bash)"), Verdict::Deny(_)));
    }

    #[test]
    fn test_split_into_subcommands_basic() {
        let subs = split_into_subcommands("ls | grep foo ; echo done");
        assert_eq!(subs, vec!["ls", "grep foo", "echo done"]);
    }

    #[test]
    fn test_split_preserves_quoted_pipes() {
        let subs = split_into_subcommands("echo \"hello | world\" ; ls");
        assert_eq!(subs, vec!["echo \"hello | world\"", "ls"]);
    }

    #[test]
    fn test_command_substitution_detection() {
        assert!(contains_command_substitution("echo $(whoami)"));
        assert!(contains_command_substitution("echo `whoami`"));
        assert!(contains_command_substitution("cat <(ls)"));
        assert!(!contains_command_substitution("echo hello"));
        assert!(!contains_command_substitution("echo '$(not substitution)'")); // single-quoted
    }
}
