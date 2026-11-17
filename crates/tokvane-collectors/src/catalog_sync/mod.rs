use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::Utc;
use tokvane_core::domain::{CatalogSyncResult, SubscriptionPlanTemplate};
use tokvane_core::pricing::{
    format_exchange_rate, parse_litellm_catalog, parse_models_dev_catalog,
    parse_remote_exchange_rate_json,
};
use tokvane_core::subscription::parse_subscription_catalog_json;
use tokvane_core::{CoreResult, Storage};

pub const DEFAULT_FRANKFURTER_URL: &str = "https://api.frankfurter.dev/v1/latest?base=USD&symbols=CNY";
pub const DEFAULT_OPEN_ER_URL: &str = "https://open.er-api.com/v6/latest/USD";
pub const DEFAULT_MODELS_DEV_URL: &str = "https://models.dev/api.json";
pub const DEFAULT_LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
pub const DEFAULT_SUBSCRIPTION_CATALOG_PAGES_URL: &str =
    "https://asecond001.github.io/ai-subscription-catalog/v1/plans.json";
pub const DEFAULT_SUBSCRIPTION_CATALOG_JSDELIVR_URL: &str =
    "https://cdn.jsdelivr.net/gh/asecond001/ai-subscription-catalog@main/catalog/subscription-plans.json";
pub const DEFAULT_SUBSCRIPTION_CATALOG_RAW_URL: &str =
    "https://raw.githubusercontent.com/asecond001/ai-subscription-catalog/main/catalog/subscription-plans.json";

pub const RATE_SYNC_TTL_MS: i64 = 12 * 3_600_000;
pub const PRICING_SYNC_TTL_MS: i64 = 24 * 3_600_000;
pub const SUBSCRIPTION_SYNC_TTL_MS: i64 = 24 * 3_600_000;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const REQUEST_TIMEOUT_SECS: u64 = 12;
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Configurable public unauthenticated HTTPS endpoints for USD->CNY rates,
/// model pricing catalogs, and `AI Subscription Catalog` templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSyncEndpoints {
    pub frankfurter_url: String,
    pub open_er_url: String,
    pub models_dev_url: String,
    pub litellm_url: String,
    pub subscription_catalog_urls: Vec<String>,
}

impl Default for CatalogSyncEndpoints {
    fn default() -> Self {
        Self {
            frankfurter_url: DEFAULT_FRANKFURTER_URL.to_string(),
            open_er_url: DEFAULT_OPEN_ER_URL.to_string(),
            models_dev_url: DEFAULT_MODELS_DEV_URL.to_string(),
            litellm_url: DEFAULT_LITELLM_URL.to_string(),
            subscription_catalog_urls: vec![
                DEFAULT_SUBSCRIPTION_CATALOG_PAGES_URL.to_string(),
                DEFAULT_SUBSCRIPTION_CATALOG_JSDELIVR_URL.to_string(),
                DEFAULT_SUBSCRIPTION_CATALOG_RAW_URL.to_string(),
            ],
        }
    }
}

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Minimal async HTTP text fetcher abstraction enabling deterministic mock testing without OS sockets.
pub trait TextHttpFetcher: Send + Sync {
    fn fetch_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, String>>;
}

/// Production reqwest-backed fetcher honoring 5s connect / 12s request timeouts and an 8 MiB cap.
#[derive(Debug, Clone)]
pub struct ReqwestTextFetcher {
    client: reqwest::Client,
}

impl ReqwestTextFetcher {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent(concat!("TokenVane/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;
        Ok(Self { client })
    }
}

impl TextHttpFetcher for ReqwestTextFetcher {
    fn fetch_text<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move { fetch_text_limited(&self.client, url).await })
    }
}

/// Synchronize public USD->CNY exchange rates and model pricing using default official endpoints.
pub async fn sync_catalog_and_rate(
    storage: &Storage,
    force: bool,
) -> CoreResult<CatalogSyncResult> {
    sync_catalog_and_rate_with_endpoints(storage, force, &CatalogSyncEndpoints::default()).await
}

/// Synchronize public USD->CNY exchange rates and model pricing using custom endpoints and live HTTPS.
/// Degrades gracefully to local cached pricing and exchange rate whenever offline or upstream fails.
pub async fn sync_catalog_and_rate_with_endpoints(
    storage: &Storage,
    force: bool,
    endpoints: &CatalogSyncEndpoints,
) -> CoreResult<CatalogSyncResult> {
    let fetcher = ReqwestTextFetcher::new()
        .map_err(tokvane_core::CoreError::Validation)?;
    sync_catalog_and_rate_with_fetcher(storage, force, endpoints, &fetcher).await
}

/// Synchronize public USD->CNY exchange rates and model pricing with an injectable HTTP text fetcher.
pub async fn sync_catalog_and_rate_with_fetcher(
    storage: &Storage,
    force: bool,
    endpoints: &CatalogSyncEndpoints,
    fetcher: &dyn TextHttpFetcher,
) -> CoreResult<CatalogSyncResult> {
    let prefs = storage.get_preferences()?;
    let now_ms = Utc::now().timestamp_millis();

    let should_sync_rate = prefs.exchange_rate_auto
        && (force
            || prefs
                .last_rate_sync_at_ms
                .is_none_or(|ts| now_ms.saturating_sub(ts) >= RATE_SYNC_TTL_MS));
    let should_sync_pricing = force
        || (prefs.auto_sync_pricing
            && prefs
                .last_pricing_sync_at_ms
                .is_none_or(|ts| now_ms.saturating_sub(ts) >= PRICING_SYNC_TTL_MS));

    let should_sync_subscriptions = !endpoints.subscription_catalog_urls.is_empty()
        && (force
            || (prefs.auto_sync_pricing
                && prefs
                    .last_subscription_sync_at_ms
                    .is_none_or(|ts| now_ms.saturating_sub(ts) >= SUBSCRIPTION_SYNC_TTL_MS)));

    let mut rate_updated = false;
    let mut pricing_updated = false;
    let mut synced_models_count = 0usize;
    let mut recomputed_events_count = 0usize;
    let mut rate_source: Option<String> = None;
    let mut pricing_source: Option<String> = None;
    let mut subscriptions_updated = false;
    let mut synced_plans_count = 0usize;
    let mut subscription_source: Option<String> = None;
    let mut warnings: Vec<String> = Vec::new();

    if should_sync_rate {
        match fetch_exchange_rate_with_fallback(fetcher, endpoints).await {
            Ok((formatted_rate, source)) => {
                rate_updated = storage.apply_synced_exchange_rate(&formatted_rate, now_ms)?;
                rate_source = Some(source);
            }
            Err(err) => {
                tracing::warn!("Exchange rate auto sync failed (offline fallback): {err}");
                warnings.push(format!("汇率同步失败，已沿用本地缓存 ({err})"));
            }
        }
    }

    if should_sync_pricing {
        let effective_date = Utc::now().format("%Y-%m-%d").to_string();
        match fetch_pricing_catalog_with_fallback(fetcher, endpoints, &effective_date).await {
            Ok((entries, source)) => {
                let (changed, count, recomputed) =
                    storage.apply_synced_pricing_catalog(&entries, now_ms)?;
                pricing_updated = changed;
                synced_models_count = count;
                recomputed_events_count = recomputed;
                pricing_source = Some(source);
            }
            Err(err) => {
                tracing::warn!("Model pricing catalog sync failed (offline fallback): {err}");
                warnings.push(format!("模型价格同步失败，已沿用本地目录 ({err})"));
            }
        }
    }

    if should_sync_subscriptions {
        match fetch_subscription_catalog_with_fallback(fetcher, &endpoints.subscription_catalog_urls)
            .await
        {
            Ok((templates, source)) => {
                let (changed, count) =
                    storage.apply_synced_subscription_catalog(&templates, now_ms)?;
                subscriptions_updated = changed;
                synced_plans_count = count;
                subscription_source = Some(source);
            }
            Err(err) => {
                tracing::warn!("AI Subscription Catalog sync failed (offline fallback): {err}");
                warnings.push(format!("订阅套餐目录同步失败，已沿用本地目录 ({err})"));
            }
        }
    }

    if synced_models_count == 0 {
        synced_models_count = storage.list_model_pricing()?.len();
    }
    if synced_plans_count == 0 {
        synced_plans_count = storage.query_subscription_templates(now_ms)?.len();
    }

    let latest_prefs = storage.get_preferences()?;
    let warning = if warnings.is_empty() {
        None
    } else {
        Some(warnings.join("；"))
    };

    Ok(CatalogSyncResult {
        rate_updated,
        usd_to_cny_rate: latest_prefs.usd_to_cny_rate,
        pricing_updated,
        synced_models_count,
        recomputed_events_count,
        synced_at_ms: now_ms,
        rate_source,
        pricing_source,
        subscriptions_updated,
        synced_plans_count,
        subscription_source,
        warning,
    })
}

fn source_label_for_catalog_url(url: &str) -> String {
    if url.contains("github.io") {
        "github-pages".to_string()
    } else if url.contains("jsdelivr.net") {
        "jsdelivr".to_string()
    } else if url.contains("raw.githubusercontent.com") {
        "github-raw".to_string()
    } else {
        url.split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .filter(|s| !s.is_empty())
            .unwrap_or("custom")
            .to_string()
    }
}

async fn fetch_subscription_catalog_with_fallback(
    fetcher: &dyn TextHttpFetcher,
    urls: &[String],
) -> Result<(Vec<SubscriptionPlanTemplate>, String), String> {
    let mut errors: Vec<String> = Vec::new();
    for url in urls {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            continue;
        }
        let label = source_label_for_catalog_url(trimmed);
        match fetcher.fetch_text(trimmed).await {
            Ok(body) => match parse_subscription_catalog_json(&body) {
                Ok(templates) => return Ok((templates, label)),
                Err(e) => errors.push(format!("{label} parse error: {e}")),
            },
            Err(e) => errors.push(format!("{label} fetch error: {e}")),
        }
    }
    if errors.is_empty() {
        Err("no subscription catalog URLs configured".to_string())
    } else {
        Err(errors.join("; "))
    }
}

async fn fetch_exchange_rate_with_fallback(
    fetcher: &dyn TextHttpFetcher,
    endpoints: &CatalogSyncEndpoints,
) -> Result<(String, String), String> {
    let primary_err = match fetcher.fetch_text(&endpoints.frankfurter_url).await {
        Ok(body) => match parse_remote_exchange_rate_json(&body) {
            Ok(dec) => return Ok((format_exchange_rate(dec), "frankfurter.dev".to_string())),
            Err(e) => format!("frankfurter parse error: {e}"),
        },
        Err(e) => format!("frankfurter fetch error: {e}"),
    };

    match fetcher.fetch_text(&endpoints.open_er_url).await {
        Ok(body) => match parse_remote_exchange_rate_json(&body) {
            Ok(dec) => Ok((format_exchange_rate(dec), "open.er-api.com".to_string())),
            Err(e) => Err(format!("{primary_err}; open-er parse error: {e}")),
        },
        Err(e) => Err(format!("{primary_err}; open-er fetch error: {e}")),
    }
}

async fn fetch_pricing_catalog_with_fallback(
    fetcher: &dyn TextHttpFetcher,
    endpoints: &CatalogSyncEndpoints,
    effective_date: &str,
) -> Result<(Vec<tokvane_core::domain::ModelPricingEntry>, String), String> {
    let primary_err = match fetcher.fetch_text(&endpoints.models_dev_url).await {
        Ok(body) => match parse_models_dev_catalog(&body, effective_date) {
            Ok(entries) => return Ok((entries, "models.dev".to_string())),
            Err(e) => format!("models.dev parse error: {e}"),
        },
        Err(e) => format!("models.dev fetch error: {e}"),
    };

    match fetcher.fetch_text(&endpoints.litellm_url).await {
        Ok(body) => match parse_litellm_catalog(&body, effective_date) {
            Ok(entries) => Ok((entries, "litellm".to_string())),
            Err(e) => Err(format!("{primary_err}; litellm parse error: {e}")),
        },
        Err(e) => Err(format!("{primary_err}; litellm fetch error: {e}")),
    }
}

async fn fetch_text_limited(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let mut resp = client.get(url).send().await.map_err(|e| {
        let mut msg = format!("request failed: {e}");
        let mut src = std::error::Error::source(&e);
        while let Some(inner) = src {
            msg.push_str(&format!(": {inner}"));
            src = inner.source();
        }
        msg
    })?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status().as_u16()));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err(format!("response content-length {len} exceeds 8 MiB limit"));
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| format!("stream error: {e}"))?
    {
        if buf.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err("response stream exceeded 8 MiB limit".to_string());
        }
        buf.extend_from_slice(&chunk);
    }

    String::from_utf8(buf).map_err(|e| format!("invalid UTF-8 response: {e}"))
}
