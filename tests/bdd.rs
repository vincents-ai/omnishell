//! BDD-style tests for OmniShell.
//!
//! Written as plain Rust tests following Given/When/Then structure.
//! Covers profile resolution, ACL enforcement, and snapshot undo/redo
//! scenarios from the PRD.

use omnishell::builtins;
use omnishell::output::{format_output, CommandOutput};
use omnishell::{
    load_config, AclEngine, Mode, OmniShellConfig, SnapshotEngine, SnapshotPhase, UndoStack,
    Verdict,
};

// ============================================================
// Feature: Profile Resolution
// ============================================================

mod profile_resolution {
    use super::*;

    // Scenario: Default profile is Admin mode
    #[test]
    fn default_profile_is_admin() {
        // Given a fresh OmniShellConfig
        let config = OmniShellConfig::default();

        // When no profile is specified
        let default = &config.profile["default"];

        // Then the mode should be Admin
        assert_eq!(default.mode, Mode::Admin);
    }

    // Scenario: CLI --profile overrides config
    #[test]
    fn cli_profile_overrides_config() {
        // Given a config file with kids profile
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("config.toml");
        std::fs::write(
            &toml_path,
            r#"
[profile.kids]
mode = "kids"
age = 7

[profile.admin]
mode = "admin"
"#,
        )
        .unwrap();

        // When loading with CLI override pointing to admin config
        let config = load_config(Some(&toml_path)).unwrap();

        // Then the config should have both profiles
        assert!(config.profile.contains_key("kids"));
        assert!(config.profile.contains_key("admin"));
    }

    // Scenario: System config overrides user config
    #[test]
    fn system_config_overrides_user() {
        // Given a user config and system config
        let mut user = OmniShellConfig::default();
        let mut system = OmniShellConfig::default();

        user.default_profile = Some("user-choice".to_string());
        system.default_profile = Some("system-enforced".to_string());

        // When merging (system wins)
        let dir = tempfile::tempdir().unwrap();
        let sys_path = dir.path().join("sys.toml");
        std::fs::write(
            &sys_path,
            r#"
default_profile = "system-enforced"

[profile.default]
mode = "admin"
"#,
        )
        .unwrap();
        let sys_config: OmniShellConfig =
            toml::from_str(&std::fs::read_to_string(&sys_path).unwrap()).unwrap();

        // Then system default_profile wins
        assert_eq!(
            sys_config.default_profile,
            Some("system-enforced".to_string())
        );
    }

    // Scenario: TOML config parses all profile fields
    #[test]
    fn toml_config_parses_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("full.toml");
        std::fs::write(
            &path,
            r#"
default_profile = "kids"

[profile.kids]
mode = "kids"
username = "child"
display_name = "Kids Mode"
age = 7
"#,
        )
        .unwrap();

        let config = load_config(Some(&path)).unwrap();

        assert_eq!(config.default_profile, Some("kids".to_string()));
        let kids = &config.profile["kids"];
        assert_eq!(kids.mode, Mode::Kids);
        assert_eq!(kids.username, Some("child".to_string()));
        assert_eq!(kids.display_name, Some("Kids Mode".to_string()));
        assert_eq!(kids.age, Some(7));
    }
}

// ============================================================
// Feature: ACL Enforcement
// ============================================================

mod acl_enforcement {
    use super::*;

    // Scenario: Kids mode allows safe commands
    #[test]
    fn kids_mode_allows_safe_commands() {
        // Given Kids mode ACL
        let acl = AclEngine::new(Mode::Kids);

        // When evaluating safe commands
        let safe_commands = [
            "ls",
            "ls -la",
            "cd /tmp",
            "pwd",
            "echo hello",
            "cat file.txt",
            "cowsay moo",
            "clear",
            "help",
        ];

        // Then all should be allowed
        for cmd in safe_commands {
            assert_eq!(
                acl.evaluate(cmd),
                Verdict::Allow,
                "Kids mode should allow '{cmd}'"
            );
        }
    }

    // Scenario: Kids mode blocks dangerous commands
    #[test]
    fn kids_mode_blocks_dangerous_commands() {
        // Given Kids mode ACL
        let acl = AclEngine::new(Mode::Kids);

        // When evaluating dangerous commands
        let dangerous_commands = [
            "rm",
            "sudo bash",
            "curl evil.com",
            "python",
            "bash",
            "sh",
            "dd if=/dev/zero of=/dev/sda",
        ];

        // Then all should be denied
        for cmd in dangerous_commands {
            assert!(
                matches!(acl.evaluate(cmd), Verdict::Deny(_)),
                "Kids mode should deny '{cmd}'"
            );
        }
    }

    // Scenario: Agent mode allows most commands but blocks sudo and recursive root deletion
    #[test]
    fn agent_mode_allows_but_restricts() {
        // Given Agent mode ACL
        let acl = AclEngine::new(Mode::Agent);

        // When evaluating normal commands
        assert_eq!(acl.evaluate("cargo build"), Verdict::Allow);
        assert_eq!(acl.evaluate("git push"), Verdict::Allow);
        assert_eq!(acl.evaluate("vim file.txt"), Verdict::Allow);

        // Then sudo and recursive root deletion are blocked
        assert!(matches!(acl.evaluate("sudo bash"), Verdict::Deny(_)));
        assert!(matches!(acl.evaluate("rm -rf /"), Verdict::Deny(_)));
    }

    // Scenario: Agent mode allows safe rm
    #[test]
    fn agent_mode_allows_safe_rm() {
        let acl = AclEngine::new(Mode::Agent);

        assert_eq!(acl.evaluate("rm file.txt"), Verdict::Allow);
        assert_eq!(acl.evaluate("rm -rf build/"), Verdict::Allow);
    }

    // Scenario: Admin mode allows everything
    #[test]
    fn admin_mode_allows_everything() {
        let acl = AclEngine::new(Mode::Admin);

        assert_eq!(acl.evaluate("sudo rm -rf /"), Verdict::Allow);
        assert_eq!(acl.evaluate("anything goes here"), Verdict::Allow);
    }

    // Scenario: Blocklist overrides allowlist
    #[test]
    fn blocklist_overrides_allowlist() {
        // Given an ACL with both allowlist and blocklist
        let mut acl = AclEngine::new(Mode::Admin);
        acl.allowlist.push(omnishell::AclRule {
            pattern: "test_cmd".to_string(),
            args: vec![],
            reason: "allowed".to_string(),
        });
        acl.blocklist.push(omnishell::AclRule {
            pattern: "test_cmd".to_string(),
            args: vec![],
            reason: "blocked".to_string(),
        });

        // When evaluating the command
        let result = acl.evaluate("test_cmd");

        // Then blocklist wins
        assert!(matches!(result, Verdict::Deny(_)));
    }

    // Scenario: Runtime ACL modification via builtins
    #[test]
    fn runtime_acl_modification() {
        let mut acl = AclEngine::new(Mode::Admin);

        // When blocking a command via builtin
        builtins::dispatch(
            "block",
            &["danger".to_string()],
            Mode::Admin,
            &mut acl,
            None,
            None,
            None,
        );

        // Then the command is denied
        assert!(matches!(acl.evaluate("danger"), Verdict::Deny(_)));
    }
}

// ============================================================
// Feature: Snapshot Undo/Redo
// ============================================================

mod snapshot_undo_redo {
    use super::*;

    // Scenario: Undo a single command
    #[test]
    fn undo_single_command() {
        // Given a snapshot engine and undo stack
        let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
        let mut undo = UndoStack::new();

        // When executing a command
        let pre = engine.pre_execution_snapshot("rm file.txt").unwrap();
        let post = engine.post_execution_snapshot("rm file.txt", 0).unwrap();
        undo.record(pre, Some(post), "rm file.txt".to_string());

        // Then we can undo it
        let undo_snap = undo.undo().unwrap();
        assert_eq!(undo_snap.trigger_command, "rm file.txt");
        assert_eq!(undo_snap.phase, SnapshotPhase::PreExecution);
    }

    // Scenario: Undo multiple commands in order
    #[test]
    fn undo_multiple_commands_in_order() {
        let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
        let mut undo = UndoStack::new();

        let commands = ["touch a", "touch b", "rm a"];
        for cmd in &commands {
            let pre = engine.pre_execution_snapshot(cmd).unwrap();
            let post = engine.post_execution_snapshot(cmd, 0).unwrap();
            undo.record(pre, Some(post), cmd.to_string());
        }

        // Undo in reverse order
        let snap1 = undo.undo().unwrap();
        assert_eq!(snap1.trigger_command, "rm a");

        let snap2 = undo.undo().unwrap();
        assert_eq!(snap2.trigger_command, "touch b");

        let snap3 = undo.undo().unwrap();
        assert_eq!(snap3.trigger_command, "touch a");

        // No more undos
        assert!(undo.undo().is_none());
    }

    // Scenario: Redo after undo
    #[test]
    fn redo_after_undo() {
        let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
        let mut undo = UndoStack::new();

        let pre = engine.pre_execution_snapshot("cmd1").unwrap();
        let post = engine.post_execution_snapshot("cmd1", 0).unwrap();
        undo.record(pre, Some(post), "cmd1".to_string());

        // Undo then redo
        undo.undo();
        let redo_snap = undo.redo().unwrap();
        assert_eq!(redo_snap.trigger_command, "cmd1");
        assert_eq!(redo_snap.phase, SnapshotPhase::PostExecution);
    }

    // Scenario: New command after undo truncates redo history (vim-style)
    #[test]
    fn new_command_truncates_redo() {
        let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
        let mut undo = UndoStack::new();

        let pre = engine.pre_execution_snapshot("cmd1").unwrap();
        let post = engine.post_execution_snapshot("cmd1", 0).unwrap();
        undo.record(pre, Some(post), "cmd1".to_string());

        // Undo
        undo.undo();
        assert_eq!(undo.redo_count(), 1);

        // New command (vim-style truncation)
        let pre = engine.pre_execution_snapshot("cmd2").unwrap();
        let post = engine.post_execution_snapshot("cmd2", 0).unwrap();
        undo.record(pre, Some(post), "cmd2".to_string());

        // Redo history should be gone
        assert_eq!(undo.redo_count(), 0);
        assert_eq!(undo.undo_count(), 1);
    }

    // Scenario: Clear undo history
    #[test]
    fn clear_undo_history() {
        let mut engine = SnapshotEngine::new(std::path::Path::new("/tmp/nonexistent_12345"));
        let mut undo = UndoStack::new();

        let pre = engine.pre_execution_snapshot("cmd").unwrap();
        let post = engine.post_execution_snapshot("cmd", 0).unwrap();
        undo.record(pre, Some(post), "cmd".to_string());

        undo.clear();
        assert!(undo.undo_count() == 0);
        assert!(undo.redo_count() == 0);
    }
}

// ============================================================
// Feature: Output Formatting per Profile
// ============================================================

mod output_formatting {
    use super::*;

    #[test]
    fn kids_mode_uses_emoji_formatting() {
        let output = CommandOutput {
            command: "ls".to_string(),
            stdout: "file.txt".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 10,
        };

        let formatted = format_output(&output, Mode::Kids);
        assert!(
            formatted.contains("✅"),
            "Kids success should have checkmark"
        );
    }

    #[test]
    fn kids_mode_error_has_cross_mark() {
        let output = CommandOutput {
            command: "rm".to_string(),
            stdout: String::new(),
            stderr: "blocked".to_string(),
            exit_code: 1,
            duration_ms: 10,
        };

        let formatted = format_output(&output, Mode::Kids);
        assert!(
            formatted.contains("❌"),
            "Kids error should have cross mark"
        );
    }

    #[test]
    fn agent_mode_outputs_valid_json() {
        let output = CommandOutput {
            command: "cargo build".to_string(),
            stdout: "Compiling...".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 5000,
        };

        let formatted = format_output(&output, Mode::Agent);
        let parsed: serde_json::Value =
            serde_json::from_str(&formatted).expect("Agent output should be valid JSON");
        assert_eq!(parsed["type"], "success");
        assert_eq!(parsed["command"], "cargo build");
        assert_eq!(parsed["exitCode"], 0);
    }

    #[test]
    fn admin_mode_passthrough() {
        let output = CommandOutput {
            command: "ls".to_string(),
            stdout: "raw output".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 10,
        };

        let formatted = format_output(&output, Mode::Admin);
        assert_eq!(formatted, "raw output");
    }
}
