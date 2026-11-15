#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod credentials;
pub mod error;
pub mod otlp;
pub mod providers;
pub mod watcher;

pub use credentials::{CredentialManager, MemoryCredentialStore, OsKeyringStore};
pub use error::{CollectorError, CollectorResult};
pub use otlp::{start_loopback_otlp_server, validate_loopback_addr, OtlpServerHandle};
pub use watcher::{CollectorCoordinator, ScanReport};
