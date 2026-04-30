//! Current execution mode stored as shrs State.

use crate::profile::Mode;

/// The current shell execution mode, stored in shrs States.
#[derive(Debug, Clone, Copy)]
pub struct ShellMode(pub Mode);

impl Default for ShellMode {
    fn default() -> Self {
        Self(Mode::Admin)
    }
}
