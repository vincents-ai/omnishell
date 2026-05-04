//! Profile resolution logic for OmniShell.

use omnishell::OmniShellConfig;

/// Resolve which profile to use based on CLI flag, username binding, and defaults.
pub fn resolve_profile(config: &OmniShellConfig, cli_profile: Option<&str>) -> String {
    // 1. CLI flag takes absolute priority
    if let Some(name) = cli_profile {
        if config.profile.contains_key(name) {
            return name.to_string();
        }
        eprintln!("omnishell: requested profile '{name}' not found, falling back.");
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
    config
        .profile
        .keys()
        .next()
        .unwrap_or(&"default".to_string())
        .clone()
}
