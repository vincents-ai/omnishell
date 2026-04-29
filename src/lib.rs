//! OmniShell — An intelligent, ACL-fortified shell powered by shrs, llm, and gitoxide.
//!
//! OmniShell serves as both a standalone interactive shell and an embeddable library
//! for agentic-loop's tool system. It provides three execution modes (Kids, Agent, Admin)
//! with per-profile configuration for ACL rules, snapshots, LLM integration, and sandboxing.

pub mod acl;
pub mod audit;
pub mod config;
pub mod error;
pub mod plugin;
pub mod profile;
pub mod sandbox;
pub mod snapshot;
pub mod llm_integration;
pub mod builtins;
pub mod output;
pub mod completion;
pub mod history;
pub mod engram_backend;
pub mod undo;
pub mod picker;
pub mod theme;

pub use acl::{AclEngine, AclRule, ArgConstraint, Verdict};
pub use config::load_config;
pub use error::{OmniShellError, Result};
pub use plugin::{OmniShellPlugin, PluginMeta, PluginContext, OmniShellBuilder};
pub use profile::{OmniShellConfig, Profile, Mode};
pub use snapshot::{SnapshotEngine, Snapshot, SnapshotPhase};
pub use undo::UndoStack;
