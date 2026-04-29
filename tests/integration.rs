//! Integration tests for OmniShell.
//!
//! Tests the interaction between modules:
//! - ACL → builtins → output pipeline
//! - Profile resolution from config
//! - Snapshot lifecycle (pre → command → post → undo)
//! - JSON output envelope in agent mode
//! - Mode switching effects

use omnishell::{
    AclEngine, Verdict,
    load_config,
    OmniShellConfig, Profile, Mode,
    SnapshotEngine, SnapshotPhase,
    UndoStack,
};
use omnishell::builtins::{self, BuiltinResult};
use omnishell::output::{CommandOutput, format_output, format_error};
use omnishell::llm_integration::system_prompt;
use omnishell::sandbox::Sandbox;
use omnishell::history::{History, HistoryConfig};

/// Test the full ACL → builtins → output pipeline for Kids mode.
#[test]
fn test_kids_mode_pipeline() {
    let mode = Mode::Kids;
    let acl = AclEngine::new(mode);

    // Safe command: ls
    assert_eq!(acl.evaluate("ls -la"), Verdict::Allow);
    let output = CommandOutput {
        command: "ls -la".to_string(),
        stdout: "file1.txt\nfile2.txt\n".to_string(),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 10,
    };
    let formatted = format_output(&output, mode);
    assert!(formatted.contains("✅"));

    // Dangerous command: sudo
    assert!(matches!(acl.evaluate("sudo bash"), Verdict::Deny(_)));
    let err = format_error("Command blocked by ACL", mode);
    assert!(err.contains("❌"));
}

/// Test the full ACL → builtins → output pipeline for Agent mode.
#[test]
fn test_agent_mode_pipeline() {
    let mode = Mode::Agent;
    let acl = AclEngine::new(mode);

    // Agent allows most things
    assert_eq!(acl.evaluate("cargo build"), Verdict::Allow);
    let output = CommandOutput {
        command: "cargo build".to_string(),
        stdout: "Compiling...".to_string(),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 5000,
    };
    let formatted = format_output(&output, mode);
    let parsed: serde_json::Value = serde_json::from_str(&formatted).unwrap();
    assert_eq!(parsed["type"], "success");
    assert_eq!(parsed["exitCode"], 0);

    // Agent blocks sudo
    assert!(matches!(acl.evaluate("sudo rm -rf /"), Verdict::Deny(_)));
    let err = format_error("sudo blocked", mode);
    let parsed_err: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(parsed_err["type"], "error");
}

/// Test built-in command dispatch with ACL.
#[test]
fn test_builtin_dispatch_with_acl() {
    let mut acl = AclEngine::new(Mode::Kids);

    // AI command should work in kids mode
    let result = builtins::dispatch("?", &["what is ls?".to_string()], Mode::Kids, &mut acl);
    assert!(matches!(result, Some(BuiltinResult::Success(_))));

    // Mode switch
    let result = builtins::dispatch("mode", &["agent".to_string()], Mode::Kids, &mut acl);
    assert!(matches!(result, Some(BuiltinResult::SwitchMode(Mode::Agent))));

    // Allow/block in admin mode
    let mut acl = AclEngine::new(Mode::Admin);
    builtins::dispatch("allow", &["custom_cmd".to_string()], Mode::Admin, &mut acl);
    assert!(acl.allowlist.iter().any(|r| r.pattern == "custom_cmd"));

    builtins::dispatch("block", &["evil_cmd".to_string()], Mode::Admin, &mut acl);
    assert!(acl.blocklist.iter().any(|r| r.pattern == "evil_cmd"));
}

/// Test snapshot lifecycle: pre-execution → command → post-execution → undo.
#[test]
fn test_snapshot_lifecycle_with_undo() {
    let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
    let mut undo_stack = UndoStack::new();

    // Pre-execution snapshot
    let pre = engine.pre_execution_snapshot("rm file.txt").unwrap();
    assert_eq!(pre.phase, SnapshotPhase::PreExecution);
    assert!(pre.commit_id.is_none()); // no git repo

    // Post-execution snapshot
    let post = engine.post_execution_snapshot("rm file.txt", 0).unwrap();
    assert_eq!(post.phase, SnapshotPhase::PostExecution);
    assert_eq!(post.exit_code, Some(0));

    // Record in undo stack
    undo_stack.record(pre, Some(post), "rm file.txt".to_string());
    assert_eq!(undo_stack.undo_count(), 1);

    // Undo
    let undo_snap = undo_stack.undo().unwrap();
    assert_eq!(undo_snap.trigger_command, "rm file.txt");
    assert_eq!(undo_stack.redo_count(), 1);
}

/// Test multiple commands with snapshot + undo.
#[test]
fn test_multiple_command_undo_redo() {
    let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
    let mut undo_stack = UndoStack::new();

    let commands = ["touch a", "touch b", "rm a"];
    for cmd in &commands {
        let pre = engine.pre_execution_snapshot(cmd).unwrap();
        let post = engine.post_execution_snapshot(cmd, 0).unwrap();
        undo_stack.record(pre, Some(post), cmd.to_string());
    }

    assert_eq!(undo_stack.undo_count(), 3);
    assert_eq!(engine.history().len(), 6); // 3 pre + 3 post

    // Undo twice
    undo_stack.undo();
    undo_stack.undo();
    assert_eq!(undo_stack.undo_count(), 1);
    assert_eq!(undo_stack.redo_count(), 2);

    // New command truncates redo
    let pre = engine.pre_execution_snapshot("mkdir new").unwrap();
    let post = engine.post_execution_snapshot("mkdir new", 0).unwrap();
    undo_stack.record(pre, Some(post), "mkdir new".to_string());
    assert_eq!(undo_stack.undo_count(), 2);
    assert_eq!(undo_stack.redo_count(), 0);
}

/// Test profile resolution from config.
#[test]
fn test_profile_resolution_from_config() {
    let mut config = OmniShellConfig::default();

    // Add a kids profile
    config.profile.insert("kids".to_string(), Profile {
        mode: Mode::Kids,
        username: Some("child".to_string()),
        display_name: Some("Kids Mode".to_string()),
        age: Some(7),
    });

    // Default is admin
    assert_eq!(config.profile["default"].mode, Mode::Admin);

    // Switch default
    config.default_profile = Some("kids".to_string());
    assert_eq!(config.profile[config.default_profile.as_ref().unwrap()].mode, Mode::Kids);
}

/// Test config loading from TOML file with profile resolution.
#[test]
fn test_config_load_and_resolve() {
    let dir = tempfile::tempdir().unwrap();
    let toml_path = dir.path().join("config.toml");
    std::fs::write(&toml_path, r#"
default_profile = "kids"

[profile.kids]
mode = "kids"
age = 7

[profile.agent]
mode = "agent"

[profile.admin]
mode = "admin"
"#).unwrap();

    let config = load_config(Some(&toml_path)).unwrap();
    assert_eq!(config.default_profile, Some("kids".to_string()));
    assert_eq!(config.profile["kids"].mode, Mode::Kids);
    assert_eq!(config.profile["kids"].age, Some(7));
    assert_eq!(config.profile["agent"].mode, Mode::Agent);
}

/// Test LLM system prompt per mode + output formatting integration.
#[test]
fn test_llm_prompt_matches_output_mode() {
    // Kids: tutor prompt + emoji output
    let kids_prompt = system_prompt(Mode::Kids);
    assert!(kids_prompt.contains("OmniTutor"));
    let kids_output = format_output(&CommandOutput {
        command: "ls".to_string(),
        stdout: "file.txt".to_string(),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 10,
    }, Mode::Kids);
    assert!(kids_output.contains("✅"));

    // Agent: JSON prompt + JSON output
    let agent_prompt = system_prompt(Mode::Agent);
    assert!(agent_prompt.contains("JSON"));
    let agent_output = format_output(&CommandOutput {
        command: "ls".to_string(),
        stdout: "file.txt".to_string(),
        stderr: String::new(),
        exit_code: 0,
        duration_ms: 10,
    }, Mode::Agent);
    let parsed: serde_json::Value = serde_json::from_str(&agent_output).unwrap();
    assert_eq!(parsed["type"], "success");
}

/// Test sandbox mode interaction with ACL.
#[test]
fn test_sandbox_acl_interaction() {
    // Kids mode: sandbox + strict ACL
    let acl = AclEngine::new(Mode::Kids);
    let sandbox = Sandbox::new(Sandbox::kids_default(std::path::Path::new("/home/child")));

    assert!(sandbox.is_enabled());
    assert!(matches!(acl.evaluate("ls"), Verdict::Allow));
    assert!(matches!(acl.evaluate("sudo"), Verdict::Deny(_)));

    // Admin mode: no sandbox + no ACL
    let acl = AclEngine::new(Mode::Admin);
    let sandbox = Sandbox::new(Sandbox::disabled());

    assert!(!sandbox.is_enabled());
    assert_eq!(acl.evaluate("sudo"), Verdict::Allow);
}

/// Test history + audit lifecycle.
#[test]
fn test_history_audit_lifecycle() {
    let mut history = History::new(Mode::Agent, HistoryConfig {
        persistent: false,
        ..Default::default()
    });

    // Simulate command execution
    let commands = ["cargo build", "cargo test", "cargo clippy"];
    for cmd in &commands {
        history.push(cmd, 0, None);
    }

    assert_eq!(history.len(), 3);

    // Search history
    let results = history.search("cargo");
    assert_eq!(results.len(), 3);

    let results = history.search("test");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].command, "cargo test");
}

/// Test ACL rules modified at runtime via builtins persist.
#[test]
fn test_runtime_acl_modification() {
    let mut acl = AclEngine::new(Mode::Admin);

    // Block a command
    builtins::dispatch("block", &["danger".to_string()], Mode::Admin, &mut acl);
    assert!(matches!(acl.evaluate("danger"), Verdict::Deny(_)));

    // Allow it back
    builtins::dispatch("allow", &["danger".to_string()], Mode::Admin, &mut acl);

    // Blocklist still has it, so it's still blocked
    // (blocklist overrides allowlist)
    assert!(matches!(acl.evaluate("danger"), Verdict::Deny(_)));
}

/// Test snapshot engine detects mutating commands.
#[test]
fn test_mutating_command_detection() {
    assert!(SnapshotEngine::is_mutating_command("rm file.txt"));
    assert!(SnapshotEngine::is_mutating_command("cargo build"));
    assert!(SnapshotEngine::is_mutating_command("git push"));
    assert!(!SnapshotEngine::is_mutating_command("ls"));
    assert!(!SnapshotEngine::is_mutating_command("git status"));
}

/// Test mode switch affects ACL, output, LLM, sandbox simultaneously.
#[test]
fn test_full_mode_switch() {
    for mode in [Mode::Kids, Mode::Agent, Mode::Admin] {
        let acl = AclEngine::new(mode);
        let sandbox_config = Sandbox::config_for_mode(mode, std::path::Path::new("/home/test"));
        let prompt = system_prompt(mode);

        match mode {
            Mode::Kids => {
                assert!(matches!(acl.evaluate("sudo"), Verdict::Deny(_)));
                assert!(sandbox_config.enabled);
                assert!(prompt.contains("OmniTutor"));
            }
            Mode::Agent => {
                assert!(matches!(acl.evaluate("cargo build"), Verdict::Allow));
                assert!(!sandbox_config.enabled);
                assert!(prompt.contains("JSON"));
            }
            Mode::Admin => {
                assert!(matches!(acl.evaluate("sudo"), Verdict::Allow));
                assert!(!sandbox_config.enabled);
                assert!(prompt.contains("system administrator"));
            }
        }
    }
}
