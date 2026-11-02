//! Single command execution pipeline for OmniShell.

use omnishell::audit::AuditLogger;
use omnishell::output::{emit_agent, format_error};
use omnishell::{builtins, AclEngine, Mode, SnapshotEngine, UndoStack, Verdict};

/// Execute a single command in non-interactive mode with full ACL/snapshot/audit pipeline.
pub fn execute_single_command(
    command: &str,
    mode: Mode,
    snapshot_engine: &mut SnapshotEngine,
    undo_stack: &mut UndoStack,
    audit: &AuditLogger,
) {
    use shrs::prelude::*;

    let mut acl = AclEngine::new(mode);

    // ACL check on the raw command
    if let Verdict::Deny(reason) = acl.evaluate(command) {
        if mode == Mode::Agent {
            // The reason belongs on the envelope's stderr field; the envelope
            // itself is the one JSON object written to stdout.
            emit_agent(command, "", &format!("omnishell: {reason}"), 126, 0);
        } else {
            eprintln!("{}", format_error(&reason, mode));
        }
        std::process::exit(126);
    }

    // Quick builtin check for simple commands (exit, help, etc)
    if let Some(tokens) = shlex::split(command) {
        if !tokens.is_empty() {
            let cmd = &tokens[0];
            let args = &tokens[1..];
            let builtin_start = std::time::Instant::now();
            if let Some(result) = builtins::dispatch(
                cmd,
                args,
                mode,
                &mut acl,
                Some(snapshot_engine),
                Some(undo_stack),
                None,
            ) {
                let duration = builtin_start.elapsed().as_millis() as u64;
                match result {
                    omnishell::builtins::BuiltinResult::Success(msg) => {
                        if mode == Mode::Agent {
                            emit_agent(command, &format!("{msg}\n"), "", 0, duration);
                        } else {
                            println!("{msg}");
                        }
                    }
                    omnishell::builtins::BuiltinResult::Error(msg) => {
                        if mode == Mode::Agent {
                            emit_agent(command, "", &format!("omnishell: {msg}\n"), 1, duration);
                        } else {
                            eprintln!("{}", format_error(&msg, mode));
                        }
                        std::process::exit(1);
                    }
                    omnishell::builtins::BuiltinResult::SwitchMode(new_mode) => {
                        let msg =
                            format!("Mode switch to {new_mode} ignored in non-interactive mode");
                        if mode == Mode::Agent {
                            emit_agent(command, "", &format!("{msg}\n"), 0, duration);
                        } else {
                            eprintln!("{msg}");
                        }
                    }
                    omnishell::builtins::BuiltinResult::Exit => {
                        if mode == Mode::Agent {
                            let code = args
                                .first()
                                .and_then(|arg| arg.parse::<i32>().ok())
                                .unwrap_or(0);
                            emit_agent(command, "", "", code, duration);
                            std::process::exit(code);
                        }
                        return;
                    }
                }
                return;
            }
        }
    }

    // Snapshot if mutating. Agent mode's evaluator does pre/post snapshots via
    // its ShellContext, so the single-command path must not duplicate them.
    if mode != Mode::Agent && SnapshotEngine::is_mutating_command(command) {
        let pre = snapshot_engine.pre_execution_snapshot(command).ok();
        // Store pre-snapshot for undo (will be completed after execution)
        std::env::set_var(
            "_OMNISHELL_PRE_SNAPSHOT",
            if pre.is_some() { "1" } else { "0" },
        );
    }

    // Build shell context and evaluate natively — no sh -c.
    //
    // Agent mode uses OmniShellLang: it is the same evaluator the interactive
    // shell uses, it propagates the command's actual exit status (shrs's
    // PosixLang collapses every successful parse to exit 0), and it owns the
    // capture that turns command output into the JSON envelope.
    //
    // Kids and admin keep PosixLang, preserving their existing output.
    let completer = omnishell::completion::CompletionEngine::new(mode);
    let builder = ShellBuilder::default()
        .with_state(omnishell::lang::FunctionTable::new())
        .with_state(omnishell::lang::ShellMode(mode))
        .with_state(omnishell::lang::ShellContext::minimal(mode))
        .with_state(omnishell::acl::AclEngine::new(mode))
        .with_state(omnishell::history::History::new(
            mode,
            omnishell::history::HistoryConfig::default(),
        ))
        .with_completer(completer);

    let (shell, states) = match mode {
        Mode::Agent => builder
            .with_lang(omnishell::lang::OmniShellLang)
            .build()
            .expect("failed to build shell config")
            .build_shell()
            .expect("failed to build shell"),
        _ => builder
            .with_lang(PosixLang::default())
            .build()
            .expect("failed to build shell config")
            .build_shell()
            .expect("failed to build shell"),
    };

    let start = std::time::Instant::now();
    let result = shell.lang.eval(&shell, &states, command.to_string());
    let duration = start.elapsed().as_millis() as u64;

    let exit_code = match result {
        Ok(cmd_output) => cmd_output.status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("omnishell: {e}");
            1
        }
    };

    // Agent mode's evaluator performs its own snapshot/audit pass through the
    // ShellContext; the PosixLang path needs the single-command pass below.
    if mode != Mode::Agent {
        if SnapshotEngine::is_mutating_command(command) {
            let post = snapshot_engine
                .post_execution_snapshot(command, exit_code)
                .ok();
            // Record in undo stack if we had a pre-snapshot
            let had_pre = std::env::var("_OMNISHELL_PRE_SNAPSHOT").unwrap_or_default() == "1";
            if had_pre {
                if let Some(pre) = snapshot_engine
                    .history()
                    .iter()
                    .rev()
                    .find(|s| s.phase == omnishell::snapshot::SnapshotPhase::PreExecution)
                {
                    undo_stack.record(pre.clone(), post, command.to_string());
                }
            }
            std::env::remove_var("_OMNISHELL_PRE_SNAPSHOT");
        }

        // Audit log
        let entry = AuditLogger::entry_for(command, mode)
            .exit_code(exit_code)
            .acl_verdict("allowed")
            .duration_ms(duration)
            .build();
        let _ = audit.log(entry);
    }

    std::process::exit(exit_code);
}
