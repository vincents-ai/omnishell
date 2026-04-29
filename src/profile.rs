//! OmniShell configuration types and loading.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Top-level configuration container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OmniShellConfig {
    /// All profiles keyed by name.
    pub profile: HashMap<String, Profile>,

    /// Default profile name.
    #[serde(default)]
    pub default_profile: Option<String>,
}

/// A single execution profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Execution mode.
    pub mode: Mode,

    /// OS username this profile binds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Display name for interactive picker.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,

    /// Age (Kids mode, drives LLM tutor tone).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age: Option<u8>,
}

/// Execution mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Kids,
    Agent,
    Admin,
}

impl Default for OmniShellConfig {
    fn default() -> Self {
        let mut profile = HashMap::new();
        profile.insert(
            "default".to_string(),
            Profile {
                mode: Mode::Admin,
                username: None,
                display_name: None,
                age: None,
            },
        );
        Self {
            profile,
            default_profile: Some("default".to_string()),
        }
    }
}
