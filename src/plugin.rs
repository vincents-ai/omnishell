//! OmniShell plugin system.
//!
//! Provides a trait-based plugin architecture where plugins can:
//! - Hook into shell lifecycle (init, shutdown)
//! - Intercept commands before/after execution
//! - Extend tab completion
//! - Register custom built-in commands
//!
//! Plugins are registered via OmniShellBuilder and initialized in order.

use std::any::Any;
use std::path::Path;

use serde::{Deserialize, Serialize};
use crate::profile::Profile;
use crate::OmniShellConfig;

/// Plugin metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginMeta {
    /// Unique plugin name.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// Human-readable description.
    pub description: String,
}

/// Context provided to plugins for accessing shell state.
pub struct PluginContext<'a> {
    /// The active profile.
    pub profile: &'a Profile,
    /// The full config.
    pub config: &'a OmniShellConfig,
    /// Current working directory.
    pub working_dir: &'a Path,
}

/// Result of a plugin's before_command hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginCommandAction {
    /// Allow the command to proceed.
    Allow,
    /// Block the command with a reason.
    Deny(String),
    /// Modify the command (replace with a different one).
    Replace(String),
}

/// The core plugin trait. All methods have default no-op implementations.
pub trait OmniShellPlugin: Send + Sync + 'static {
    /// Return plugin metadata.
    fn meta(&self) -> PluginMeta;

    /// Called once when the shell starts up.
    fn on_init(&self, _ctx: &PluginContext) -> Result<(), String> {
        Ok(())
    }

    /// Called once when the shell shuts down.
    fn on_shutdown(&self, _ctx: &PluginContext) {}

    /// Called before a command is executed. Return Deny to block it.
    fn on_before_command(&self, _command: &str, _ctx: &PluginContext) -> PluginCommandAction {
        PluginCommandAction::Allow
    }

    /// Called after a command completes.
    fn on_after_command(&self, _command: &str, _exit_code: i32, _ctx: &PluginContext) {}

    /// Called to add completion candidates. Return additional candidates.
    fn on_complete(&self, _partial: &str, _ctx: &PluginContext) -> Vec<String> {
        Vec::new()
    }

    /// Downcast to concrete type for plugin-specific configuration.
    fn as_any(&self) -> &dyn Any {
        // Default: no downcast support
        &()
    }
}

/// Builder for constructing an OmniShell instance with plugins and configuration.
pub struct OmniShellBuilder {
    pub config: OmniShellConfig,
    pub plugins: Vec<Box<dyn OmniShellPlugin>>,
    /// Whether to fail on plugin initialization errors.
    pub fail_on_plugin_error: bool,
}

impl OmniShellBuilder {
    /// Create a new builder with the given config.
    pub fn new(config: OmniShellConfig) -> Self {
        Self {
            config,
            plugins: Vec::new(),
            fail_on_plugin_error: false,
        }
    }

    /// Add a plugin.
    pub fn with_plugin(mut self, plugin: impl OmniShellPlugin) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    /// Set whether plugin init failures should abort shell startup.
    pub fn fail_on_plugin_error(mut self, fail: bool) -> Self {
        self.fail_on_plugin_error = fail;
        self
    }

    /// Initialize all plugins. Returns a list of (plugin_name, error) for any that failed.
    pub fn init_plugins(&self, profile: &Profile, working_dir: &Path) -> Vec<(String, String)> {
        let ctx = PluginContext {
            profile,
            config: &self.config,
            working_dir,
        };

        let mut errors = Vec::new();
        for plugin in &self.plugins {
            let meta = plugin.meta();
            if let Err(e) = plugin.on_init(&ctx) {
                errors.push((meta.name.clone(), e));
            }
        }
        errors
    }

    /// Run before_command hooks for all plugins. Returns the first Deny or Replace.
    pub fn before_command(&self, command: &str, profile: &Profile, working_dir: &Path) -> PluginCommandAction {
        let ctx = PluginContext {
            profile,
            config: &self.config,
            working_dir,
        };

        for plugin in &self.plugins {
            match plugin.on_before_command(command, &ctx) {
                PluginCommandAction::Allow => continue,
                action => return action,
            }
        }

        PluginCommandAction::Allow
    }

    /// Run after_command hooks for all plugins.
    pub fn after_command(&self, command: &str, exit_code: i32, profile: &Profile, working_dir: &Path) {
        let ctx = PluginContext {
            profile,
            config: &self.config,
            working_dir,
        };

        for plugin in &self.plugins {
            plugin.on_after_command(command, exit_code, &ctx);
        }
    }

    /// Collect completion candidates from all plugins.
    pub fn completions(&self, partial: &str, profile: &Profile, working_dir: &Path) -> Vec<String> {
        let ctx = PluginContext {
            profile,
            config: &self.config,
            working_dir,
        };

        let mut candidates = Vec::new();
        for plugin in &self.plugins {
            candidates.extend(plugin.on_complete(partial, &ctx));
        }
        candidates
    }

    /// Get plugin count.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }

    /// Get plugin metadata for all registered plugins.
    pub fn plugin_metas(&self) -> Vec<PluginMeta> {
        self.plugins.iter().map(|p| p.meta()).collect()
    }
}

/// A simple example plugin for testing.
pub struct EchoPlugin {
    pub prefix: String,
}

impl OmniShellPlugin for EchoPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "echo".to_string(),
            version: "0.1.0".to_string(),
            description: "Echo plugin for testing".to_string(),
        }
    }

    fn on_before_command(&self, command: &str, _ctx: &PluginContext) -> PluginCommandAction {
        if command.starts_with("echo ") {
            PluginCommandAction::Allow
        } else {
            PluginCommandAction::Allow
        }
    }
}

/// A plugin that blocks commands matching a pattern.
pub struct BlocklistPlugin {
    pub blocked: Vec<String>,
}

impl OmniShellPlugin for BlocklistPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "blocklist".to_string(),
            version: "0.1.0".to_string(),
            description: "Blocks configured commands".to_string(),
        }
    }

    fn on_before_command(&self, command: &str, _ctx: &PluginContext) -> PluginCommandAction {
        let cmd_name = command.split_whitespace().next().unwrap_or("");
        if self.blocked.iter().any(|b| b == cmd_name) {
            PluginCommandAction::Deny(format!("Command '{}' blocked by blocklist plugin", cmd_name))
        } else {
            PluginCommandAction::Allow
        }
    }
}

/// A plugin that logs all commands.
pub struct AuditPlugin {
    pub log: std::sync::Mutex<Vec<String>>,
}

impl AuditPlugin {
    pub fn new() -> Self {
        Self {
            log: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl OmniShellPlugin for AuditPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "audit".to_string(),
            version: "0.1.0".to_string(),
            description: "Logs all commands".to_string(),
        }
    }

    fn on_after_command(&self, command: &str, exit_code: i32, _ctx: &PluginContext) {
        if let Ok(mut log) = self.log.lock() {
            log.push(format!("{} [exit={}]", command, exit_code));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::Mode;

    fn make_profile() -> Profile {
        Profile { mode: Mode::Admin, ..Default::default() }
    }

    fn make_config() -> OmniShellConfig {
        OmniShellConfig::default()
    }

    #[test]
    fn test_builder_add_plugins() {
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(EchoPlugin { prefix: ">".to_string() })
            .with_plugin(BlocklistPlugin { blocked: vec!["rm".to_string()] });

        assert_eq!(builder.plugin_count(), 2);
        let metas = builder.plugin_metas();
        assert_eq!(metas[0].name, "echo");
        assert_eq!(metas[1].name, "blocklist");
    }

    #[test]
    fn test_before_command_allow() {
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(EchoPlugin { prefix: ">".to_string() });

        let profile = make_profile();
        let action = builder.before_command("ls", &profile, Path::new("/tmp"));
        assert_eq!(action, PluginCommandAction::Allow);
    }

    #[test]
    fn test_before_command_deny() {
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(BlocklistPlugin { blocked: vec!["rm".to_string()] });

        let profile = make_profile();
        let action = builder.before_command("rm file.txt", &profile, Path::new("/tmp"));
        assert!(matches!(action, PluginCommandAction::Deny(_)));
    }

    #[test]
    fn test_before_command_multiple_plugins_first_deny_wins() {
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(BlocklistPlugin { blocked: vec!["rm".to_string()] })
            .with_plugin(EchoPlugin { prefix: ">".to_string() });

        let profile = make_profile();
        let action = builder.before_command("rm file.txt", &profile, Path::new("/tmp"));
        assert!(matches!(action, PluginCommandAction::Deny(_)));
    }

    #[test]
    fn test_after_command_audit() {
        let audit = AuditPlugin::new();
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(audit);

        let profile = make_profile();
        builder.after_command("ls -la", 0, &profile, Path::new("/tmp"));
        builder.after_command("cargo build", 0, &profile, Path::new("/tmp"));

        // Check the log was populated (need to access the plugin)
        // Since we can't easily access it after registration, test the plugin directly
    }

    #[test]
    fn test_audit_plugin_direct() {
        let plugin = AuditPlugin::new();
        let profile = make_profile();
        let config = make_config();
        let ctx = PluginContext {
            profile: &profile,
            config: &config,
            working_dir: Path::new("/tmp"),
        };

        plugin.on_after_command("ls", 0, &ctx);
        plugin.on_after_command("rm", 1, &ctx);

        let log = plugin.log.lock().unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0], "ls [exit=0]");
        assert_eq!(log[1], "rm [exit=1]");
    }

    #[test]
    fn test_init_plugins() {
        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(EchoPlugin { prefix: ">".to_string() });

        let profile = make_profile();
        let errors = builder.init_plugins(&profile, Path::new("/tmp"));
        assert!(errors.is_empty());
    }

    #[test]
    fn test_completions_from_plugins() {
        struct CompletePlugin;
        impl OmniShellPlugin for CompletePlugin {
            fn meta(&self) -> PluginMeta {
                PluginMeta { name: "complete".to_string(), version: "0.1.0".to_string(), description: "test".to_string() }
            }
            fn on_complete(&self, partial: &str, _ctx: &PluginContext) -> Vec<String> {
                if partial.starts_with("ca") {
                    vec!["cargo".to_string(), "cat".to_string()]
                } else {
                    vec![]
                }
            }
        }

        let builder = OmniShellBuilder::new(make_config())
            .with_plugin(CompletePlugin);

        let profile = make_profile();
        let completions = builder.completions("ca", &profile, Path::new("/tmp"));
        assert_eq!(completions.len(), 2);
        assert!(completions.contains(&"cargo".to_string()));
        assert!(completions.contains(&"cat".to_string()));
    }

    #[test]
    fn test_plugin_meta_serialization() {
        let meta = PluginMeta {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            description: "A test plugin".to_string(),
        };
        let json = serde_json::to_string(&meta).unwrap();
        assert!(json.contains("\"name\":\"test\""));
        let parsed: PluginMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
    }
}
