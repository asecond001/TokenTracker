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
use crate::providers::parse_timestamp_ms;

pub const DEFAULT_MOONSHOT_BALANCE_URL: &str = "https://api.moonshot.cn/v1/users/me/balance";
pub const DEFAULT_MOONSHOT_GLOBAL_BALANCE_URL: &str =
    "https://api.moonshot.ai/v1/users/me/balance";
pub const DEFAULT_KIMI_CODING_USAGES_URL: &str = "https://api.kimi.com/coding/v1/usages";
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
    pub moonshot_global_url: String,
    pub kimi_coding_url: String,
    pub deepseek_url: String,
    pub qcode_url: String,
    pub minimax_url: String,
}

impl Default for BalanceApiEndpoints {
    fn default() -> Self {
        Self {
            moonshot_url: DEFAULT_MOONSHOT_BALANCE_URL.to_string(),
            moonshot_global_url: DEFAULT_MOONSHOT_GLOBAL_BALANCE_URL.to_string(),
            kimi_coding_url: DEFAULT_KIMI_CODING_USAGES_URL.to_string(),
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

fn candidate_endpoints_for_provider(
    provider_id: &str,
    api_key: &str,
    endpoints: &BalanceApiEndpoints,
) -> CollectorResult<Vec<String>> {
    match provider_id {
        provider_ids::KIMI_CODE => {
            let mut list = Vec::new();
            let push_unique = |out: &mut Vec<String>, url: &str| {
                if !url.trim().is_empty() && !out.iter().any(|u| u == url) {
                    out.push(url.to_string());
                }
            };
            if api_key.trim().starts_with("sk-kimi-") {
                push_unique(&mut list, &endpoints.kimi_coding_url);
            } else {
                push_unique(&mut list, &endpoints.moonshot_url);
                push_unique(&mut list, &endpoints.moonshot_global_url);
                push_unique(&mut list, &endpoints.kimi_coding_url);
            }
            Ok(list)
        }
        provider_ids::DEEPSEEK_HARNESS => Ok(vec![endpoints.deepseek_url.clone()]),
        provider_ids::QCODE => Ok(vec![endpoints.qcode_url.clone()]),
        provider_ids::MINIMAX_CODE => Ok(vec![endpoints.minimax_url.clone()]),
        other => Err(CollectorError::Compliance(format!(
            "Provider '{other}' does not expose a documented public REST balance/quota API for simple Bearer keys; use local logs or subscription window rules"
        ))),
    }
}

fn default_endpoint_label(provider_id: &str, endpoints: &BalanceApiEndpoints) -> CollectorResult<String> {
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
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        &fetcher,
        &BalanceApiEndpoints::default(),
        Utc::now().timestamp_millis(),
        false,
    )
    .await
}

/// Immediately force-refresh a provider's balance/quota bypassing the 60-second cooldown
/// (e.g. right after saving a new API key).
pub async fn refresh_provider_balance_force(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
) -> CollectorResult<BalanceRefreshOutcome> {
    let fetcher = ReqwestAuthFetcher::new()?;
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        &fetcher,
        &BalanceApiEndpoints::default(),
        Utc::now().timestamp_millis(),
        true,
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
    refresh_provider_balance_with_fetcher_inner(
        storage,
        provider_id,
        credentials,
        fetcher,
        endpoints,
        now_ms,
        false,
    )
    .await
}

pub async fn refresh_provider_balance_with_fetcher_inner(
    storage: &Storage,
    provider_id: &str,
    credentials: &dyn CredentialManager,
    fetcher: &dyn AuthHttpFetcher,
    endpoints: &BalanceApiEndpoints,
    now_ms: i64,
    bypass_cooldown: bool,
) -> CollectorResult<BalanceRefreshOutcome> {
    let default_ep = default_endpoint_label(provider_id, endpoints)?;

    if !bypass_cooldown {
        if let Some(cached) = storage.get_provider_balance(provider_id)? {
            let age_ms = now_ms.saturating_sub(cached.fetched_at_ms);
            if (0..BALANCE_COOLDOWN_MS).contains(&age_ms) {
                let remaining_ms = BALANCE_COOLDOWN_MS - age_ms;
                let remaining_secs = (remaining_ms.max(1) as u64).div_ceil(1000);
                return Ok(BalanceRefreshOutcome {
                    snapshot: cached,
                    rate_limits_updated: 0,
                    endpoint: default_ep,
                    from_cache: true,
                    cooldown_remaining_secs: remaining_secs,
                });
            }
        }
    }

    let api_key = credentials
        .get_api_key(provider_id)?
        .ok_or_else(|| CollectorError::Credential(format!("未配置 {provider_id} 官方 API Key")))?;

    let candidates = candidate_endpoints_for_provider(provider_id, &api_key, endpoints)?;
    let mut selected_endpoint = default_ep;
    let mut response_body: Option<Value> = None;
    let mut last_error: Option<CollectorError> = None;

    for (idx, candidate_ep) in candidates.iter().enumerate() {
        let fetched = fetcher.fetch_bearer_get(candidate_ep, &api_key).await;
        match fetched {
            Ok((status, raw_body)) => {
                let body: Value = serde_json::from_str(&raw_body).unwrap_or(Value::Null);
                if status < 400 {
                    selected_endpoint = candidate_ep.clone();
                    response_body = Some(body);
                    break;
                }
                let detail = body
                    .get("error")
                    .and_then(|e| e.get("message").or(Some(e)))
                    .or_else(|| body.get("message"))
                    .or_else(|| body.get("msg"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("request rejected");
                let err_msg = format!("{provider_id} 官方接口 ({candidate_ep}) 返回 HTTP {status}: {detail}");
                last_error = Some(CollectorError::Compliance(err_msg));
                let retryable_fallback = idx + 1 < candidates.len()
                    && matches!(status, 401 | 403 | 404);
                if !retryable_fallback {
                    break;
                }
            }
            Err(net_err) => {
                last_error = Some(CollectorError::Compliance(net_err));
                if idx + 1 >= candidates.len() {
                    break;
                }
            }
        }
    }

    let body = match response_body {
        Some(b) => b,
        None => {
            return Err(last_error.unwrap_or_else(|| {
                CollectorError::Compliance(format!("{provider_id} 官方接口请求失败"))
            }))
        }
    };

    let (snapshot, rate_limits) = match provider_id {
        provider_ids::KIMI_CODE => {
            if selected_endpoint == endpoints.kimi_coding_url
                || body.get("usage").is_some()
                || body.get("limits").is_some()
            {
                parse_kimi_coding_usages(&body, provider_id, now_ms)?
            } else {
                (
                    parse_moonshot_balance(&body, provider_id, &selected_endpoint, now_ms)?,
                    Vec::new(),
                )
            }
        }
        provider_ids::DEEPSEEK_HARNESS => {
            (parse_deepseek_balance(&body, provider_id, now_ms)?, Vec::new())
        }
        provider_ids::QCODE => parse_qcode_usage(&body, provider_id, now_ms)?,
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
        endpoint: selected_endpoint,
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

fn json_number_as_f64(val: Option<&Value>) -> Option<f64> {
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
    parsed.is_finite().then_some(parsed)
}

fn compute_usage_percent_from_obj(obj: &Value) -> Option<f64> {
    if let Some(pct) = json_number_as_f64(
        obj.get("used_percent")
            .or_else(|| obj.get("usedPercent"))
            .or_else(|| obj.get("percent")),
    ) {
        return Some(pct.clamp(0.0, 100.0));
    }
    let limit = json_number_as_f64(obj.get("limit"))?;
    if limit <= 0.0 {
        return None;
    }
    if let Some(used) = json_number_as_f64(obj.get("used")) {
        return Some(((used / limit) * 100.0).clamp(0.0, 100.0));
    }
    if let Some(rem) = json_number_as_f64(obj.get("remaining")) {
        return Some((((limit - rem).max(0.0) / limit) * 100.0).clamp(0.0, 100.0));
    }
    None
}

fn parse_kimi_coding_usages(
    body: &Value,
    provider_id: &str,
    now_ms: i64,
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
    let data = body.get("data").unwrap_or(body);
    let plan_hint = body
        .pointer("/user/membership/level")
        .or_else(|| data.pointer("/user/membership/level"))
        .or_else(|| data.pointer("/membership/level"))
        .or_else(|| data.get("plan"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut windows = Vec::new();
    let mut primary_used_count: Option<i64> = None;
    let mut primary_limit_count: Option<i64> = None;
    let mut weekly_used_count: Option<i64> = None;
    let mut weekly_limit_count: Option<i64> = None;

    if let Some(usage_obj) = data.get("usage").or_else(|| body.get("usage")) {
        let limit = json_number_as_f64(usage_obj.get("limit"));
        let used = json_number_as_f64(usage_obj.get("used")).or_else(|| {
            match (limit, json_number_as_f64(usage_obj.get("remaining"))) {
                (Some(l), Some(r)) => Some((l - r).max(0.0)),
                _ => None,
            }
        });
        if let (Some(u), Some(l)) = (used, limit) {
            weekly_used_count = Some(u.round() as i64);
            weekly_limit_count = Some(l.round() as i64);
        }
        if let Some(used_percent) = compute_usage_percent_from_obj(usage_obj) {
            let resets_at_ms = parse_timestamp_ms(
                usage_obj
                    .get("resetTime")
                    .or_else(|| usage_obj.get("reset_time"))
                    .or_else(|| usage_obj.get("resetAt"))
                    .or_else(|| usage_obj.get("resets_at")),
            );
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: "7d".to_string(),
                used_percent,
                window_minutes: Some(10_080),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: plan_hint.clone(),
            });
        }
    }

    if let Some(limits) = data
        .get("limits")
        .or_else(|| body.get("limits"))
        .and_then(|v| v.as_array())
    {
        for (idx, item) in limits.iter().enumerate() {
            let detail = item.get("detail").unwrap_or(item);
            let Some(used_percent) = compute_usage_percent_from_obj(detail) else {
                continue;
            };
            let raw_dur = json_number_as_f64(item.pointer("/window/duration"))
                .map(|d| d.round() as i64)
                .unwrap_or(if idx == 0 { 300 } else { 10_080 });
            let unit_str = item
                .pointer("/window/timeUnit")
                .or_else(|| item.pointer("/window/time_unit"))
                .and_then(|v| v.as_str())
                .unwrap_or("TIME_UNIT_MINUTE")
                .to_ascii_uppercase();
            let window_minutes = if unit_str.contains("HOUR") {
                raw_dur.max(1) * 60
            } else if unit_str.contains("DAY") {
                raw_dur.max(1) * 1_440
            } else if unit_str.contains("SECOND") {
                (raw_dur / 60).max(1)
            } else {
                raw_dur.max(1)
            };

            let window_name = match window_minutes {
                300 => "5h".to_string(),
                1_440 => "24h".to_string(),
                10_080 => "7d".to_string(),
                m if m % 60 == 0 => format!("{}h", m / 60),
                m => format!("{m}m"),
            };

            let limit = json_number_as_f64(detail.get("limit"));
            let used = json_number_as_f64(detail.get("used")).or_else(|| {
                match (limit, json_number_as_f64(detail.get("remaining"))) {
                    (Some(l), Some(r)) => Some((l - r).max(0.0)),
                    _ => None,
                }
            });
            if primary_limit_count.is_none() {
                if let (Some(u), Some(l)) = (used, limit) {
                    primary_used_count = Some(u.round() as i64);
                    primary_limit_count = Some(l.round() as i64);
                }
            }

            let resets_at_ms = parse_timestamp_ms(
                detail
                    .get("resetTime")
                    .or_else(|| detail.get("reset_time"))
                    .or_else(|| detail.get("resetAt"))
                    .or_else(|| detail.get("resets_at")),
            );
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name,
                used_percent,
                window_minutes: Some(window_minutes),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: plan_hint.clone(),
            });
        }
    }

    if windows.is_empty() && primary_limit_count.is_none() && weekly_limit_count.is_none() {
        return Err(CollectorError::Compliance(
            "Kimi Coding usage response missing usage / limits fields".to_string(),
        ));
    }

    let snapshot = ProviderBalanceSnapshot {
        provider_id: provider_id.to_string(),
        currency: "QUOTA".to_string(),
        total_balance_micro: None,
        granted_balance_micro: None,
        topped_up_balance_micro: None,
        daily_quota_used_micro: primary_used_count.or(weekly_used_count),
        daily_quota_limit_micro: primary_limit_count.or(weekly_limit_count),
        fetched_at_ms: now_ms,
    };

    Ok((snapshot, windows))
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
) -> CollectorResult<(ProviderBalanceSnapshot, Vec<ObservedRateLimitWindow>)> {
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

    let qcode_plan_hint = data
        .get("plan_name")
        .or_else(|| data.get("tier"))
        .or_else(|| data.get("package_name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| "qcode_daily".to_string());

    let mut windows = Vec::new();
    if let (Some(used), Some(limit)) = (daily_used, daily_limit) {
        if limit > 0 {
            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: "24h".to_string(),
                used_percent: ((used as f64 / limit as f64) * 100.0).clamp(0.0, 100.0),
                window_minutes: Some(1_440),
                resets_at_ms: None,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some(qcode_plan_hint),
            });
        }
    }

    Ok((
        ProviderBalanceSnapshot {
            provider_id: provider_id.to_string(),
            currency,
            total_balance_micro: total_balance,
            granted_balance_micro: None,
            topped_up_balance_micro: None,
            daily_quota_used_micro: daily_used,
            daily_quota_limit_micro: daily_limit,
            fetched_at_ms: now_ms,
        },
        windows,
    ))
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

            let minimax_plan_hint = item
                .get("plan_name")
                .or_else(|| item.get("combo_name"))
                .or_else(|| data.get("plan_name"))
                .or_else(|| data.get("combo_name"))
                .or_else(|| data.get("package_name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| "minimax_coding_plan".to_string());

            windows.push(ObservedRateLimitWindow {
                provider_id: provider_id.to_string(),
                window_name: format!("5h · {model_name}"),
                used_percent,
                window_minutes: Some(300),
                resets_at_ms,
                observed_at_ms: now_ms,
                source: Some("official_api".to_string()),
                plan_hint: Some(minimax_plan_hint),
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
