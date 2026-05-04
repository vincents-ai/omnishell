//! CLI argument parsing for OmniShell.

use clap::Parser;
use omnishell::Mode;

/// OmniShell CLI arguments.
#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "OmniShell: An intelligent, ACL-fortified shell"
)]
pub struct Args {
    /// Execution mode
    #[arg(short, long, value_enum, default_value_t = ShellMode::Admin)]
    pub mode: ShellMode,

    /// Profile name to use
    #[arg(short, long)]
    pub profile: Option<String>,

    /// Path to config file (overrides auto-discovery)
    #[arg(long)]
    pub config: Option<String>,

    /// Disable LLM features for this session
    #[arg(long)]
    pub no_llm: bool,

    /// Run a single command and exit (non-interactive)
    #[arg(short, long)]
    pub command: Option<String>,
}

#[derive(clap::ValueEnum, Clone, Debug, PartialEq)]
pub enum ShellMode {
    Kids,
    Agent,
    Admin,
}

impl From<ShellMode> for Mode {
    fn from(mode: ShellMode) -> Mode {
        match mode {
            ShellMode::Kids => Mode::Kids,
            ShellMode::Agent => Mode::Agent,
            ShellMode::Admin => Mode::Admin,
        }
    }
}
