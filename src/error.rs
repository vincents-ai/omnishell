//! OmniShell error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum OmniShellError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("ACL violation: {0}")]
    AclViolation(String),

    #[error("Snapshot error: {0}")]
    Snapshot(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),

    #[error("Profile error: {0}")]
    Profile(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, OmniShellError>;
