use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokentracker_core::domain::{
    provider_ids, ObservedRateLimitWindow, ProviderBalanceSnapshot,
};
use tokentracker_core::Storage;

use crate::credentials::CredentialManager;
use crate::error::{CollectorError, CollectorResult};

pub const DEFAULT_MOONSHOT_BALANCE_URL: &str = "https://api.moonshot.cn/v1/users/me/balance";
pub const DEFAULT_DEEPSEEK_BALANCE_URL: &str = "https://api.deepseek.com/user/balance";
pub const DEFAULT_QCODE_USAGE_URL: &str = "https://api.qcode.cc/api/v1/usage";
pub const DEFAULT_MINIMAX_REMAINS_URL: &str = "https://www.minimaxi.com/v1/token_plan/remains";
pub const BALANCE_COOLDOWN_MS: i64 = 60_000;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Summary returned after reconciling local metrics against an official vendor Usage/Cost API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OfficialApiReconcileResult {
    pub provider_id: String,
    pub endpoint: String,
    pub status_code: u16,
    pub buckets_received: usize,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub reported_cost_usd: Option<f64>,
    pub checked_at_ms: i64,
}

/// Configurable endpoints for official public Bearer balance / quota queries (overridable in tests).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BalanceApiEndpoints {
    pub moonshot_url: String,
    pub deepseek_url: String,
    pub qcode_url: String,
    pub minimax_url: String,
}

impl Default for BalanceApiEndpoints {
    fn default() -> Self {
        Self {
            moonshot_url: DEFAULT_MOONSHOT_BALANCE_URL.to_string(),
            deepseek_url: DEFAULT_DEEPSEEK_BALANCE_URL.to_string(),
            qcode_url: DEFAULT_QCODE_USAGE_URL.to_string(),
            minimax_url: DEFAULT_MINIMAX_REMAINS_URL.to_string(),
        }
    }
}

/// Outcome returned when refreshing a provider's official balance / quota snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BalanceRefreshOutcome {
    pub snapshot: ProviderBalanceSnapshot,
    pub rate_limits_updated: usize,
    pub endpoint: String,
    pub from_cache: bool,
    pub cooldown_remaining_secs: u64,
}

/// Minimal authenticated GET abstraction so official balance polling can be tested offline.
pub trait AuthHttpFetcher: Send + Sync {
    fn fetch_bearer_get<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> BoxFuture<'a, Result<(u16, String), String>>;
}

/// Production HTTP client honoring strict connect/request timeouts.
#[derive(Debug, Clone)]
pub struct ReqwestAuthFetcher {
    client: Client,
}

impl ReqwestAuthFetcher {
    pub fn new() -> CollectorResult<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .user_agent(concat!("TokenTracker/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self { client })
    }
}

impl AuthHttpFetcher for ReqwestAuthFetcher {
    fn fetch_bearer_get<'a>(
        &'a self,
        url: &'a str,
        bearer_token: &'a str,
    ) -> BoxFuture<'a, Result<(u16, String), String>> {
        Box::pin(async move {
            let resp = self
                .client
                .get(url)
                .bearer_auth(bearer_token)
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| format!("Network request failed: {e}"))?;
            let status = resp.status().as_u16();
            let text = resp
                .text()
                .await
                .map_err(|e| format!("Failed reading response body: {e}"))?;
            Ok((status, text))
        })
    }
}

fn endpoint_for_provider(
    provider_id: &str,
    endpoints: &BalanceApiEndpoints,
) -> CollectorResult<String> {
    match provider_id {
        provider_ids::KIMI_CODE => Ok(endpoints.moonshot_url.clone()),
        provider_ids::DEEPSEEK_HARNESS => Ok(endpoints.deepseek_url.clone()),
        provider_ids::QCODE => Ok(endpoints.qcode_url.clone()),
        provider_ids::MINIMAX_CODE => Ok(endpoints.minimax_url.clone()),
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public REST balance/quota API for simple Bearer keys; use local logs or subscription window rules"
        ))),
    }
}

/// Call documented official organization usage/cost APIs using a key stored in OS Credential Manager.
/// Strictly rejects any provider without a documented public billing/usage endpoint.
pub async fn reconcile_official_usage_api(
    provider_id: &str,
    credentials: &dyn CredentialManager,
    lookback_days: i64,
) -> CollectorResult<OfficialApiReconcileResult> {
    let api_key = credentials
        .get_api_key(provider_id)?
        .ok_or_else(|| CollectorError::Credential(format!("No API key configured for {provider_id}")))?;

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    let now = Utc::now();
    let start = now - chrono::Duration::days(lookback_days.clamp(1, 31));

    match provider_id {
        provider_ids::CLAUDE_CODE => {
            let url = format!(
                "https://api.anthropic.com/v1/organizations/usage_report/messages?starting_at={}&ending_at={}&bucket_width=1d",
                start.to_rfc3339(),
                now.to_rfc3339()
            );
            let resp = client
                .get(&url)
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .send()
                .await?;
            let status_code = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status_code >= 400 {
                return Err(CollectorError::Compliance(format!(
                    "Anthropic Admin API returned HTTP {status_code}: {}",
                    body.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("request rejected")
                )));
            }
            let buckets = body
                .get("data")
                .and_then(|d| d.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let (in_tok, out_tok) = sum_anthropic_buckets(&body);
            Ok(OfficialApiReconcileResult {
                provider_id: provider_id.to_string(),
                endpoint: "https://api.anthropic.com/v1/organizations/usage_report/messages"
                    .to_string(),
                status_code,
                buckets_received: buckets,
                total_input_tokens: in_tok,
                total_output_tokens: out_tok,
                reported_cost_usd: None,
                checked_at_ms: now.timestamp_millis(),
            })
        }
        provider_ids::CODEX => {
            let start_sec = start.timestamp();
            let end_sec = now.timestamp();
            let url = format!(
                "https://api.openai.com/v1/organization/usage/completions?start_time={start_sec}&end_time={end_sec}&bucket_width=1d"
            );
            let resp = client
                .get(&url)
                .bearer_auth(&api_key)
                .send()
                .await?;
            let status_code = resp.status().as_u16();
            let body: Value = resp.json().await.unwrap_or(Value::Null);
            if status_code >= 400 {
                return Err(CollectorError::Compliance(format!(
                    "OpenAI Usage API returned HTTP {status_code}: {}",
                    body.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .unwrap_or("request rejected")
                )));
            }
            let buckets = body
                .get("data")
                .and_then(|d| d.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            let (in_tok, out_tok) = sum_openai_buckets(&body);
            Ok(OfficialApiReconcileResult {
                provider_id: provider_id.to_string(),
                endpoint: "https://api.openai.com/v1/organization/usage/completions".to_string(),
                status_code,
                buckets_received: buckets,
                total_input_tokens: in_tok,
                total_output_tokens: out_tok,
                reported_cost_usd: None,
                checked_at_ms: now.timestamp_millis(),
            })
        }
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public organization usage API"
        ))),
    }
}

/// Query a provider's official public balance/quota endpoint using the Bearer API key in `credentials`,
/// honoring a strict 60-second cooldown and persisting results into SQLite `provider_balances`.
pub async fn refresh_provider_balance(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
) -> CollectorResult<BalanceRefreshOutcome> {
    let fetcher = ReqwestAuthFetcher::new()?;
    refresh_provider_balance_with_fetcher(
        storage,
        provider_id,
        credentials,
        &fetcher,
        &BalanceApiEndpoints::default(),
        Utc::now().timestamp_millis(),
    )
    .await
}

/// Deterministic testable entry point supporting injected HTTP fetcher and endpoints.
pub async fn refresh_provider_balance_with_fetcher(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
    fetcher: &dyn AuthHttpFetcher,
    endpoints: &BalanceApiEndpoints,
    now_ms: i64,
) -> CollectorResult<BalanceRefreshOutcome> {
    let endpoint = endpoint_for_provider(provider_id, endpoints)?;

    if let Some(cached) = storage.get_provider_balance(provider_id)? {
        let age_ms = now_ms.saturating_sub(cached.fetched_at_ms);
        if (0..BALANCE_COOLDOWN_MS).contains(&age_ms) {
            let remaining_ms = BALANCE_COOLDOWN_MS - age_ms;
            let remaining_secs = (remaining_ms.max(1) as u64).div_ceil(1000);
            return Ok(BalanceRefreshOutcome {
                snapshot: cached,
                rate_limits_updated: 0,
                endpoint,
                from_cache: true,
                cooldown_remaining_secs: remaining_secs,
            });
        }
    }

    let api_key = credentials
        .get_api_key(provider_id)?
        .ok_or_else(|| CollectorError::Credential(format!("未配置 {provider_id} 官方 API Key")))?;

    let (status, raw_body) = fetcher
        .fetch_bearer_get(&endpoint, &api_key)
        .await
        .map_err(CollectorError::Compliance)?;

    let body: Value = serde_json::from_str(&raw_body).unwrap_or(Value::Null);
    if status >= 400 {
        let detail = body
            .get("error")
            .and_then(|e| e.get("message").or(Some(e)))
            .or_else(|| body.get("message"))
            .or_else(|| body.get("msg"))
            .and_then(|v| v.as_str())
            .unwrap_or("request rejected");
        return Err(CollectorError::Compliance(format!(
            "{provider_id} 官方接口返回 HTTP {status}: {detail}"
        )));
    }

    let (snapshot, rate_limits) = match provider_id {
        provider_ids::KIMI_CODE => (
            parse_moonshot_balance(&body, provider_id, &endpoint, now_ms)?,
            Vec::new(),
        ),
        provider_ids::DEEPSEEK_HARNESS => (parse_deepseek_balance(&body, provider_id, now_ms)?, Vec::new()),
        provider_ids::QCODE => (parse_qcode_usage(&body, provider_id, now_ms)?, Vec::new()),
        provider_ids::MINIMAX_CODE => parse_minimax_remains(&body, provider_id, now_ms)?,
        _ => unreachable!("validated above"),
    };

    storage.upsert_provider_balance(&snapshot)?;
    let rate_limits_updated = rate_limits.len();
    if !rate_limits.is_empty() {
        storage.upsert_observed_rate_limits(&rate_limits)?;
    }

    Ok(BalanceRefreshOutcome {
        snapshot,
        rate_limits_updated,
        endpoint,
        from_cache: false,
        cooldown_remaining_secs: 0,
    })
}

fn json_amount_to_micro(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    let parsed = if let Some(n) = v.as_f64() {
        Some(n)
    } else if let Some(i) = v.as_i64() {
        Some(i as f64)
    } else if let Some(s) = v.as_str() {
        s.trim().parse::<f64>().ok()
    } else {
        None
    }?;
    if !parsed.is_finite() {
        return None;
    }
    Some((parsed * 1_000_000.0).round() as i64)
}

fn parse_moonshot_balance(
    body: &Value,
    provider_id: &str,
    endpoint: &str,
    now_ms: i64,
) -> CollectorResult<ProviderBalanceSnapshot> {
    let data = body.get("data").unwrap_or(body);
    let available = json_amount_to_micro(data.get("available_balance"));
    let voucher = json_amount_to_micro(data.get("voucher_balance"));
    let cash = json_amount_to_micro(data.get("cash_balance"));
    if available.is_none() && voucher.is_none() && cash.is_none() {
        return Err(CollectorError::Compliance(
            "Moonshot balance response missing available_balance / voucher_balance / cash_balance"
                .to_string(),
        ));
    }
    let currency = if endpoint.contains("moonshot.ai") {
        "USD"
    } else {
        "CNY"
    };
    Ok(ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: currency.to_string(),
        total_balance_micro: available,
        granted_balance_micro: voucher,
        topped_up_balance_micro: cash,
        daily_quota_used_micro: None,
        daily_quota_limit_micro: None,
        fetched_at_ms: now_ms,
    })
}

fn parse_deepseek_balance(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CollectorResult<ProviderBalanceSnapshot> {
    let infos = body
        .get("balance_infos")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            CollectorError::Compliance("DeepSeek balance response missing balance_infos".to_string())
        })?;
    if infos.is_empty() {
        return Err(CollectorError::Compliance(
            "DeepSeek balance_infos array is empty".to_string(),
        ));
    }
    let selected = infos
        .iter()
        .find(|b| b.get("currency").and_then(|c| c.as_str()) == Some("CNY"))
        .or_else(|| {
            infos
                .iter()
                .find(|b| b.get("currency").and_then(|c| c.as_str()) == Some("USD"))
        })
        .unwrap_or(&infos[0]);

    let currency = selected
        .get("currency")
        .and_then(|c| c.as_str())
        .unwrap_or("CNY")
        .to_string();

    Ok(ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency,
        total_balance_micro: json_amount_to_micro(selected.get("total_balance")),
        granted_balance_micro: json_amount_to_micro(selected.get("granted_balance")),
        topped_up_balance_micro: json_amount_to_micro(selected.get("topped_up_balance")),
        daily_quota_used_micro: None,
        daily_quota_limit_micro: None,
        fetched_at_ms: now_ms,
    })
}

fn parse_qcode_usage(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CollectorResult<ProviderBalanceSnapshot> {
    let data = body.get("data").unwrap_or(body);
    let currency = data
        .get("currency")
        .and_then(|v| v.as_str())
        .unwrap_or("USD")
        .to_string();

    let mut total_balance = json_amount_to_micro(
        data.get("remaining_balance")
            .or_else(|| data.get("available_balance"))
            .or_else(|| data.get("balance"))
            .or_else(|| data.get("remaining_quota")),
    );
    let mut daily_used = json_amount_to_micro(
        data.get("daily_used")
            .or_else(|| data.get("today_used"))
            .or_else(|| data.get("daily_usage")),
    );
    let mut daily_limit = json_amount_to_micro(
        data.get("daily_limit")
            .or_else(|| data.get("daily_quota")),
    );

    if let Some(keys) = data.get("keys").and_then(|v| v.as_array()) {
        let mut sum_rem = 0i64;
        let mut sum_used = 0i64;
        let mut sum_limit = 0i64;
        let mut has_rem = false;
        let mut has_used = false;
        let mut has_limit = false;
        for k in keys {
            if let Some(v) = json_amount_to_micro(
                k.get("remaining_quota")
                    .or_else(|| k.get("quota_remaining"))
                    .or_else(|| k.get("balance")),
            ) {
                sum_rem += v;
                has_rem = true;
            }
            if let Some(v) =
                json_amount_to_micro(k.get("daily_usage").or_else(|| k.get("daily_used")))
            {
                sum_used += v;
                has_used = true;
            }
            if let Some(v) =
                json_amount_to_micro(k.get("daily_limit").or_else(|| k.get("daily_quota")))
            {
                sum_limit += v;
                has_limit = true;
            }
        }
        if total_balance.is_none() && has_rem {
            total_balance = Some(sum_rem);
        }
        if daily_used.is_none() && has_used {
            daily_used = Some(sum_used);
        }
        if daily_limit.is_none() && has_limit {
            daily_limit = Some(sum_limit);
        }
    }

    if total_balance.is_none() && daily_used.is_none() && daily_limit.is_none() {
        return Err(CollectorError::Compliance(
            "QCode usage response did not contain balance or daily quota fields".to_string(),
        ));
    }

    Ok(ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency,
        total_balance_micro: total_balance,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: daily_used,
        daily_quota_limit_micro: daily_limit,
        fetched_at_ms: now_ms,
    })
}

fn parse_minimax_remains(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    if let Some(code) = body
        .pointer("/base_resp/status_code")
        .and_then(|v| v.as_i64())
    {
        if code != 0 {
            let msg = body
                .pointer("/base_resp/status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("MiniMax API error");
            return Err(CollectorError::Compliance(format!(
                "MiniMax API returned status_code {code}: {msg}"
            )));
        }
    }

    let data = body.get("data").unwrap_or(body);
    let total_balance = json_amount_to_micro(
        data.get("available_amount")
            .or_else(|| data.get("available_balance"))
            .or_else(|| data.get("balance")),
    );

    let mut windows = Vec::new();
    let mut sum_used_count = 0i64;
    let mut sum_total_count = 0i64;

    if let Some(remains) = data.get("model_remains").and_then(|v| v.as_array()) {
        for item in remains {
            let model_name = item
                .get("model_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("MiniMax Coding Plan");
            let total_count = item
                .get("current_interval_total_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0);
            if total_count == 0 {
                continue;
            }
            let used_count = if let Some(u) = item
                .get("current_interval_used_count")
                .and_then(|v| v.as_i64())
            {
                u.clamp(0, total_count)
            } else if let Some(rem) = item
                .get("current_interval_remains_count")
                .and_then(|v| v.as_i64())
            {
                total_count.saturating_sub(rem.max(0))
            } else {
                item.get("current_interval_usage_count")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0)
                    .clamp(0, total_count)
            };

            let used_percent = ((used_count as f64 / total_count as f64) * 100.0).clamp(0.0, 100.0);
            let resets_at_ms = item
                .get("end_time")
                .and_then(|v| v.as_i64())
                .map(|t| if t.abs() < 10_000_000_000 { t * 1000 } else { t })
                .or_else(|| {
                    item.get("remains_time")
                        .and_then(|v| v.as_i64())
                        .filter(|&r| r > 0)
                        .map(|r| now_ms + if r < 1_000_000 { r * 1000 } else { r })
                });

            sum_used_count += used_count;
            sum_total_count += total_count;

            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: format!("5h · {model_name}"),
                used_percent,
                window_minutes: Some(300),
                resets_at_ms,
                observed_at_ms: now_ms,
            });
        }
    }

    if windows.is_empty() && total_balance.is_none() {
        return Err(CollectorError::Compliance(
            "MiniMax remains response missing model_remains and balance fields".to_string(),
        ));
    }

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: data
            .get("currency")
            .and_then(|v| v.as_str())
            .unwrap_or("USD")
            .to_string(),
        total_balance_micro: total_balance,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: (sum_total_count > 0).then_some(sum_used_count),
        daily_quota_limit_micro: (sum_total_count > 0).then_some(sum_total_count),
        fetched_at_ms: now_ms,
    };

    Ok((snapshot, windows))
}

fn sum_anthropic_buckets(body: &Value) -> (i64, i64) {
    let mut input = 0i64;
    let mut output = 0i64;
    if let Some(buckets) = body.get("data").and_then(|d| d.as_array()) {
        for b in buckets {
            if let Some(results) = b.get("results").and_then(|r| r.as_array()) {
                for r in results {
                    input += r.get("uncached_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    input += r.get("cache_read_input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    output += r.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                }
            }
        }
    }
    (input, output)
}

fn sum_openai_buckets(body: &Value) -> (i64, i64) {
    let mut input = 0i64;
    let mut output = 0i64;
    if let Some(buckets) = body.get("data").and_then(|d| d.as_array()) {
        for b in buckets {
            if let Some(results) = b.get("results").and_then(|r| r.as_array()) {
                for r in results {
                    input += r.get("input_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                    output += r.get("output_tokens").and_then(|v| v.as_i64()).unwrap_or(0);
                }
            }
        }
    }
    (input, output)
}
