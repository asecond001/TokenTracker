use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokentracker_core::domain::provider_ids;

use crate::credentials::CredentialManager;
use crate::error::{CollectorError, CollectorResult};

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
