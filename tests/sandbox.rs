//! Namespace sandbox tests for OmniShell.
//!
//! Tests the Linux namespace sandboxing configuration and path-based
//! access control. Actual namespace operations (clone, mount, chroot)
//! require CAP_SYS_ADMIN and are tested separately in privileged CI.

use omnishell::profile::Mode;
use omnishell::sandbox::{BindMount, ResourceLimit, RlimitResource, Sandbox, SandboxConfig};

// --- Configuration tests (all platforms) ---

#[test]
fn test_kids_sandbox_has_readonly_system_mounts() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));

    // Should bind-mount system dirs read-only
    let readonly_mounts: Vec<&BindMount> =
        config.bind_mounts.iter().filter(|m| m.read_only).collect();

    assert!(
        !readonly_mounts.is_empty(),
        "Kids sandbox should have read-only mounts"
    );

    let mount_dests: Vec<&str> = readonly_mounts
        .iter()
        .map(|m| m.destination.to_str().unwrap())
        .collect();

    assert!(mount_dests.contains(&"/usr"), "Should mount /usr read-only");
    assert!(mount_dests.contains(&"/bin"), "Should mount /bin read-only");
    assert!(mount_dests.contains(&"/lib"), "Should mount /lib read-only");
}

#[test]
fn test_kids_sandbox_has_resource_limits() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));

    assert!(
        !config.rlimits.is_empty(),
        "Kids sandbox should have resource limits"
    );

    let has_process_limit = config
        .rlimits
        .iter()
        .any(|r| matches!(r.resource, RlimitResource::Processes));
    let has_file_limit = config
        .rlimits
        .iter()
        .any(|r| matches!(r.resource, RlimitResource::OpenFiles));
    let has_size_limit = config
        .rlimits
        .iter()
        .any(|r| matches!(r.resource, RlimitResource::FileSize));

    assert!(has_process_limit, "Should limit processes");
    assert!(has_file_limit, "Should limit open files");
    assert!(has_size_limit, "Should limit file size");
}

#[test]
fn test_kids_sandbox_process_limit_is_reasonable() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));

    let process_limit = config
        .rlimits
        .iter()
        .find(|r| matches!(r.resource, RlimitResource::Processes))
        .unwrap();

    assert!(
        process_limit.hard <= 200,
        "Hard process limit should be <= 200"
    );
    assert!(
        process_limit.soft <= process_limit.hard,
        "Soft limit should not exceed hard"
    );
}

#[test]
fn test_kids_sandbox_file_size_limit_is_reasonable() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));

    let file_limit = config
        .rlimits
        .iter()
        .find(|r| matches!(r.resource, RlimitResource::FileSize))
        .unwrap();

    // Should be at most 100 MB
    assert!(
        file_limit.hard <= 100 * 1024 * 1024,
        "Hard file size limit should be <= 100MB"
    );
}

#[test]
fn test_kids_sandbox_has_pid_namespace() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));
    assert!(
        config.new_pid_namespace,
        "Kids sandbox should create new PID namespace"
    );
}

#[test]
fn test_kids_sandbox_has_network_namespace() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));
    assert!(
        config.new_network_namespace,
        "Kids sandbox should create new network namespace"
    );
}

#[test]
fn test_disabled_sandbox_has_no_namespaces() {
    let config = Sandbox::disabled();
    assert!(
        !config.new_pid_namespace,
        "Disabled sandbox should not have PID namespace"
    );
    assert!(
        !config.new_network_namespace,
        "Disabled sandbox should not have network namespace"
    );
}

#[test]
fn test_sandbox_config_serialization_roundtrip() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/test"));
    let json = serde_json::to_string(&config).unwrap();
    let parsed: SandboxConfig = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed.enabled, config.enabled);
    assert_eq!(parsed.new_pid_namespace, config.new_pid_namespace);
    assert_eq!(parsed.new_network_namespace, config.new_network_namespace);
    assert_eq!(parsed.bind_mounts.len(), config.bind_mounts.len());
    assert_eq!(parsed.rlimits.len(), config.rlimits.len());
}

#[test]
fn test_bind_mount_serialization() {
    let mount = BindMount {
        source: std::path::PathBuf::from("/usr"),
        destination: std::path::PathBuf::from("/usr"),
        read_only: true,
    };
    let json = serde_json::to_string(&mount).unwrap();
    let parsed: BindMount = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.source, mount.source);
    assert!(parsed.read_only);
}

#[test]
fn test_resource_limit_serialization() {
    let limit = ResourceLimit {
        resource: RlimitResource::Processes,
        soft: 50,
        hard: 100,
    };
    let json = serde_json::to_string(&limit).unwrap();
    let parsed: ResourceLimit = serde_json::from_str(&json).unwrap();
    assert!(matches!(parsed.resource, RlimitResource::Processes));
    assert_eq!(parsed.soft, 50);
    assert_eq!(parsed.hard, 100);
}

#[test]
fn test_rlimit_resource_serde_variants() {
    for resource in [
        RlimitResource::Cpu,
        RlimitResource::FileSize,
        RlimitResource::Data,
        RlimitResource::Stack,
        RlimitResource::Core,
        RlimitResource::ResidentSet,
        RlimitResource::Processes,
        RlimitResource::OpenFiles,
        RlimitResource::LockedMemory,
        RlimitResource::AddressSpace,
    ] {
        let json = serde_json::to_string(&resource).unwrap();
        let parsed: RlimitResource = serde_json::from_str(&json).unwrap();
        assert!(format!("{parsed:?}")
            .contains(format!("{resource:?}").split_whitespace().next().unwrap()));
    }
}

#[test]
fn test_config_for_mode_kids() {
    let config = Sandbox::config_for_mode(Mode::Kids, std::path::Path::new("/home/test"));
    assert!(config.enabled);
}

#[test]
fn test_config_for_mode_agent() {
    let config = Sandbox::config_for_mode(Mode::Agent, std::path::Path::new("/home/test"));
    assert!(!config.enabled);
}

#[test]
fn test_config_for_mode_admin() {
    let config = Sandbox::config_for_mode(Mode::Admin, std::path::Path::new("/home/test"));
    assert!(!config.enabled);
}

#[test]
fn test_sandbox_prepare_creates_root() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("sandbox_new");
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

#[test]
fn test_sandbox_prepare_disabled_is_noop() {
    let sandbox = Sandbox::new(Sandbox::disabled());
    assert!(sandbox.prepare().is_ok());
}

#[test]
fn test_sandbox_root_dir_in_home() {
    let config = Sandbox::kids_default(std::path::Path::new("/home/child"));
    assert!(
        config.root_dir.starts_with("/home/child"),
        "Sandbox root should be under home"
    );
    assert!(
        config.root_dir.to_str().unwrap().contains("sandbox"),
        "Sandbox root should contain 'sandbox'"
    );
}
