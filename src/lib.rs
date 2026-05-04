//! OmniShell — An intelligent, ACL-fortified shell powered by shrs, llm, and gitoxide.
//!
//! OmniShell serves as both a standalone interactive shell and an embeddable library
//! for agentic-loop's tool system. It provides three execution modes (Kids, Agent, Admin)
//! with per-profile configuration for ACL rules, snapshots, LLM integration, and sandboxing.

pub mod acl;
pub mod audit;
pub mod builtins;
pub mod completion;
pub mod config;
pub mod engram_backend;
pub mod error;
pub mod history;
pub mod lang;
pub mod llm_integration;
pub mod output;
pub mod picker;
pub mod picture;
pub mod plugin;
pub mod profile;
#[cfg(target_os = "linux")]
pub mod sandbox;
pub mod snapshot;
pub mod theme;
pub mod undo;

pub use acl::{AclEngine, AclRule, ArgConstraint, Verdict};
pub use config::load_config;
pub use error::{OmniShellError, Result};
pub use plugin::{OmniShellBuilder, OmniShellPlugin, PluginContext, PluginMeta};
pub use profile::{Mode, OmniShellConfig, Profile};
pub use snapshot::{Snapshot, SnapshotEngine, SnapshotPhase};
pub use undo::UndoStack;
