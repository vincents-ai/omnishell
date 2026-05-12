//! Shell context — subsystems available to the interactive eval loop.
//!
//! Bundles snapshot engine, audit logger, undo stack, engram context,
//! and plugin hooks into a single shrs State. This is the single point
//! of wiring that makes all subsystems available in `lang_impl.rs`.

use std::sync::{Arc, Mutex};

use crate::audit::{AuditConfig, AuditLogger};
use crate::engram_backend::EngramContext;
use crate::profile::Mode;
use crate::snapshot::SnapshotEngine;
use crate::undo::UndoStack;

/// The current shell execution mode, stored in shrs States.
#[derive(Debug, Clone, Copy)]
pub struct ShellMode(pub Mode);

impl Default for ShellMode {
    fn default() -> Self {
        Self(Mode::Admin)
    }
}

/// Shell subsystems available to the interactive eval loop.
///
/// Wrapped in `Arc<Mutex<>>` for interior mutability since shrs States
/// are accessed via shared references (`states.get()`).
pub struct ShellContext {
    /// Snapshot engine for pre/post execution snapshots.
    pub snapshot_engine: Arc<Mutex<SnapshotEngine>>,
    /// Undo stack for undo/redo of mutating commands.
    pub undo_stack: Arc<Mutex<UndoStack>>,
    /// Audit logger for structured command logging.
    pub audit_logger: AuditLogger,
    /// Engram context for LLM task context injection.
    pub engram_context: EngramContext,
    /// Current working directory at shell startup.
    pub working_dir: String,
    /// Shared theme name — updated on mode switch, read by prompt closure.
    pub current_theme_name: Arc<Mutex<String>>,
}

impl ShellContext {
    /// Create a new shell context with all subsystems.
    pub fn new(
        _mode: Mode,
        working_dir: &str,
        snapshot_engine: SnapshotEngine,
        audit_logger: AuditLogger,
        engram_context: EngramContext,
        theme_name: &str,
    ) -> Self {
        Self {
            snapshot_engine: Arc::new(Mutex::new(snapshot_engine)),
            undo_stack: Arc::new(Mutex::new(UndoStack::new())),
            audit_logger,
            engram_context,
            working_dir: working_dir.to_string(),
            current_theme_name: Arc::new(Mutex::new(theme_name.to_string())),
        }
    }

    /// Create a minimal context for non-interactive / test use.
    pub fn minimal(mode: Mode) -> Self {
        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        Self {
            snapshot_engine: Arc::new(Mutex::new(SnapshotEngine::new(
                std::path::Path::new(&working_dir),
            ))),
            undo_stack: Arc::new(Mutex::new(UndoStack::new())),
            audit_logger: AuditLogger::new(mode, AuditConfig::default()),
            engram_context: EngramContext::unavailable(),
            working_dir,
            current_theme_name: Arc::new(Mutex::new("admin".to_string())),
        }
    }
}

impl Default for ShellContext {
    fn default() -> Self {
        Self::minimal(Mode::Admin)
    }
}
