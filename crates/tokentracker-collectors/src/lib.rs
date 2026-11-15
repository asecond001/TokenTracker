#![cfg_attr(not(test), deny(clippy::unwrap_used))]

pub mod catalog_sync;
pub mod credentials;
pub mod error;
pub mod otlp;
pub mod providers;
pub mod sync_client;
pub mod watcher;

pub use catalog_sync::{
    sync_catalog_and_rate, sync_catalog_and_rate_with_endpoints, sync_catalog_and_rate_with_fetcher,
    CatalogSyncEndpoints, ReqwestTextFetcher, TextHttpFetcher,
};
pub use credentials::{CredentialManager, MemoryCredentialStore, OsKeyringStore};
pub use error::{CollectorError, CollectorResult};
pub use otlp::{start_loopback_otlp_server, validate_loopback_addr, OtlpServerHandle};
pub use providers::official_api::{
    refresh_provider_balance, refresh_provider_balance_with_fetcher, AuthHttpFetcher,
    BalanceApiEndpoints, BalanceRefreshOutcome, ReqwestAuthFetcher, BALANCE_COOLDOWN_MS,
};
pub use sync_client::{
    clear_sync_credentials, has_sync_credentials, read_sync_status_view, run_sync_cycle_once,
    run_sync_cycle_with_transport, set_sync_credentials, ReqwestSyncTransport, SseByteStream,
    SyncBoxFuture, SyncClientHandle, SyncHttpTransport, SyncRunReport, SyncStatusView,
    SYNC_BEARER_TOKEN_KEY, SYNC_E2EE_PASSPHRASE_KEY,
};
pub use watcher::{CollectorCoordinator, ScanReport};
