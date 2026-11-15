use thiserror::Error;
use tokentracker_core::CoreError;

#[derive(Debug, Error)]
pub enum CollectorError {
    #[error("Core storage/domain error: {0}")]
    Core(#[from] CoreError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Credential store error: {0}")]
    Credential(String),

    #[error("File watcher error: {0}")]
    Watcher(#[from] notify::Error),

    #[error("HTTP client error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Compliance or configuration error: {0}")]
    Compliance(String),
}

pub type CollectorResult<T> = Result<T, CollectorError>;
