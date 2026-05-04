//! OmniShell — An intelligent, ACL-fortified shell.
//!
//! Main entry point: CLI parsing, config loading, and dispatch to either
//! single-command execution or interactive shell mode.

mod cli;
mod profile_resolver;
mod shell_builder;
mod single_command;

use std::path::Path;

use clap::Parser;
use omnishell::audit::{AuditConfig, AuditLogger};
use omnishell::{load_config, Mode, OmniShellConfig, SnapshotEngine};

fn main() {
    let args = cli::Args::parse();

    // Load configuration
    let config_path = args.config.as_deref().map(Path::new);
    let config = load_config(config_path).unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config: {e}. Using defaults.");
        OmniShellConfig::default()
    });

    // Resolve profile
    let profile_name = profile_resolver::resolve_profile(&config, args.profile.as_deref());
    let profile = config.profile.get(&profile_name).unwrap_or_else(|| {
        eprintln!("Warning: Profile '{profile_name}' not found. Using default.");
        config
            .profile
            .get("default")
            .expect("default profile always exists")
    });

    // --mode flag overrides the profile's mode
    let mode: Mode = args.mode.into();
    let _llm_enabled = config.llm.enabled && !args.no_llm;

    // Initialize shell components
    let mut snapshot_engine = SnapshotEngine::new(&std::env::current_dir().unwrap_or_default());
    let audit = AuditLogger::new(mode, AuditConfig::default());
    let theme = profile.theme();

    // Print startup banner
    eprintln!(
        "{}",
        theme.primary(&format!(
            "OmniShell {} — {}",
            env!("CARGO_PKG_VERSION"),
            profile_name,
        ))
    );
    if !_llm_enabled {
        eprintln!("{}", theme.error("(LLM disabled)"));
    }

    // If --command was provided, execute and exit
    if let Some(ref cmd) = args.command {
        let mut undo_stack = omnishell::UndoStack::new();
        single_command::execute_single_command(
            cmd,
            mode,
            &mut snapshot_engine,
            &mut undo_stack,
            &audit,
        );
        return;
    }

    // Launch interactive shell via shrs
    shell_builder::run_interactive_shell(mode, &theme);
}
