//! OmniShell plugin system.

use std::path::Path;

use crate::profile::Profile;
use crate::OmniShellConfig;

/// Plugin metadata.
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub name: String,
    pub version: String,
    pub description: String,
}

/// Context provided to plugins for accessing shell state.
pub struct PluginContext<'a> {
    pub profile: &'a Profile,
    pub config: &'a OmniShellConfig,
    pub working_dir: &'a Path,
}

/// The core plugin trait. All methods have default no-op implementations.
pub trait OmniShellPlugin: Send + Sync + 'static {
    /// Return plugin metadata.
    fn meta(&self) -> PluginMeta;

    /// Called once when the shell starts up.
    fn on_init(&self, _ctx: &PluginContext) {}

    /// Called once when the shell shuts down.
    fn on_shutdown(&self, _ctx: &PluginContext) {}
}

/// Builder for constructing an OmniShell instance with plugins and configuration.
pub struct OmniShellBuilder {
    pub config: OmniShellConfig,
    pub plugins: Vec<Box<dyn OmniShellPlugin>>,
}

impl OmniShellBuilder {
    pub fn new(config: OmniShellConfig) -> Self {
        Self {
            config,
            plugins: Vec::new(),
        }
    }

    pub fn with_plugin(mut self, plugin: impl OmniShellPlugin) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    // TODO: build() will construct the full shell once shrs integration is complete
}
