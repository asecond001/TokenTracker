use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};

use crate::domain::{
    is_valid_tps_sample, provider_ids, sanitize_session_title, AppPreferences, CollectionMode,
    CurrencyCode, CustomProtocolKind, CustomProviderRule, DeclarativeMappingSpec, DeviceRecord,
    IngestionCursor, ModelPricingEntry, NormalizedUsageEvent, ObservedRateLimitWindow, PaygAccount,
    PaygAccountStatus, ProviderBalanceSnapshot, ProviderConfig, QuotaUnit,
    RemoteSubscriptionSnapshot, RemoteSyncApplyResult, SessionTitleRecord, SubscriptionPlan,
    SubscriptionPlanTemplate, SubscriptionWindowRule, SyncConfigState, SyncEnvelopeV1,
    SyncHourlyDelta, SyncSubscriptionSnapshot, MAX_VALID_OUTPUT_TPS, MIN_VALID_TPS_DURATION_MS,
    MIN_VALID_TPS_OUTPUT_TOKENS,
};
use crate::error::{CoreError, CoreResult};
use crate::pricing::{
    apply_pricing_to_event, compute_catalog_hash, default_pricing_catalog, parse_exchange_rate,
    resolve_pricing,
};
use crate::storage::migrations::run_migrations;
use crate::subscription::{
    compute_subscription_catalog_hash, evaluate_subscription_plan, subscription_plan_templates,
    WindowUsageSample,
};

mod events;
mod providers;
mod pricing;
mod subscriptions;
mod preferences;
mod sync;
#[cfg(test)]
mod tests;

pub(crate) use self::events::*;
pub(crate) use self::pricing::*;
pub(crate) use self::subscriptions::*;
pub(crate) use self::preferences::*;
pub(crate) use self::sync::*;

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 86_400_000;

/// Result counts from an atomic ingestion batch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IngestOutcome {
    pub received: usize,
    pub inserted: usize,
    pub deduplicated: usize,
}

#[derive(Debug, Default)]
struct RollupDelta {
    request_count: i64,
    observable_step_count: i64,
    input_tokens: i64,
    output_tokens: i64,
    cache_read_tokens: i64,
    cache_write_tokens: i64,
    reasoning_tokens: i64,
    estimated_cost_micro_usd: i64,
    cache_saved_micro_usd: i64,
    total_duration_ms: i64,
    tps_sample_count: i64,
    tps_sum: f64,
    max_output_tps: f64,
}

impl RollupDelta {
    fn add_event(&mut self, event: &NormalizedUsageEvent) {
        self.request_count += 1;
        if event.observable_only {
            self.observable_step_count += 1;
        }
        self.input_tokens += event.input_tokens;
        self.output_tokens += event.output_tokens;
        self.cache_read_tokens += event.cache_read_tokens;
        self.cache_write_tokens += event.cache_write_tokens;
        self.reasoning_tokens += event.reasoning_tokens;
        self.estimated_cost_micro_usd += event.estimated_cost_micro_usd;
        self.cache_saved_micro_usd += event.cache_saved_micro_usd;
        if let Some(dur) = event.duration_ms {
            if dur > 0 {
                self.total_duration_ms += dur;
            }
        }
        if let Some(tps) = event.output_tps {
            if tps.is_finite() && tps > 0.0 {
                self.tps_sample_count += 1;
                self.tps_sum += tps;
                if tps > self.max_output_tps {
                    self.max_output_tps = tps;
                }
            }
        }
    }
}

/// Number of dedicated read-only connections opened for file-backed databases.
///
/// WAL mode lets these readers run concurrently with the single writer, so heavy
/// analytics queries never queue behind ingestion batches.
const READ_POOL_SIZE: usize = 3;

/// Thread-safe SQLite repository backing all core domain operations.
///
/// Writes (and any read that must observe uncommitted state inside the writer)
/// go through the single `conn`. Pure analytics reads go through `readers`, a
/// small pool of `SQLITE_OPEN_READ_ONLY` connections. In-memory databases cannot
/// share state across connections, so they keep `readers` empty and fall back to
/// the writer connection.
#[derive(Clone)]
pub struct Storage {
    conn: Arc<Mutex<Connection>>,
    readers: Arc<Vec<Mutex<Connection>>>,
    next_reader: Arc<AtomicUsize>,
    event_count: Arc<Mutex<EventCountCache>>,
}

/// Cached `usage_events` row count; `epoch` bumps on every committed insert batch.
#[derive(Debug, Default)]
struct EventCountCache {
    epoch: u64,
    count: Option<i64>,
}

impl Storage {
    /// Open (or create) a file-backed SQLite database with WAL mode and run migrations.
    pub fn open_file(path: &Path) -> CoreResult<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    CoreError::Validation(format!(
                        "Failed to create database directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }
        }
        let mut conn = Connection::open(path)?;
        configure_connection(&conn, true)?;
        run_migrations(&mut conn)?;

        // Readers are opened after migrations so the WAL/-shm files already exist.
        let mut readers = Vec::with_capacity(READ_POOL_SIZE);
        for _ in 0..READ_POOL_SIZE {
            let reader = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )?;
            reader.pragma_update(None, "busy_timeout", 5000i64)?;
            readers.push(Mutex::new(reader));
        }

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            readers: Arc::new(readers),
            next_reader: Arc::new(AtomicUsize::new(0)),
            event_count: Arc::new(Mutex::new(EventCountCache::default())),
        })
    }

    /// Open an isolated in-memory SQLite database for tests and benchmarks.
    pub fn open_in_memory() -> CoreResult<Self> {
        let mut conn = Connection::open_in_memory()?;
        configure_connection(&conn, false)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            readers: Arc::new(Vec::new()),
            next_reader: Arc::new(AtomicUsize::new(0)),
            event_count: Arc::new(Mutex::new(EventCountCache::default())),
        })
    }

    /// Run a pure read query on a read-only pooled connection.
    ///
    /// Falls back to the writer connection for in-memory databases. Any write
    /// attempted through this path fails with `SQLITE_READONLY` on file databases.
    pub(crate) fn with_read_conn<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&Connection) -> CoreResult<T>,
    {
        if self.readers.is_empty() {
            return self.with_conn(f);
        }
        let start = self.next_reader.fetch_add(1, Ordering::Relaxed);
        let len = self.readers.len();
        // Prefer an idle reader; otherwise wait on the round-robin slot.
        for i in 0..len {
            if let Ok(guard) = self.readers[(start + i) % len].try_lock() {
                return f(&guard);
            }
        }
        let guard = self.readers[start % len]
            .lock()
            .map_err(|_| CoreError::Validation("SQLite reader mutex poisoned".to_string()))?;
        f(&guard)
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&Connection) -> CoreResult<T>,
    {
        let guard = self
            .conn
            .lock()
            .map_err(|_| CoreError::Validation("SQLite mutex poisoned".to_string()))?;
        f(&guard)
    }

    pub(crate) fn with_conn_mut<F, T>(&self, f: F) -> CoreResult<T>
    where
        F: FnOnce(&mut Connection) -> CoreResult<T>,
    {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| CoreError::Validation("SQLite mutex poisoned".to_string()))?;
        f(&mut guard)
    }

    pub fn local_device(&self) -> CoreResult<DeviceRecord> {
        self.with_conn(query_local_device)
    }
}

pub(crate) fn configure_connection(conn: &Connection, enable_wal: bool) -> CoreResult<()> {
    if enable_wal {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
    }
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 5000i64)?;
    Ok(())
}

pub(crate) fn query_local_device(conn: &Connection) -> CoreResult<DeviceRecord> {
    conn.query_row(
        "SELECT device_id, device_name, platform, created_at_ms FROM devices ORDER BY created_at_ms ASC LIMIT 1",
        [],
        |row| {
            Ok(DeviceRecord {
                device_id: row.get(0)?,
                device_name: row.get(1)?,
                platform: row.get(2)?,
                created_at_ms: row.get(3)?,
            })
        },
    )
    .map_err(CoreError::from)
}
