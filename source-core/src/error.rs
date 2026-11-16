use thiserror::Error;

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CoreError {
    #[error("network policy blocked destination: {0}")]
    NetworkBlocked(String),

    #[error("approval required for {0}")]
    ApprovalRequired(String),

    #[error("approval denied for {0}")]
    ApprovalDenied(String),

    #[error("invalid mode transition from {from} to {to}")]
    InvalidModeTransition { from: String, to: String },

    #[error("provider {0} is disabled or unavailable")]
    ProviderUnavailable(String),

    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("checkpoint not found: {0}")]
    CheckpointNotFound(String),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    #[error("{0}")]
    Other(String),
}

impl CoreError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}