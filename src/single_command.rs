//! Single command execution pipeline for OmniShell.

use std::path::Path;

use omnishell::audit::AuditLogger;
use omnishell::lang::ScriptParams;
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
    execute_command_text(command, None, mode, snapshot_engine, undo_stack, audit);
}

/// Read a script file and execute its text through the single-command pipeline.
///
/// The script path becomes `$0` and `args` become `$1`, `$2`, ... and `$@`.
/// The shebang line is stripped; its interpreter and single optional argument
/// are recorded but not exposed as positional parameters, because the kernel
/// consumes a shebang argument before the script ever sees its arguments.
pub fn execute_script(
    path: &Path,
    args: &[String],
    mode: Mode,
    snapshot_engine: &mut SnapshotEngine,
    undo_stack: &mut UndoStack,
    audit: &AuditLogger,
) {
    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(e) => {
            let reason = format!("{}: {e}", path.display());
            if mode == Mode::Agent {
                emit_agent(
                    &path.to_string_lossy(),
                    "",
                    &format!("omnishell: {reason}"),
                    126,
                    0,
                );
            } else {
                eprintln!("{}", format_error(&reason, mode));
            }
            std::process::exit(126);
        }
    };

    let (body, _shebang) = split_shebang(&source);
    let params = ScriptParams {
        name: path.to_string_lossy().to_string(),
        args: args.to_vec(),
    };
    omnishell::lang::set_script_params(Some(params.clone()));

    execute_command_text(
        &body,
        Some(&params),
        mode,
        snapshot_engine,
        undo_stack,
        audit,
    );
}

/// Split a leading `#!` line from script source.
///
/// Returns the remaining source and the shebang's interpreter plus its optional
/// single argument (`#!/bin/sh -e` gives `("/bin/sh", Some("-e"))`). A shebang
/// carries at most one argument; anything after it is not a kernel shebang.
pub fn split_shebang(source: &str) -> (String, Option<(String, Option<String>)>) {
    let normalized = source.strip_prefix('\u{feff}').unwrap_or(source);
    if !normalized.starts_with("#!") {
        return (normalized.to_string(), None);
    }
    let (first, rest) = match normalized.find('\n') {
        Some(pos) => (&normalized[..pos], &normalized[pos + 1..]),
        None => (normalized, ""),
    };
    let mut parts = first[2..].split_whitespace();
    let interpreter = parts.next().map(ToString::to_string);
    let shebang_arg = parts.next().map(ToString::to_string);
    match interpreter {
        Some(interpreter) => (rest.to_string(), Some((interpreter, shebang_arg))),
        None => (rest.to_string(), None),
    }
}

fn execute_command_text(
    command: &str,
    script: Option<&ScriptParams>,
    mode: Mode,
    snapshot_engine: &mut SnapshotEngine,
    undo_stack: &mut UndoStack,
    audit: &AuditLogger,
) {
    use shrs::prelude::*;

    let mut acl = AclEngine::new(mode);

    // ACL check on the raw command. A script is checked command by command as
    // the evaluator reaches each simple command, so the whole script text is
    // not one ACL subject.
    if script.is_none() {
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
    }

    // Quick builtin check for simple commands (exit, help, etc). A script is a
    // sequence of commands, so it always goes through the evaluator below.
    let builtin_tokens = if script.is_none() {
        shlex::split(command)
    } else {
        None
    };
    if let Some(tokens) = builtin_tokens {
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
    if script.is_none() && mode != Mode::Agent && SnapshotEngine::is_mutating_command(command) {
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
    // A script also uses OmniShellLang in every mode: `$0`, `$1`, `$#` and
    // `$@` are expanded there, and PosixLang would both drop them and collapse
    // the script's exit status. Kids and admin single commands keep PosixLang,
    // preserving their existing output.
    let positional_script = script.is_some();
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

    let (shell, states) = if mode == Mode::Agent || positional_script {
        builder
            .with_lang(omnishell::lang::OmniShellLang)
            .build()
            .expect("failed to build shell config")
            .build_shell()
            .expect("failed to build shell")
    } else {
        builder
            .with_lang(PosixLang::default())
            .build()
            .expect("failed to build shell config")
            .build_shell()
            .expect("failed to build shell")
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
    // A script's commands each run that same per-command pass, so the script
    // text itself is not audited as one command.
    if script.is_none() && mode != Mode::Agent {
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
