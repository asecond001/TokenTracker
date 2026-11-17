#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod analytics;
pub mod declarative;
pub mod domain;
pub mod error;
pub mod pricing;
pub mod storage;
pub mod subscription;
pub mod sync_crypto;
pub mod sync_mobile;

pub use error::{CoreError, CoreResult};
pub use storage::{IngestOutcome, Storage};
