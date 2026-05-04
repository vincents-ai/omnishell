//! OmniShell configuration types and loading.

use std::collections::HashMap;

use crate::theme::Theme;
use serde::{Deserialize, Serialize};

/// Top-level configuration container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmniShellConfig {
    /// All profiles keyed by name.
    pub profile: HashMap<String, Profile>,

    /// Default profile name.
    #[serde(default)]
    pub default_profile: Option<String>,

    /// Global LLM configuration (overridden by per-profile llm settings).
    #[serde(default)]
    pub llm: LlmConfig,

    /// Global ACL configuration.
    #[serde(default)]
    pub acl: Option<AclConfig>,
}

/// LLM provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    /// Whether LLM features are enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// LLM provider to use.
    /// Supported: "openai", "anthropic", "ollama", "custom"
    #[serde(default = "default_provider")]
    pub provider: String,

    /// Model identifier (provider-specific).
    /// Examples: "gpt-4o", "claude-sonnet-4-20250514", "llama3"
    #[serde(default = "default_model")]
    pub model: String,

    /// API base URL (for Ollama or custom providers).
    /// Default: provider's official API endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,

    /// API key. Recommended: use env var OMNISHELL_LLM_API_KEY instead.
    #[serde(skip_serializing)]
    pub api_key: Option<String>,

    /// Generation temperature (0.0 - 2.0).
    #[serde(default = "default_temperature")]
    pub temperature: f32,

    /// Maximum tokens to generate per request.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,

    /// Request timeout in seconds.
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_provider() -> String {
    "openai".to_string()
}
fn default_model() -> String {
    "gpt-4o".to_string()
}
fn default_temperature() -> f32 {
    0.7
}
fn default_max_tokens() -> u32 {
    1024
}
fn default_timeout() -> u64 {
    30
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_provider(),
            model: default_model(),
            api_base: None,
            api_key: None,
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            timeout_secs: default_timeout(),
        }
    }
}

/// ACL configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AclConfig {
    /// Additional commands to allow (added to mode defaults).
    #[serde(default)]
    pub extra_allow: Vec<String>,

    /// Additional commands to block (added to mode defaults).
    #[serde(default)]
    pub extra_block: Vec<String>,
}

/// A single execution profile.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Execution mode.
    pub mode: Mode,

    /// OS username this profile binds to (auto-selected on login).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Display name for interactive picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Age (Kids mode, drives LLM tutor tone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<u8>,

    /// Per-profile LLM overrides (merge with global llm config).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm: Option<LlmConfig>,

    /// Per-profile ACL overrides.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acl: Option<AclConfig>,

    /// Per-profile theme overrides (prompt, colors, emoji).
    /// If not set, the mode default theme is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeOverride>,

    /// Environment variable allowlist for spawned commands.
    /// If set, only these vars are passed to child processes.
    /// If not set, all vars are passed (unless filtered by env_deny).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_allow: Option<Vec<String>>,

    /// Environment variable denylist for spawned commands.
    /// These vars are removed from the child process environment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env_deny: Option<Vec<String>>,

    /// Shell aliases for this profile (name -> expansion).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub aliases: HashMap<String, String>,

    /// System prompt configuration mode.
    #[serde(default)]
    pub system_prompt_mode: SystemPromptMode,

    /// Custom system prompt text (used when mode is Override or Append).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_extra: Option<String>,
}

/// System prompt configuration mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SystemPromptMode {
    /// Use the built-in template for the mode.
    #[default]
    Default,
    /// Replace the entire system prompt with custom text.
    Override,
    /// Append extra instructions to the default template.
    Append,
}

/// Theme overrides for a profile.
/// Any field set here replaces the mode default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeOverride {
    /// PS1 prompt template.
    /// Supports: {user}, {host}, {cwd}, {mode}, {git_branch}, {emoji}
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Mode emoji.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

impl Profile {
    /// Build the resolved Theme for this profile, applying overrides
    /// on top of the mode default.
    pub fn theme(&self) -> Theme {
        let mut theme = Theme::for_mode(self.mode);
        if let Some(ref overrides) = self.theme {
            if let Some(ref prompt) = overrides.prompt {
                theme.prompt = prompt.clone();
            }
            if let Some(ref emoji) = overrides.emoji {
                theme.emoji = Some(emoji.clone());
            }
        }
        theme
    }

    /// Filter environment variables for child processes.
    /// Returns a HashMap suitable for Command::envs().
    pub fn filtered_env(&self) -> HashMap<String, String> {
        let all: HashMap<String, String> = std::env::vars().collect();

        let mut result = match &self.env_allow {
            Some(allow) => {
                // Only include explicitly allowed vars
                all.into_iter()
                    .filter(|(k, _)| allow.iter().any(|a| a == k))
                    .collect()
            }
            None => all,
        };

        if let Some(deny) = &self.env_deny {
            for d in deny {
                result.remove(d);
            }
        }

        result
    }
}

/// Execution mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Admin,
    Kids,
    Agent,
}

impl Mode {
    /// Default Kids profile with strict env var allowlist.
    pub fn kids_profile() -> Profile {
        Profile {
            mode: Mode::Kids,
            username: None,
            display_name: Some("Kids".to_string()),
            age: Some(10),
            llm: None,
            acl: None,
            theme: None,
            env_allow: Some(vec![
                "PATH".to_string(),
                "HOME".to_string(),
                "USER".to_string(),
                "TERM".to_string(),
                "LANG".to_string(),
                "XDG_CONFIG_HOME".to_string(),
                "XDG_DATA_HOME".to_string(),
            ]),
            env_deny: None,
            aliases: HashMap::from([
                ("dir".to_string(), "ls".to_string()),
                ("cls".to_string(), "clear".to_string()),
                ("copy".to_string(), "cp".to_string()),
                ("move".to_string(), "mv".to_string()),
                ("del".to_string(), "rm -i".to_string()),
                ("type".to_string(), "cat".to_string()),
            ]),
            system_prompt_mode: SystemPromptMode::Default,
            system_prompt_extra: None,
        }
    }
}

impl Default for OmniShellConfig {
    fn default() -> Self {
        let mut profile = HashMap::new();
        profile.insert("default".to_string(), Profile::default());
        Self {
            profile,
            default_profile: Some("default".to_string()),
            llm: LlmConfig::default(),
            acl: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_has_llm() {
        let config = OmniShellConfig::default();
        assert!(config.llm.enabled);
        assert_eq!(config.llm.provider, "openai");
        assert_eq!(config.llm.model, "gpt-4o");
    }

    #[test]
    fn test_llm_config_with_ollama() {
        let toml_str = r#"
provider = "ollama"
model = "llama3"
api_base = "http://localhost:11434"
temperature = 0.5
max_tokens = 2048
"#;
        let llm: LlmConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(llm.provider, "ollama");
        assert_eq!(llm.model, "llama3");
        assert_eq!(llm.api_base, Some("http://localhost:11434".to_string()));
        assert_eq!(llm.temperature, 0.5);
        assert_eq!(llm.max_tokens, 2048);
    }

    #[test]
    fn test_llm_config_with_anthropic() {
        let toml_str = r#"
provider = "anthropic"
model = "claude-sonnet-4-20250514"
"#;
        let llm: LlmConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(llm.provider, "anthropic");
        assert_eq!(llm.model, "claude-sonnet-4-20250514");
    }

    #[test]
    fn test_profile_with_llm_override() {
        let toml_str = r#"
[profile.kids]
mode = "kids"
age = 7

[profile.kids.llm]
provider = "ollama"
model = "llama3"
temperature = 0.3
max_tokens = 256
"#;
        let config: OmniShellConfig = toml::from_str(toml_str).unwrap();
        let kids = &config.profile["kids"];
        assert_eq!(kids.mode, Mode::Kids);
        let kids_llm = kids.llm.as_ref().unwrap();
        assert_eq!(kids_llm.model, "llama3");
        assert_eq!(kids_llm.temperature, 0.3);
        assert_eq!(kids_llm.max_tokens, 256);
    }

    #[test]
    fn test_llm_config_serialization_roundtrip() {
        let config = LlmConfig {
            enabled: true,
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            api_base: None,
            api_key: None,
            temperature: 0.8,
            max_tokens: 4096,
            timeout_secs: 60,
        };
        let json = serde_json::to_string(&config).unwrap();
        let parsed: LlmConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.provider, config.provider);
        assert_eq!(parsed.model, config.model);
        assert!((parsed.temperature - config.temperature).abs() < f32::EPSILON);
    }

    #[test]
    fn test_profile_theme_default() {
        let profile = Profile::default();
        let theme = profile.theme();
        assert_eq!(theme.name, "admin");
        assert!(theme.prompt.contains("{user}@{host}"));
    }

    #[test]
    fn test_profile_theme_prompt_override() {
        let profile = Profile {
            theme: Some(ThemeOverride {
                prompt: Some("custom> ".to_string()),
                emoji: None,
            }),
            ..Default::default()
        };
        let theme = profile.theme();
        assert_eq!(theme.prompt, "custom> ");
    }

    #[test]
    fn test_profile_theme_emoji_override() {
        let mut profile = Profile {
            mode: Mode::Kids,
            ..Default::default()
        };
        profile.theme = Some(ThemeOverride {
            prompt: None,
            emoji: Some("🎮".to_string()),
        });
        let theme = profile.theme();
        assert_eq!(theme.emoji.as_deref(), Some("🎮"));
        // Prompt should still be the kids default
        assert!(theme.prompt.contains("🐚"));
    }

    #[test]
    fn test_theme_override_deserialization() {
        let toml_str = r#"
mode = "agent"
[theme]
prompt = "[{mode}] {cwd}> "
emoji = "🛸"
"#;
        let profile: Profile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.mode, Mode::Agent);
        let theme = profile.theme();
        assert_eq!(theme.prompt, "[{mode}] {cwd}> ");
        assert_eq!(theme.emoji.as_deref(), Some("🛸"));
    }

    #[test]
    fn test_api_key_not_serialized() {
        let config = LlmConfig {
            api_key: Some("secret-key".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("secret-key"),
            "API key must not appear in serialized output"
        );
    }
}
