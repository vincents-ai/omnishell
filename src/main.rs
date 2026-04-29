use clap::Parser;
use omnishell::{OmniShellBuilder, OmniShellConfig, load_config};

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
    let config_path = args.config.as_deref().map(std::path::Path::new);
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

    eprintln!("OmniShell starting in {:?} mode with profile '{}'", profile.mode, profile_name);

    // Build OmniShell
    let _builder = OmniShellBuilder::new(config);

    // TODO: Wire shrs shell once fork is complete
    eprintln!("OmniShell is scaffolding. Full shell integration pending shrs fork.");
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
