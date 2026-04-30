use std::path::Path;

use clap::Parser;
use omnishell::{
    OmniShellConfig, Mode,
    load_config,
    AclEngine, Verdict,
    SnapshotEngine, UndoStack,
    builtins,
};
use omnishell::output::format_error;
use omnishell::history::{History, HistoryConfig};
use omnishell::audit::{AuditLogger, AuditConfig};
use omnishell::theme::Theme;

#[derive(Parser, Debug)]
#[command(author, version, about = "OmniShell: An intelligent, ACL-fortified shell")]
struct Args {
    /// Execution mode
    #[arg(short, long, value_enum, default_value_t = ShellMode::Admin)]
    mode: ShellMode,

    /// Profile name to use
    #[arg(short, long)]
    profile: Option<String>,

    /// Path to config file (overrides auto-discovery)
    #[arg(long)]
    config: Option<String>,

    /// Disable LLM features for this session
    #[arg(long)]
    no_llm: bool,

    /// Run a single command and exit (non-interactive)
    #[arg(short, long)]
    command: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
enum ShellMode {
    Kids,
    Agent,
    Admin,
}

fn main() {
    let args = Args::parse();

    // Load configuration
    let config_path = args.config.as_deref().map(Path::new);
    let config = load_config(config_path).unwrap_or_else(|e| {
        eprintln!("Warning: Failed to load config: {}. Using defaults.", e);
        OmniShellConfig::default()
    });

    // Resolve profile
    let profile_name = resolve_profile(&config, args.profile.as_deref());
    let _profile = config.profile.get(&profile_name).unwrap_or_else(|| {
        eprintln!("Warning: Profile '{}' not found. Using default.", profile_name);
        config.profile.get("default").expect("default profile always exists")
    });

    // --mode flag overrides the profile's mode
    let mode = match args.mode {
        ShellMode::Kids => Mode::Kids,
        ShellMode::Agent => Mode::Agent,
        ShellMode::Admin => Mode::Admin,
    };
    // Resolve LLM config
    let _llm_enabled = config.llm.enabled && !args.no_llm;

    // Initialize shell components
    let mut snapshot_engine = SnapshotEngine::new(&std::env::current_dir().unwrap_or_default());
    let _undo_stack = UndoStack::new();
    let _history = History::new(mode, HistoryConfig::default());
    let audit = AuditLogger::new(mode, AuditConfig::default());
    let theme = Theme::for_mode(mode);

    // Print startup banner
    eprintln!("{}", theme.primary(&format!(
        "OmniShell {} — {}",
        env!("CARGO_PKG_VERSION"),
        profile_name,
    )));
    if !_llm_enabled {
        eprintln!("{}", theme.error("(LLM disabled)"));
    }

    // If --command was provided, execute and exit
    if let Some(ref cmd) = args.command {
        execute_single_command(cmd, mode, &mut snapshot_engine, &audit);
        return;
    }

    // Launch interactive shell via shrs
    run_interactive_shell(mode);
}

/// Execute a single command non-interactively.
fn execute_single_command(
    command: &str,
    mode: Mode,
    snapshot_engine: &mut SnapshotEngine,
    audit: &AuditLogger,
) {
    let mut acl = AclEngine::new(mode);

    // ACL check
    if let Verdict::Deny(reason) = acl.evaluate(command) {
        eprintln!("{}", format_error(&reason, mode));
        std::process::exit(126);
    }

    // Check builtins
    let tokens: Vec<String> = command.split_whitespace().map(|s| s.to_string()).collect();
    if !tokens.is_empty() {
        let cmd = &tokens[0];
        let args = &tokens[1..];

        if let Some(result) = builtins::dispatch(cmd, args, mode, &mut acl) {
            match result {
                omnishell::builtins::BuiltinResult::Success(msg) => println!("{}", msg),
                omnishell::builtins::BuiltinResult::Error(msg) => {
                    eprintln!("{}", format_error(&msg, mode));
                    std::process::exit(1);
                }
                omnishell::builtins::BuiltinResult::SwitchMode(new_mode) => {
                    eprintln!("Mode switch to {} ignored in non-interactive mode", new_mode);
                }
                omnishell::builtins::BuiltinResult::Exit => return,
            }
            return;
        }
    }

    // Snapshot if mutating
    if SnapshotEngine::is_mutating_command(command) {
        let _ = snapshot_engine.pre_execution_snapshot(command);
    }

    // Execute via system
    let start = std::time::Instant::now();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .expect("failed to execute command");
    let duration = start.elapsed().as_millis() as u64;
    let exit_code = status.code().unwrap_or(1);

    if SnapshotEngine::is_mutating_command(command) {
        let _ = snapshot_engine.post_execution_snapshot(command, exit_code);
    }

    // Audit log
    let entry = omnishell::audit::AuditLogger::entry_for(command, mode)
        .exit_code(exit_code)
        .acl_verdict("allowed")
        .duration_ms(duration)
        .build();
    let _ = audit.log(entry);

    std::process::exit(exit_code);
}

/// Launch the interactive shell using shrs.
fn run_interactive_shell(mode: Mode) {
    use shrs::prelude::*;

    let myshell = ShellBuilder::default()
        .with_lang(omnishell::lang::OmniShellLang::default())
        .with_state(omnishell::lang::FunctionTable::new())
        .with_state(omnishell::lang::ShellMode(mode))
        .build()
        .unwrap();

    myshell.run().unwrap();
}

fn resolve_profile(config: &OmniShellConfig, cli_profile: Option<&str>) -> String {
    // 1. Check $USER binding (enforced, no override)
    if let Ok(username) = std::env::var("USER") {
        for (name, profile) in &config.profile {
            if profile.username.as_deref() == Some(&username) {
                return name.clone();
            }
        }
    }

    // 2. Check --profile CLI flag
    if let Some(name) = cli_profile {
        if config.profile.contains_key(name) {
            return name.to_string();
        }
    }

    // 3. Default profile
    if let Some(ref default) = config.default_profile {
        return default.clone();
    }

    // 4. First available profile
    config.profile.keys().next().unwrap_or(&"default".to_string()).clone()
}
