//! Common utility types and errors for the Terminal Workspace.

use thiserror::Error;

/// Central Error definition across domain layers.
#[derive(Debug, Error)]
pub enum WorkspaceError {
    /// Database or storage related issues.
    #[error("Storage failure: {0}")]
    Storage(String),

    /// External integration connection issues.
    #[error("Integration error: {0}")]
    Integration(String),

    /// Plugin runtime faults.
    #[error("Plugin fault: {0}")]
    Plugin(String),

    /// Configuration parsing issues.
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Security or capability violation.
    #[error("Permission denied: {0}")]
    Security(String),

    /// Fallback error.
    #[error("Internal error: {0}")]
    Internal(String),
}

/// Generic Result type wrapper using the central WorkspaceError.
pub type Result<T> = std::result::Result<T, WorkspaceError>;
