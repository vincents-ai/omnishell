//! OmniShell built-in commands.
//!
//! These commands are resolved before any external command lookup:
//! - `?` / `ai` — LLM interface
//! - `snapshots` — List snapshot history
//! - `undo` — Undo last command
//! - `redo` — Redo last undone command
//! - `allow` — Add command to allowlist
//! - `block` — Add command to blocklist
//! - `mode` — Show or switch execution mode
//! - `help` — Show available commands

use crate::acl::AclEngine;
use crate::llm_integration::LlmClient;
use crate::profile::Mode;
use crate::snapshot::SnapshotEngine;
use crate::undo::UndoStack;

/// Result of executing a built-in command.
#[derive(Debug)]
pub enum BuiltinResult {
    /// Command executed successfully with output.
    Success(String),
    /// Command failed with an error message.
    Error(String),
    /// Request to change mode.
    SwitchMode(Mode),
    /// Request to exit the shell.
    Exit,
}

/// Dispatch a built-in command. Returns None if the command is not a builtin.
pub fn dispatch(
    command: &str,
    args: &[String],
    mode: Mode,
    acl: &mut AclEngine,
    snapshot_engine: Option<&SnapshotEngine>,
    undo_stack: Option<&mut UndoStack>,
    llm_client: Option<&LlmClient>,
) -> Option<BuiltinResult> {
    match command {
        "?" | "ai" => Some(cmd_ai(args, mode, llm_client)),
        "snapshots" => Some(cmd_snapshots(args, snapshot_engine)),
        "undo" => Some(cmd_undo(args, undo_stack)),
        "redo" => Some(cmd_redo(args, undo_stack)),
        "allow" => Some(cmd_allow(args, acl, mode)),
        "block" => Some(cmd_block(args, acl, mode)),
        "export" => Some(cmd_export(args)),
        "source" | "." => Some(cmd_source(args, mode)),
        "alias" => Some(cmd_alias(args, mode)),
        "unalias" => Some(cmd_unalias(args, mode)),
        "mode" => Some(cmd_mode(args, mode)),
        "help" => Some(cmd_help(mode)),
        "exit" | "quit" => Some(BuiltinResult::Exit),
        _ => None,
    }
}

/// `?` / `ai` — LLM interface.
fn cmd_ai(args: &[String], mode: Mode, llm_client: Option<&LlmClient>) -> BuiltinResult {
    let prompt = args.join(" ");
    if prompt.is_empty() {
        return BuiltinResult::Error("Usage: ? <question> or ai <question>".to_string());
    }

    match llm_client {
        Some(client) => match client.query_sync(&prompt) {
            crate::llm_integration::LlmResponse::Success(content) => {
                BuiltinResult::Success(content)
            }
            crate::llm_integration::LlmResponse::Disabled(msg) => BuiltinResult::Error(msg),
            crate::llm_integration::LlmResponse::Error(msg) => BuiltinResult::Error(msg),
        },
        None => {
            // Fallback mock responses when no LLM client available
            match mode {
                Mode::Kids => BuiltinResult::Success(format!(
                    "🤖 (Kids tutor mode) You asked: \"{prompt}\". LLM not configured."
                )),
                Mode::Agent => BuiltinResult::Success(format!(
                    "{{\"type\":\"llm_request\",\"prompt\":\"{prompt}\",\"mode\":\"agent\"}}"
                )),
                Mode::Admin => {
                    BuiltinResult::Success(format!("[LLM] Query: {prompt} (no client configured)"))
                }
            }
        }
    }
}

/// `snapshots` — List snapshot history.
///
/// Supports filter args:
///   `snapshots` — list all
///   `snapshots --limit N` — show last N
///   `snapshots --command <pattern>` — filter by command pattern
///   `snapshots --phase pre|post` — filter by phase
fn cmd_snapshots(args: &[String], engine: Option<&SnapshotEngine>) -> BuiltinResult {
    let mut limit: Option<usize> = None;
    let mut command_filter: Option<&str> = None;
    let mut phase_filter: Option<&str> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--limit" | "-n" => {
                i += 1;
                if i < args.len() {
                    limit = args[i].parse().ok();
                }
            }
            "--command" | "-c" => {
                i += 1;
                if i < args.len() {
                    command_filter = Some(&args[i]);
                }
            }
            "--phase" | "-p" => {
                i += 1;
                if i < args.len() {
                    phase_filter = Some(&args[i]);
                }
            }
            "--help" | "-h" => {
                return BuiltinResult::Success(
                    "Usage: snapshots [--limit N] [--command <pattern>] [--phase pre|post]\n\
                     \n\
                     Options:\n\
                       --limit, -n     Show last N snapshots\n\
                       --command, -c   Filter by command pattern\n\
                       --phase, -p     Filter by phase (pre/post)\n\
                       --help, -h      Show this help"
                        .to_string(),
                );
            }
            other => {
                return BuiltinResult::Error(format!("snapshots: unknown option: {other}"));
            }
        }
        i += 1;
    }

    match engine {
        Some(engine) => {
            let history = engine.history();
            let filtered: Vec<_> = history
                .iter()
                .rev()
                .filter(|s| {
                    if let Some(c) = command_filter {
                        if !s.trigger_command.contains(c) {
                            return false;
                        }
                    }
                    if let Some(p) = phase_filter {
                        let phase_str = match s.phase {
                            crate::snapshot::SnapshotPhase::PreExecution => "pre",
                            crate::snapshot::SnapshotPhase::PostExecution => "post",
                        };
                        if phase_str != p {
                            return false;
                        }
                    }
                    true
                })
                .collect();
            let to_show = match limit {
                Some(n) => filtered.iter().take(n),
                None => filtered.iter().take(50),
            };
            let mut msg = String::new();
            for snap in to_show {
                let phase_str = match snap.phase {
                    crate::snapshot::SnapshotPhase::PreExecution => "PRE",
                    crate::snapshot::SnapshotPhase::PostExecution => "POST",
                };
                let commit = snap
                    .commit_id
                    .map(|id| id.to_string().chars().take(7).collect::<String>())
                    .unwrap_or_else(|| "(none)".to_string());
                msg.push_str(&format!(
                    "  [{commit}] {phase_str} {} (exit={})\n",
                    snap.trigger_command,
                    snap.exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                ));
            }
            if msg.is_empty() {
                msg = "No snapshots recorded.\n".to_string();
            }
            BuiltinResult::Success(msg)
        }
        None => BuiltinResult::Success("Snapshot history: (engine not available)".to_string()),
    }
}

/// `undo` — Undo last command via UndoStack.
fn cmd_undo(args: &[String], undo_stack: Option<&mut UndoStack>) -> BuiltinResult {
    let count = if args.is_empty() {
        1
    } else {
        args[0].parse::<usize>().unwrap_or(1)
    };

    match undo_stack {
        Some(stack) => {
            let mut undone = 0;
            for _ in 0..count {
                if stack.undo().is_none() {
                    break;
                }
                undone += 1;
            }
            if undone == 0 {
                BuiltinResult::Error("Nothing to undo".to_string())
            } else {
                BuiltinResult::Success(format!("Undone {undone} command(s)"))
            }
        }
        None => BuiltinResult::Success(format!(
            "Undoing {count} command(s)... (stack not available)"
        )),
    }
}

/// `redo` — Redo last undone command via UndoStack.
fn cmd_redo(args: &[String], undo_stack: Option<&mut UndoStack>) -> BuiltinResult {
    let count = if args.is_empty() {
        1
    } else {
        args[0].parse::<usize>().unwrap_or(1)
    };

    match undo_stack {
        Some(stack) => {
            let mut redone = 0;
            for _ in 0..count {
                if stack.redo().is_none() {
                    break;
                }
                redone += 1;
            }
            if redone == 0 {
                BuiltinResult::Error("Nothing to redo".to_string())
            } else {
                BuiltinResult::Success(format!("Redone {redone} command(s)"))
            }
        }
        None => BuiltinResult::Success(format!(
            "Redoing {count} command(s)... (stack not available)"
        )),
    }
}

/// `allow` — Add command to allowlist.
fn cmd_allow(args: &[String], acl: &mut AclEngine, mode: Mode) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Error("Usage: allow <command>".to_string());
    }

    // Only Admin mode can modify ACL
    if mode != Mode::Admin {
        return BuiltinResult::Error("'allow' command is not available in this mode".to_string());
    }

    let pattern = &args[0];
    let reason = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        format!("Allowed by admin in {} mode", acl.mode)
    };

    acl.allowlist.push(crate::acl::AclRule {
        pattern: pattern.clone(),
        args: vec![],
        reason,
    });

    BuiltinResult::Success(format!("Added '{pattern}' to allowlist"))
}

/// `block` — Add command to blocklist.
fn cmd_block(args: &[String], acl: &mut AclEngine, mode: Mode) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Error("Usage: block <command>".to_string());
    }

    // Only Admin mode can modify ACL
    if mode != Mode::Admin {
        return BuiltinResult::Error("'block' command is not available in this mode".to_string());
    }

    let pattern = &args[0];
    let reason = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        format!("Blocked by admin in {} mode", acl.mode)
    };

    acl.blocklist.push(crate::acl::AclRule {
        pattern: pattern.clone(),
        args: vec![],
        reason,
    });

    BuiltinResult::Success(format!("Added '{pattern}' to blocklist"))
}

/// `export` — Set environment variable for child processes.
fn cmd_export(args: &[String]) -> BuiltinResult {
    if args.is_empty() {
        // With no args, list all exported env vars
        let vars: Vec<String> = std::env::vars().map(|(k, v)| format!("{k}={v}")).collect();
        return BuiltinResult::Success(vars.join("\n"));
    }

    for arg in args {
        if let Some((key, value)) = arg.split_once('=') {
            std::env::set_var(key, value);
        } else {
            // export VAR (no value) — mark for export by ensuring it exists
            if std::env::var(arg).is_err() {
                std::env::set_var(arg, "");
            }
        }
    }
    BuiltinResult::Success(String::new())
}

/// `source` / `.` — Execute commands from a file in current shell context.
fn cmd_source(args: &[String], _mode: Mode) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Error("Usage: source <file> [args...]".to_string());
    }

    let path = &args[0];
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => return BuiltinResult::Error(format!("source: {path}: {e}")),
    };

    // Note: actual evaluation happens in the caller's context.
    // For non-interactive mode, we return the content to be evaluated.
    // The caller (main.rs execute_single_command or interactive loop) will
    // feed this through the shell evaluator.
    BuiltinResult::Success(content)
}

/// `alias` — Define or list shell aliases.
fn cmd_alias(args: &[String], mode: Mode) -> BuiltinResult {
    // Kids mode cannot define aliases
    if mode == Mode::Kids && !args.is_empty() {
        return BuiltinResult::Error("alias: not available in Kids mode".to_string());
    }

    if args.is_empty() {
        // List all aliases (from profile config)
        // Note: runtime aliases are not yet tracked in a shared state
        BuiltinResult::Success("(alias list: use profile config to define aliases)".to_string())
    } else {
        for arg in args {
            if let Some((name, value)) = arg.split_once('=') {
                // Store as env var with prefix for now (proper state tracking needs shared State)
                std::env::set_var(format!("_OMNISHELL_ALIAS_{name}"), value);
            } else {
                return BuiltinResult::Error(format!(
                    "alias: invalid format: {arg} (use name=value)"
                ));
            }
        }
        BuiltinResult::Success(String::new())
    }
}

/// `unalias` — Remove a shell alias.
fn cmd_unalias(args: &[String], mode: Mode) -> BuiltinResult {
    if mode == Mode::Kids {
        return BuiltinResult::Error("unalias: not available in Kids mode".to_string());
    }

    if args.is_empty() {
        return BuiltinResult::Error("Usage: unalias <name>".to_string());
    }

    for name in args {
        let key = format!("_OMNISHELL_ALIAS_{name}");
        if std::env::var(&key).is_ok() {
            std::env::remove_var(&key);
        } else {
            return BuiltinResult::Error(format!("unalias: {name}: not found"));
        }
    }
    BuiltinResult::Success(String::new())
}

/// `mode` — Show or switch execution mode.
fn cmd_mode(args: &[String], current: Mode) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Success(format!("Current mode: {current}"));
    }

    let new_mode = match args[0].to_lowercase().as_str() {
        "kids" | "child" => Mode::Kids,
        "agent" | "ai" => Mode::Agent,
        "admin" | "root" => Mode::Admin,
        _ => {
            return BuiltinResult::Error(format!(
                "Unknown mode: '{}'. Use: kids, agent, admin",
                args[0]
            ))
        }
    };

    if new_mode == current {
        return BuiltinResult::Success(format!("Already in {new_mode} mode"));
    }

    BuiltinResult::SwitchMode(new_mode)
}

/// `help` — Show available commands.
fn cmd_help(mode: Mode) -> BuiltinResult {
    let mut help = String::new();
    help.push_str("OmniShell Built-in Commands:\n");
    help.push_str("  ?, ai <prompt>  — Ask the AI assistant\n");
    help.push_str("  snapshots       — List command snapshots\n");
    help.push_str("  undo [n]        — Undo last n commands (default: 1)\n");
    help.push_str("  redo [n]        — Redo last n undone commands (default: 1)\n");

    match mode {
        Mode::Admin => {
            help.push_str("  allow <cmd>     — Add command to allowlist\n");
            help.push_str("  block <cmd>     — Add command to blocklist\n");
            help.push_str("  mode [mode]     — Show or switch mode (kids/agent/admin)\n");
        }
        Mode::Agent => {
            help.push_str("  block <cmd>     — Add command to blocklist\n");
        }
        Mode::Kids => {
            // Minimal help for kids
        }
    }

    help.push_str("  help            — Show this help\n");
    help.push_str("  exit, quit      — Exit the shell\n");

    BuiltinResult::Success(help)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_ai_kids() {
        let mut acl = AclEngine::new(Mode::Kids);
        let result = dispatch(
            "?",
            &["what".to_string(), "is".to_string(), "ls?".to_string()],
            Mode::Kids,
            &mut acl,
            None,
            None,
            None,
        );
        let msg = result.unwrap();
        if let BuiltinResult::Success(s) = msg {
            assert!(s.contains("Kids tutor mode"));
        } else {
            panic!("Expected Success");
        }
    }

    #[test]
    fn test_dispatch_ai_agent() {
        let mut acl = AclEngine::new(Mode::Agent);
        let result = dispatch(
            "ai",
            &["build".to_string(), "project".to_string()],
            Mode::Agent,
            &mut acl,
            None,
            None,
            None,
        );
        let msg = result.unwrap();
        if let BuiltinResult::Success(s) = msg {
            assert!(s.contains("\"type\":\"llm_request\""));
        } else {
            panic!("Expected Success");
        }
    }

    #[test]
    fn test_dispatch_mode() {
        let mut acl = AclEngine::new(Mode::Admin);
        let result = dispatch("mode", &[], Mode::Admin, &mut acl, None, None, None);
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        let result = dispatch(
            "mode",
            &["kids".to_string()],
            Mode::Admin,
            &mut acl,
            None,
            None,
            None,
        );
        assert!(matches!(
            result,
            Some(BuiltinResult::SwitchMode(Mode::Kids))
        ));
    }

    #[test]
    fn test_dispatch_allow_block() {
        let mut acl = AclEngine::new(Mode::Admin);

        let result = dispatch(
            "allow",
            &["neato".to_string()],
            Mode::Admin,
            &mut acl,
            None,
            None,
            None,
        );
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        let result = dispatch(
            "block",
            &["dangerous".to_string()],
            Mode::Admin,
            &mut acl,
            None,
            None,
            None,
        );
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        assert!(acl.allowlist.iter().any(|r| r.pattern == "neato"));
        assert!(acl.blocklist.iter().any(|r| r.pattern == "dangerous"));
    }

    #[test]
    fn test_dispatch_unknown() {
        let mut acl = AclEngine::new(Mode::Admin);
        assert!(dispatch("ls", &[], Mode::Admin, &mut acl, None, None, None).is_none());
        assert!(dispatch(
            "cargo",
            &["build".to_string()],
            Mode::Admin,
            &mut acl,
            None,
            None,
            None
        )
        .is_none());
    }

    #[test]
    fn test_dispatch_exit() {
        let mut acl = AclEngine::new(Mode::Admin);
        assert!(matches!(
            dispatch("exit", &[], Mode::Admin, &mut acl, None, None, None),
            Some(BuiltinResult::Exit)
        ));
        assert!(matches!(
            dispatch("quit", &[], Mode::Admin, &mut acl, None, None, None),
            Some(BuiltinResult::Exit)
        ));
    }

    #[test]
    fn test_help() {
        let mut acl = AclEngine::new(Mode::Admin);
        let result = dispatch("help", &[], Mode::Admin, &mut acl, None, None, None);
        if let Some(BuiltinResult::Success(s)) = result {
            assert!(s.contains("OmniShell"));
            assert!(s.contains("allow"));
            assert!(s.contains("block"));
        }
    }
}
