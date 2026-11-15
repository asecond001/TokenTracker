use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokentracker_core::domain::{compute_event_id, provider_ids, NormalizedUsageEvent};
use tokentracker_core::Storage;

use crate::error::{CollectorError, CollectorResult};

#[derive(Clone)]
struct OtlpState {
    storage: Storage,
}

/// Handle controlling the optional loopback-only OTLP/HTTP server.
pub struct OtlpServerHandle {
    pub port: u16,
    shutdown_tx: watch::Sender<bool>,
    join_handle: JoinHandle<()>,
}

impl OtlpServerHandle {
    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub async fn stop(self) {
        self.signal_shutdown();
        let _ = self.join_handle.await;
    }
}

/// Validate that any OTLP bind target strictly resolves to loopback (`127.0.0.1` / `::1`).
pub fn validate_loopback_addr(ip: IpAddr, port: u16) -> CollectorResult<SocketAddr> {
    if !ip.is_loopback() {
        return Err(CollectorError::Compliance(format!(
            "OTLP listener is strictly restricted to loopback addresses (refused {ip})"
        )));
    }
    Ok(SocketAddr::new(ip, port))
}

/// Start the optional local OTLP/HTTP receiver bound strictly to `127.0.0.1:<port>`.
pub async fn start_loopback_otlp_server(
    storage: Storage,
    port: u16,
) -> CollectorResult<OtlpServerHandle> {
    let addr = validate_loopback_addr(IpAddr::V4(Ipv4Addr::LOCALHOST), port)?;
    let listener = TcpListener::bind(addr).await?;
    let actual_port = listener.local_addr()?.port();

    let (shutdown_tx, mut shutdown_rx) = watch::channel(false);
    let app = build_otlp_router(storage);

    let join_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                while !*shutdown_rx.borrow() {
                    if shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
    });

    Ok(OtlpServerHandle {
        port: actual_port,
        shutdown_tx,
        join_handle,
    })
}

pub fn build_otlp_router(storage: Storage) -> Router {
    let state = OtlpState { storage };
    Router::new()
        .route("/healthz", get(handle_healthz))
        .route("/v1/metrics", post(handle_otlp_metrics))
        .route("/v1/logs", post(handle_otlp_logs))
        .with_state(state)
}

async fn handle_healthz() -> (StatusCode, Json<Value>) {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "loopback_only": true,
            "routes": ["/v1/metrics", "/v1/logs"]
        })),
    )
}

/// Parse and ingest an OTLP/JSON `resourceMetrics` payload directly into storage.
pub fn ingest_otlp_metrics_payload(
    storage: &Storage,
    payload: &Value,
) -> CollectorResult<tokentracker_core::storage::IngestOutcome> {
    let device_id = storage.local_device()?.device_id;
    let events = parse_otlp_metrics_json(payload, &device_id);
    if events.is_empty() {
        return Ok(tokentracker_core::storage::IngestOutcome::default());
    }
    Ok(storage.ingest_events_with_cursor(&events, None, &[])?)
}

/// Parse and ingest an OTLP/JSON `resourceLogs` payload directly into storage.
pub fn ingest_otlp_logs_payload(
    storage: &Storage,
    payload: &Value,
) -> CollectorResult<tokentracker_core::storage::IngestOutcome> {
    let device_id = storage.local_device()?.device_id;
    let events = parse_otlp_logs_json(payload, &device_id);
    if events.is_empty() {
        return Ok(tokentracker_core::storage::IngestOutcome::default());
    }
    Ok(storage.ingest_events_with_cursor(&events, None, &[])?)
}

async fn handle_otlp_metrics(
    State(state): State<OtlpState>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match ingest_otlp_metrics_payload(&state.storage, &payload) {
        Ok(out) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "accepted": out.received,
                "inserted": out.inserted,
                "deduplicated": out.deduplicated
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn handle_otlp_logs(
    State(state): State<OtlpState>,
    Json(payload): Json<Value>,
) -> (StatusCode, Json<Value>) {
    match ingest_otlp_logs_payload(&state.storage, &payload) {
        Ok(out) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "accepted": out.received,
                "inserted": out.inserted,
                "deduplicated": out.deduplicated
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// Parse OTLP/JSON `resourceMetrics` (e.g. Claude Code `claude_code.token.usage` or `gen_ai.client.token.usage`).
pub fn parse_otlp_metrics_json(payload: &Value, device_id: &str) -> Vec<NormalizedUsageEvent> {
    let mut events = Vec::new();
    let Some(res_metrics) = payload.get("resourceMetrics").and_then(|v| v.as_array()) else {
        return events;
    };

    for rm in res_metrics {
        let res_attrs = extract_attrs(rm.get("resource").and_then(|r| r.get("attributes")));
        let default_provider = infer_provider_from_attrs(&res_attrs);

        let Some(scope_metrics) = rm.get("scopeMetrics").and_then(|v| v.as_array()) else {
            continue;
        };
        for sm in scope_metrics {
            let Some(metrics) = sm.get("metrics").and_then(|v| v.as_array()) else {
                continue;
            };
            for metric in metrics {
                let name = metric.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if !name.contains("token") && !name.contains("usage") {
                    continue;
                }
                let data_points = metric
                    .get("sum")
                    .or_else(|| metric.get("gauge"))
                    .and_then(|s| s.get("dataPoints"))
                    .and_then(|d| d.as_array());
                let Some(dps) = data_points else {
                    continue;
                };

                // Group datapoints sharing (session_id, model, timestamp_ms) into one event
                #[derive(Default)]
                struct GroupedMetric {
                    provider_id: String,
                    session_id: String,
                    model_id: String,
                    ts_ms: i64,
                    input: i64,
                    output: i64,
                    cache_read: i64,
                    cache_write: i64,
                    reasoning: i64,
                }
                let mut grouped: HashMap<(String, String, i64), GroupedMetric> = HashMap::new();

                for dp in dps {
                    let attrs = extract_attrs(dp.get("attributes"));
                    let ts_ms = parse_nano_to_ms(dp.get("timeUnixNano"))
                        .unwrap_or_else(|| Utc::now().timestamp_millis());
                    let val = dp
                        .get("asInt")
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<i64>().ok()).or_else(|| v.as_i64()))
                        .or_else(|| dp.get("asDouble").and_then(|v| v.as_f64()).map(|f| f.round() as i64))
                        .unwrap_or(0)
                        .max(0);
                    if val == 0 {
                        continue;
                    }

                    let model_id = attrs
                        .get("model")
                        .or_else(|| attrs.get("gen_ai.request.model"))
                        .cloned()
                        .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
                    let session_id = attrs
                        .get("session.id")
                        .cloned()
                        .unwrap_or_else(|| "otlp-metrics".to_string());
                    let provider_id = attrs
                        .get("gen_ai.system")
                        .map(|s| map_system_to_provider(s))
                        .unwrap_or_else(|| default_provider.clone());
                    let token_type = attrs
                        .get("type")
                        .or_else(|| attrs.get("gen_ai.token.type"))
                        .map(|s| s.to_ascii_lowercase())
                        .unwrap_or_else(|| "input".to_string());

                    let entry = grouped
                        .entry((session_id.clone(), model_id.clone(), ts_ms))
                        .or_insert_with(|| GroupedMetric {
                            provider_id,
                            session_id,
                            model_id,
                            ts_ms,
                            ..Default::default()
                        });

                    if token_type.contains("cache_read") || token_type == "cacheread" {
                        entry.cache_read += val;
                    } else if token_type.contains("cache_creation")
                        || token_type.contains("cache_write")
                        || token_type == "cachecreation"
                    {
                        entry.cache_write += val;
                    } else if token_type.contains("reason") || token_type.contains("thinking") {
                        entry.reasoning += val;
                    } else if token_type.contains("output") || token_type.contains("completion") {
                        entry.output += val;
                    } else {
                        entry.input += val;
                    }
                }

                for (_, g) in grouped {
                    let event_id = compute_event_id(
                        &g.provider_id,
                        &g.session_id,
                        &format!("otlp-metric-{}", g.model_id),
                        g.ts_ms,
                    );
                    events.push(NormalizedUsageEvent {
                        event_id,
                        device_id: device_id.to_string(),
                        provider_id: g.provider_id,
                        session_id: g.session_id,
                        project_label: "otlp".to_string(),
                        model_id: g.model_id,
                        timestamp_ms: g.ts_ms,
                        input_tokens: g.input,
                        output_tokens: g.output,
                        cache_read_tokens: g.cache_read,
                        cache_write_tokens: g.cache_write,
                        reasoning_tokens: g.reasoning,
                        observable_only: false,
                        duration_ms: None,
                        ttft_ms: None,
                        output_tps: None,
                        estimated_cost_micro_usd: 0,
                        cache_saved_micro_usd: 0,
                    });
                }
            }
        }
    }
    events
}

/// Parse OTLP/JSON `resourceLogs` (e.g. Claude Code `claude_code.api_request` events).
pub fn parse_otlp_logs_json(payload: &Value, device_id: &str) -> Vec<NormalizedUsageEvent> {
    let mut events = Vec::new();
    let Some(res_logs) = payload.get("resourceLogs").and_then(|v| v.as_array()) else {
        return events;
    };

    for rl in res_logs {
        let res_attrs = extract_attrs(rl.get("resource").and_then(|r| r.get("attributes")));
        let default_provider = infer_provider_from_attrs(&res_attrs);

        let Some(scope_logs) = rl.get("scopeLogs").and_then(|v| v.as_array()) else {
            continue;
        };
        for sl in scope_logs {
            let Some(records) = sl.get("logRecords").and_then(|v| v.as_array()) else {
                continue;
            };
            for (idx, rec) in records.iter().enumerate() {
                let attrs = extract_attrs(rec.get("attributes"));
                let event_name = attrs.get("event.name").cloned().unwrap_or_default();

                let body_str = rec
                    .get("body")
                    .and_then(|b| b.get("stringValue"))
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                if !event_name.contains("api_request") && !body_str.contains("api_request") {
                    continue;
                }

                let input = parse_attr_i64(&attrs, &["input_tokens", "gen_ai.usage.input_tokens"]);
                let output =
                    parse_attr_i64(&attrs, &["output_tokens", "gen_ai.usage.output_tokens"]);
                let cache_read = parse_attr_i64(&attrs, &["cache_read_tokens"]);
                let cache_write = parse_attr_i64(&attrs, &["cache_creation_tokens"]);
                let reasoning = parse_attr_i64(&attrs, &["reasoning_tokens"]);
                if input + output + cache_read + cache_write + reasoning == 0 {
                    continue;
                }

                let ts_ms = parse_nano_to_ms(rec.get("timeUnixNano"))
                    .unwrap_or_else(|| Utc::now().timestamp_millis());
                let duration_ms = attrs
                    .get("duration_ms")
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|&d| d > 0);
                let output_tps = duration_ms.and_then(|dur| {
                    let eff = output + reasoning;
                    if eff > 0 && dur > 0 {
                        Some(eff as f64 / (dur as f64 / 1000.0))
                    } else {
                        None
                    }
                });

                let cost_micro_usd = attrs
                    .get("cost_usd")
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|c| c.is_finite() && *c > 0.0)
                    .map(|c| (c * 1_000_000.0).round() as i64)
                    .unwrap_or(0);

                let model_id = attrs
                    .get("model")
                    .or_else(|| attrs.get("gen_ai.request.model"))
                    .cloned()
                    .unwrap_or_else(|| "claude-sonnet-4-5".to_string());
                let session_id = attrs
                    .get("session.id")
                    .cloned()
                    .unwrap_or_else(|| "otlp-logs".to_string());
                let provider_id = attrs
                    .get("gen_ai.system")
                    .map(|s| map_system_to_provider(s))
                    .unwrap_or_else(|| default_provider.clone());
                let step_key = attrs
                    .get("request_id")
                    .cloned()
                    .unwrap_or_else(|| format!("otlp-log-{idx}"));

                let event_id = compute_event_id(&provider_id, &session_id, &step_key, ts_ms);

                events.push(NormalizedUsageEvent {
                    event_id,
                    device_id: device_id.to_string(),
                    provider_id,
                    session_id,
                    project_label: "otlp".to_string(),
                    model_id,
                    timestamp_ms: ts_ms,
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: cache_read,
                    cache_write_tokens: cache_write,
                    reasoning_tokens: reasoning,
                    observable_only: false,
                    duration_ms,
                    ttft_ms: None,
                    output_tps,
                    estimated_cost_micro_usd: cost_micro_usd,
                    cache_saved_micro_usd: 0,
                });
            }
        }
    }

    events
}

fn extract_attrs(arr: Option<&Value>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(items) = arr.and_then(|v| v.as_array()) else {
        return map;
    };
    for item in items {
        let Some(k) = item.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(val_obj) = item.get("value") else {
            continue;
        };
        if let Some(s) = val_obj.get("stringValue").and_then(|v| v.as_str()) {
            map.insert(k.to_string(), s.to_string());
        } else if let Some(i) = val_obj.get("intValue") {
            if let Some(s) = i.as_str() {
                map.insert(k.to_string(), s.to_string());
            } else if let Some(n) = i.as_i64() {
                map.insert(k.to_string(), n.to_string());
            }
        } else if let Some(d) = val_obj.get("doubleValue").and_then(|v| v.as_f64()) {
            map.insert(k.to_string(), d.to_string());
        }
    }
    map
}

fn parse_attr_i64(attrs: &HashMap<String, String>, keys: &[&str]) -> i64 {
    for k in keys {
        if let Some(v) = attrs.get(*k) {
            if let Ok(n) = v.parse::<i64>() {
                return n.max(0);
            }
        }
    }
    0
}

fn parse_nano_to_ms(val: Option<&Value>) -> Option<i64> {
    let v = val?;
    let nanos = v
        .as_str()
        .and_then(|s| s.parse::<i128>().ok())
        .or_else(|| v.as_i64().map(i128::from))?;
    Some((nanos / 1_000_000) as i64)
}

fn map_system_to_provider(sys: &str) -> String {
    let low = sys.to_ascii_lowercase();
    if low.contains("openai") || low.contains("codex") {
        provider_ids::CODEX.to_string()
    } else if low.contains("google") || low.contains("gemini") || low.contains("antigravity") {
        provider_ids::ANTIGRAVITY.to_string()
    } else {
        provider_ids::CLAUDE_CODE.to_string()
    }
}

fn infer_provider_from_attrs(attrs: &HashMap<String, String>) -> String {
    attrs
        .get("service.name")
        .or_else(|| attrs.get("gen_ai.system"))
        .map(|s| map_system_to_provider(s))
        .unwrap_or_else(|| provider_ids::CLAUDE_CODE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    #[test]
    fn refuses_non_loopback_otlp_bind_addresses() {
        assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::LOCALHOST), 4318).is_ok());
        assert!(validate_loopback_addr(IpAddr::V6(Ipv6Addr::LOCALHOST), 4318).is_ok());
        assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 4318).is_err());
        assert!(validate_loopback_addr(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)), 4318).is_err());
    }

    #[test]
    fn parses_claude_code_otlp_log_and_metrics_payloads() {
        let log_json = serde_json::json!({
            "resourceLogs": [{
                "resource": { "attributes": [{ "key": "service.name", "value": { "stringValue": "claude-code" } }] },
                "scopeLogs": [{
                    "logRecords": [{
                        "timeUnixNano": "1760000000000000000",
                        "attributes": [
                            { "key": "event.name", "value": { "stringValue": "claude_code.api_request" } },
                            { "key": "model", "value": { "stringValue": "claude-sonnet-4-5" } },
                            { "key": "session.id", "value": { "stringValue": "sess-otlp" } },
                            { "key": "input_tokens", "value": { "intValue": "1200" } },
                            { "key": "output_tokens", "value": { "intValue": "400" } },
                            { "key": "cache_read_tokens", "value": { "intValue": "8000" } },
                            { "key": "duration_ms", "value": { "intValue": "2000" } },
                            { "key": "cost_usd", "value": { "doubleValue": 0.012 } }
                        ]
                    }]
                }]
            }]
        });

        let events = parse_otlp_logs_json(&log_json, "dev-1");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].input_tokens, 1200);
        assert_eq!(events[0].output_tokens, 400);
        assert_eq!(events[0].cache_read_tokens, 8000);
        assert_eq!(events[0].estimated_cost_micro_usd, 12_000);
        assert_eq!(events[0].output_tps, Some(200.0));
    }
}
