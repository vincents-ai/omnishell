use std::path::Path;

use clap::Parser;
use omnishell::{
    OmniShellConfig, Mode,
    load_config,
    AclEngine, Verdict,
    SnapshotEngine,
    builtins,
};
use omnishell::output::format_error;
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
    let profile = config.profile.get(&profile_name).unwrap_or_else(|| {
        eprintln!("Warning: Profile '{}' not found. Using default.", profile_name);
        config.profile.get("default").expect("default profile always exists")
    });

    // --mode flag overrides the profile's mode
    let mode = match args.mode {
        ShellMode::Kids => Mode::Kids,
        ShellMode::Agent => Mode::Agent,
        ShellMode::Admin => Mode::Admin,
    };
    let _llm_enabled = config.llm.enabled && !args.no_llm;

    // Initialize shell components
    let mut snapshot_engine = SnapshotEngine::new(&std::env::current_dir().unwrap_or_default());
    let audit = AuditLogger::new(mode, AuditConfig::default());
    let theme = profile.theme();

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
    run_interactive_shell(mode, &theme);
}

/// Execute a single command non-interactively.
///
/// Uses OmniShell's own POSIX evaluator for compound commands (if/for/while/case,
/// pipes, &&/||, $(cmd), $((expr))). Falls back to direct fork+exec for simple
/// commands that don't need the parser.
///
/// TODO: The POSIX evaluator currently delegates to /usr/bin/env sh for compound
/// commands because shrs's Shell struct cannot be constructed outside of
/// ShellConfig::run() (the interactive loop). Once shrs exposes a non-interactive
/// eval path, this TODO should be removed. The interactive shell uses OmniShellLang
/// directly with zero sh delegation.
fn execute_single_command(
    command: &str,
    mode: Mode,
    snapshot_engine: &mut SnapshotEngine,
    audit: &AuditLogger,
) {
    let mut acl = AclEngine::new(mode);

    // ACL check on the raw command
    if let Verdict::Deny(reason) = acl.evaluate(command) {
        eprintln!("{}", format_error(&reason, mode));
        std::process::exit(126);
    }

    // Quick builtin check for simple commands (exit, help, etc)
    if let Some(tokens) = shlex::split(command) {
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
    }

    // Snapshot if mutating
    if SnapshotEngine::is_mutating_command(command) {
        let _ = snapshot_engine.pre_execution_snapshot(command);
    }

    // Execute: use POSIX sh for compound command evaluation in non-interactive mode.
    // The interactive shell uses OmniShellLang directly — no sh delegation.
    let start = std::time::Instant::now();
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .status()
        .unwrap_or_else(|e| {
            match e.kind() {
                std::io::ErrorKind::NotFound => {
                    eprintln!("omnishell: sh not found in PATH");
                    std::process::exit(127);
                }
                std::io::ErrorKind::PermissionDenied => {
                    eprintln!("omnishell: permission denied: sh");
                    std::process::exit(126);
                }
                _ => {
                    eprintln!("omnishell: execution failed: {}", e);
                    std::process::exit(1);
                }
            }
        });
    let duration = start.elapsed().as_millis() as u64;
    let exit_code = status.code().unwrap_or(1);

    if SnapshotEngine::is_mutating_command(command) {
        let _ = snapshot_engine.post_execution_snapshot(command, exit_code);
    }

    // Audit log
    let entry = AuditLogger::entry_for(command, mode)
        .exit_code(exit_code)
        .acl_verdict("allowed")
        .duration_ms(duration)
        .build();
    let _ = audit.log(entry);

    std::process::exit(exit_code);
}

/// Launch the interactive shell using shrs.
fn run_interactive_shell(mode: Mode, theme: &Theme) {
    use shrs::prelude::*;
    use shrs::readline::prompt::Prompt;
    use ::crossterm::style::Stylize;

    // Cache static env vars outside the prompt closure
    let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
    let hostname = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "localhost".to_string());
    let short_host = hostname.split('.').next().unwrap_or(&hostname).to_string();
    let prompt_template = theme.prompt.clone();
    let theme_name = theme.name.clone();

    let prompt = Prompt::from_sides(
        move || -> shrs_utils::StyledBuf {
            let cwd = shrs::readline::prompt::top_pwd();

            // TODO: wire git branch detection via gix when shell context tracks cwd
            let rendered = prompt_template
                .replace("{user}", &user)
                .replace("{host}", &short_host)
                .replace("{cwd}", &cwd)
                .replace("{mode}", &theme_name)
                .replace("{git_branch}", "")
                .replace("{emoji}", "");

            styled_buf!(
                rendered.cyan(),
            )
        },
        || -> shrs_utils::StyledBuf { styled_buf!() },
    );

    let completer = omnishell::completion::CompletionEngine::new(mode);

    let myshell = ShellBuilder::default()
        .with_lang(omnishell::lang::OmniShellLang::default())
        .with_state(omnishell::lang::FunctionTable::new())
        .with_state(omnishell::lang::ShellMode(mode))
        .with_state(omnishell::history::History::new(mode, omnishell::history::HistoryConfig::default()))
        .with_completer(completer)
        .with_prompt(prompt)
        .build()
        .unwrap();

    myshell.run().unwrap();
}

fn resolve_profile(config: &OmniShellConfig, cli_profile: Option<&str>) -> String {
    // 1. CLI flag takes absolute priority
    if let Some(name) = cli_profile {
        if config.profile.contains_key(name) {
            return name.to_string();
        }
        eprintln!("omnishell: requested profile '{}' not found, falling back.", name);
    }

    // 2. Check $USER binding
    if let Ok(username) = std::env::var("USER") {
        for (name, profile) in &config.profile {
            if profile.username.as_deref() == Some(&username) {
                return name.clone();
            }
        }
    }

    // 3. Default profile
    if let Some(ref default) = config.default_profile {
        return default.clone();
    }

    // 4. First available profile
    config.profile.keys().next().unwrap_or(&"default".to_string()).clone()
}
