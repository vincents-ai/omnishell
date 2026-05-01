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
use crate::profile::Mode;

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
) -> Option<BuiltinResult> {
    match command {
        "?" | "ai" => Some(cmd_ai(args, mode)),
        "snapshots" => Some(cmd_snapshots(args)),
        "undo" => Some(cmd_undo(args)),
        "redo" => Some(cmd_redo(args)),
        "allow" => Some(cmd_allow(args, acl)),
        "block" => Some(cmd_block(args, acl)),
        "mode" => Some(cmd_mode(args, mode)),
        "help" => Some(cmd_help(mode)),
        "exit" | "quit" => Some(BuiltinResult::Exit),
        _ => None,
    }
}

/// `?` / `ai` — LLM interface.
fn cmd_ai(args: &[String], mode: Mode) -> BuiltinResult {
    let prompt = args.join(" ");
    if prompt.is_empty() {
        return BuiltinResult::Error("Usage: ? <question> or ai <question>".to_string());
    }

    match mode {
        Mode::Kids => BuiltinResult::Success(format!(
            "🤖 (Kids tutor mode) You asked: \"{prompt}\". Let me think about that..."
        )),
        Mode::Agent => BuiltinResult::Success(format!(
            "{{\"type\":\"llm_request\",\"prompt\":\"{prompt}\",\"mode\":\"agent\"}}"
        )),
        Mode::Admin => BuiltinResult::Success(format!(
            "[LLM] Query: {prompt}"
        )),
    }
}

/// `snapshots` — List snapshot history.
fn cmd_snapshots(args: &[String]) -> BuiltinResult {
    let _ = args; // TODO: support filtering
    // In a real implementation, this would query the SnapshotEngine
    BuiltinResult::Success("Snapshot history: (not yet connected to engine)".to_string())
}

/// `undo` — Undo last command.
fn cmd_undo(args: &[String]) -> BuiltinResult {
    let count = if args.is_empty() {
        1
    } else {
        args[0].parse::<usize>().unwrap_or(1)
    };
    BuiltinResult::Success(format!("Undoing {count} command(s)..."))
}

/// `redo` — Redo last undone command.
fn cmd_redo(args: &[String]) -> BuiltinResult {
    let count = if args.is_empty() {
        1
    } else {
        args[0].parse::<usize>().unwrap_or(1)
    };
    BuiltinResult::Success(format!("Redoing {count} command(s)..."))
}

/// `allow` — Add command to allowlist.
fn cmd_allow(args: &[String], acl: &mut AclEngine) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Error("Usage: allow <command>".to_string());
    }

    let pattern = &args[0];
    let reason = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        format!("Allowed by user in {} mode", acl.mode)
    };

    acl.allowlist.push(crate::acl::AclRule {
        pattern: pattern.clone(),
        args: vec![],
        reason,
    });

    BuiltinResult::Success(format!("Added '{pattern}' to allowlist"))
}

/// `block` — Add command to blocklist.
fn cmd_block(args: &[String], acl: &mut AclEngine) -> BuiltinResult {
    if args.is_empty() {
        return BuiltinResult::Error("Usage: block <command>".to_string());
    }

    let pattern = &args[0];
    let reason = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        format!("Blocked by user in {} mode", acl.mode)
    };

    acl.blocklist.push(crate::acl::AclRule {
        pattern: pattern.clone(),
        args: vec![],
        reason,
    });

    BuiltinResult::Success(format!("Added '{pattern}' to blocklist"))
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
        _ => return BuiltinResult::Error(format!("Unknown mode: '{}'. Use: kids, agent, admin", args[0])),
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
        let result = dispatch("?", &["what".to_string(), "is".to_string(), "ls?".to_string()], Mode::Kids, &mut acl);
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
        let result = dispatch("ai", &["build".to_string(), "project".to_string()], Mode::Agent, &mut acl);
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
        let result = dispatch("mode", &[], Mode::Admin, &mut acl);
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        let result = dispatch("mode", &["kids".to_string()], Mode::Admin, &mut acl);
        assert!(matches!(result, Some(BuiltinResult::SwitchMode(Mode::Kids))));
    }

    #[test]
    fn test_dispatch_allow_block() {
        let mut acl = AclEngine::new(Mode::Admin);

        let result = dispatch("allow", &["neato".to_string()], Mode::Admin, &mut acl);
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        let result = dispatch("block", &["dangerous".to_string()], Mode::Admin, &mut acl);
        assert!(matches!(result, Some(BuiltinResult::Success(_))));

        assert!(acl.allowlist.iter().any(|r| r.pattern == "neato"));
        assert!(acl.blocklist.iter().any(|r| r.pattern == "dangerous"));
    }

    #[test]
    fn test_dispatch_unknown() {
        let mut acl = AclEngine::new(Mode::Admin);
        assert!(dispatch("ls", &[], Mode::Admin, &mut acl).is_none());
        assert!(dispatch("cargo", &["build".to_string()], Mode::Admin, &mut acl).is_none());
    }

    #[test]
    fn test_dispatch_exit() {
        let mut acl = AclEngine::new(Mode::Admin);
        assert!(matches!(dispatch("exit", &[], Mode::Admin, &mut acl), Some(BuiltinResult::Exit)));
        assert!(matches!(dispatch("quit", &[], Mode::Admin, &mut acl), Some(BuiltinResult::Exit)));
    }

    #[test]
    fn test_help() {
        let mut acl = AclEngine::new(Mode::Admin);
        let result = dispatch("help", &[], Mode::Admin, &mut acl);
        if let Some(BuiltinResult::Success(s)) = result {
            assert!(s.contains("OmniShell"));
            assert!(s.contains("allow"));
            assert!(s.contains("block"));
        }
    }
}
