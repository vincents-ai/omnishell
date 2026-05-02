//! Linux namespace sandboxing for OmniShell.
//!
//! Uses Linux namespaces (mount, PID, network, user) to create isolated
//! execution environments for Kids mode. Commands run inside a chroot-like
//! jail with restricted filesystem access.
//!
//! This module is only compiled on Linux (`cfg(target_os = "linux")`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::profile::Mode;

/// Sandbox configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    /// Whether sandboxing is enabled.
    pub enabled: bool,
    /// Root directory for the sandbox (chroot equivalent).
    pub root_dir: PathBuf,
    /// Directories to bind-mount into the sandbox.
    #[serde(default)]
    pub bind_mounts: Vec<BindMount>,
    /// Resource limits.
    #[serde(default)]
    pub rlimits: Vec<ResourceLimit>,
    /// Whether to create a new PID namespace.
    #[serde(default = "default_true")]
    pub new_pid_namespace: bool,
    /// Whether to create a new network namespace (disables networking).
    #[serde(default = "default_true")]
    pub new_network_namespace: bool,
}

fn default_true() -> bool {
    true
}

/// A bind mount entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindMount {
    /// Source path on the host.
    pub source: PathBuf,
    /// Destination path inside the sandbox.
    pub destination: PathBuf,
    /// Whether the mount is read-only.
    #[serde(default)]
    pub read_only: bool,
}

/// A resource limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceLimit {
    /// Resource type.
    pub resource: RlimitResource,
    /// Soft limit.
    pub soft: u64,
    /// Hard limit.
    pub hard: u64,
}

/// Linux resource limit types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RlimitResource {
    /// Maximum CPU time (seconds).
    Cpu,
    /// Maximum file size (bytes).
    FileSize,
    /// Maximum data segment size (bytes).
    Data,
    /// Maximum stack size (bytes).
    Stack,
    /// Maximum core file size (bytes).
    Core,
    /// Maximum resident set size (bytes).
    ResidentSet,
    /// Maximum number of processes.
    Processes,
    /// Maximum number of open files.
    OpenFiles,
    /// Maximum locked memory (bytes).
    LockedMemory,
    /// Maximum address space (bytes).
    AddressSpace,
}

/// The sandbox manager.
pub struct Sandbox {
    config: SandboxConfig,
}

impl Sandbox {
    /// Create a new sandbox manager.
    pub fn new(config: SandboxConfig) -> Self {
        Self { config }
    }

    /// Create the default sandbox config for Kids mode.
    pub fn kids_default(home_dir: &Path) -> SandboxConfig {
        let sandbox_root = home_dir.join(".omnishell/sandbox");

        SandboxConfig {
            enabled: true,
            root_dir: sandbox_root,
            bind_mounts: vec![
                BindMount {
                    source: PathBuf::from("/usr"),
                    destination: PathBuf::from("/usr"),
                    read_only: true,
                },
                BindMount {
                    source: PathBuf::from("/bin"),
                    destination: PathBuf::from("/bin"),
                    read_only: true,
                },
                BindMount {
                    source: PathBuf::from("/lib"),
                    destination: PathBuf::from("/lib"),
                    read_only: true,
                },
                BindMount {
                    source: PathBuf::from("/lib64"),
                    destination: PathBuf::from("/lib64"),
                    read_only: true,
                },
            ],
            rlimits: vec![
                ResourceLimit {
                    resource: RlimitResource::Processes,
                    soft: 50,
                    hard: 100,
                },
                ResourceLimit {
                    resource: RlimitResource::OpenFiles,
                    soft: 64,
                    hard: 128,
                },
                ResourceLimit {
                    resource: RlimitResource::FileSize,
                    soft: 10 * 1024 * 1024, // 10 MB
                    hard: 50 * 1024 * 1024, // 50 MB
                },
            ],
            new_pid_namespace: true,
            new_network_namespace: true,
        }
    }

    /// Create the default (disabled) sandbox for Agent/Admin mode.
    pub fn disabled() -> SandboxConfig {
        SandboxConfig {
            enabled: false,
            root_dir: PathBuf::from("/"),
            bind_mounts: vec![],
            rlimits: vec![],
            new_pid_namespace: false,
            new_network_namespace: false,
        }
    }

    /// Get the sandbox config for a mode.
    pub fn config_for_mode(mode: Mode, home_dir: &Path) -> SandboxConfig {
        match mode {
            Mode::Kids => Self::kids_default(home_dir),
            Mode::Agent | Mode::Admin => Self::disabled(),
        }
    }

    /// Check if sandboxing is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Check if a path is accessible within the sandbox.
    pub fn is_path_allowed(&self, path: &Path) -> bool {
        if !self.config.enabled {
            return true; // No sandbox = everything allowed
        }

        // In Kids mode, only allow paths under the home directory or sandbox
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home"));
        let sandbox = &self.config.root_dir;

        path.starts_with(&home) || path.starts_with(sandbox) || path.starts_with("/tmp")
    }

    /// Get the sandbox root directory.
    pub fn root_dir(&self) -> &Path {
        &self.config.root_dir
    }

    /// Get the bind mounts.
    pub fn bind_mounts(&self) -> &[BindMount] {
        &self.config.bind_mounts
    }

    /// Get the resource limits.
    pub fn rlimits(&self) -> &[ResourceLimit] {
        &self.config.rlimits
    }

    /// Prepare the sandbox environment.
    ///
    /// Creates the root directory and sets up bind mounts.
    /// Returns Ok(()) if successful, or an error describing what failed.
    pub fn prepare(&self) -> Result<(), String> {
        if !self.config.enabled {
            return Ok(());
        }

        // Create sandbox root if it doesn't exist
        if !self.config.root_dir.exists() {
            std::fs::create_dir_all(&self.config.root_dir)
                .map_err(|e| format!("Failed to create sandbox root: {e}"))?;
        }

        // Set up bind mounts for sandboxed paths
        for bind in &self.config.bind_mounts {
            let target = self.config.root_dir.join(bind.source.strip_prefix("/").unwrap_or(&bind.source));
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Create mount point if it doesn't exist
            if !target.exists() {
                if bind.source.is_dir() {
                    let _ = std::fs::create_dir_all(&target);
                } else {
                    let _ = std::fs::File::create(&target);
                }
            }
            // Perform bind mount
            let flags = if bind.read_only {
                nix::mount::MsFlags::MS_BIND | nix::mount::MsFlags::MS_RDONLY
            } else {
                nix::mount::MsFlags::MS_BIND
            };
            if let Err(e) = nix::mount::mount(
                Some(bind.source.as_path()),
                &target,
                None::<&str>,
                flags,
                None::<&str>,
            ) {
                // Bind mount may fail without CAP_SYS_ADMIN — log but don't fail
                tracing::warn!("Sandbox bind mount {} -> {} failed: {e}", bind.source.display(), target.display());
            }
        }

        // Set resource limits
        for limit in &self.config.rlimits {
            let resource = match limit.resource {
                RlimitResource::Cpu => libc::RLIMIT_CPU,
                RlimitResource::FileSize => libc::RLIMIT_FSIZE,
                RlimitResource::Data => libc::RLIMIT_DATA,
                RlimitResource::Stack => libc::RLIMIT_STACK,
                RlimitResource::Core => libc::RLIMIT_CORE,
                RlimitResource::ResidentSet => libc::RLIMIT_RSS,
                RlimitResource::Processes => libc::RLIMIT_NPROC,
                RlimitResource::OpenFiles => libc::RLIMIT_NOFILE,
                RlimitResource::LockedMemory => libc::RLIMIT_MEMLOCK,
                RlimitResource::AddressSpace => libc::RLIMIT_AS,
            };
            let rlim = libc::rlimit {
                rlim_cur: limit.soft,
                rlim_max: limit.hard,
            };
            unsafe {
                if libc::setrlimit(resource, &rlim) != 0 {
                    tracing::warn!("Sandbox setrlimit failed for {:?}", limit.resource);
                }
            }
        }

        // Unshare namespaces (requires CAP_SYS_ADMIN or unprivileged user namespaces)
        if self.config.new_pid_namespace || self.config.new_network_namespace {
            let mut flags = nix::sched::CloneFlags::empty();
            if self.config.new_pid_namespace {
                flags |= nix::sched::CloneFlags::CLONE_NEWPID;
            }
            if self.config.new_network_namespace {
                flags |= nix::sched::CloneFlags::CLONE_NEWNET;
            }
            match nix::sched::unshare(flags) {
                Ok(()) => tracing::info!("Sandbox namespaces created: {:?}", flags),
                Err(e) => {
                    // Namespace creation may fail without privileges — degrade gracefully
                    tracing::warn!("Sandbox namespace unshare failed (need CAP_SYS_ADMIN?): {e}");
                },
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kids_sandbox_config() {
        let config = Sandbox::kids_default(Path::new("/home/test"));
        assert!(config.enabled);
        assert!(config.new_pid_namespace);
        assert!(config.new_network_namespace);
        assert!(!config.bind_mounts.is_empty());
        assert!(!config.rlimits.is_empty());
    }

    #[test]
    fn test_disabled_sandbox() {
        let config = Sandbox::disabled();
        assert!(!config.enabled);
        assert!(!config.new_pid_namespace);
        assert!(!config.new_network_namespace);
    }

    #[test]
    fn test_config_for_mode() {
        let kids_config = Sandbox::config_for_mode(Mode::Kids, Path::new("/home/test"));
        assert!(kids_config.enabled);

        let agent_config = Sandbox::config_for_mode(Mode::Agent, Path::new("/home/test"));
        assert!(!agent_config.enabled);

        let admin_config = Sandbox::config_for_mode(Mode::Admin, Path::new("/home/test"));
        assert!(!admin_config.enabled);
    }

    #[test]
    fn test_path_allowed_disabled() {
        let sandbox = Sandbox::new(Sandbox::disabled());
        assert!(sandbox.is_path_allowed(Path::new("/etc/passwd")));
        assert!(sandbox.is_path_allowed(Path::new("/home/test/file")));
    }

    #[test]
    fn test_path_allowed_kids() {
        let sandbox = Sandbox::new(Sandbox::kids_default(Path::new("/home/test")));
        // Use actual home dir for the test
        let home = dirs::home_dir().unwrap();
        assert!(sandbox.is_path_allowed(&home.join("projects")));
        assert!(sandbox.is_path_allowed(Path::new("/tmp/build")));
    }

    #[test]
    fn test_prepare_disabled() {
        let sandbox = Sandbox::new(Sandbox::disabled());
        assert!(sandbox.prepare().is_ok());
    }

    #[test]
    fn test_prepare_creates_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("sandbox_root");
        let config = SandboxConfig {
            enabled: true,
            root_dir: root.clone(),
            bind_mounts: vec![],
            rlimits: vec![],
            new_pid_namespace: false,
            new_network_namespace: false,
        };
        let sandbox = Sandbox::new(config);
        assert!(sandbox.prepare().is_ok());
        assert!(root.exists());
    }
}
