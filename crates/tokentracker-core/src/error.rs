use thiserror::Error;

/// Unified error type returned by `tokentracker-core`.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Decimal conversion error: {0}")]
    Decimal(#[from] rust_decimal::Error),

    #[error("Invalid configuration or input: {0}")]
    Validation(String),

    #[error("Resource not found: {0}")]
    NotFound(String),

    #[error("Time calculation overflow: {0}")]
    TimeOverflow(String),
}

pub type CoreResult<T> = Result<T, CoreError>;
