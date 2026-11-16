use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tokentracker_core::domain::{
    compute_event_id, provider_ids, sanitize_project_label, NormalizedUsageEvent,
};

use crate::providers::compatible_common::{
    appdata_dir, init_generic_context, local_appdata_dir, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, now_ms, parse_timestamp_ms, user_home_dir,
    FileParseContext, ParsedLineResult, ProviderLogParser,
};

const KIMI_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::KIMI_CODE,
    default_model: "kimi-k2-0905-preview",
    prefer_routed_model: false,
};

/// Read-only parser for local Kimi Code / Kimi CLI JSONL logs
/// (`~/.kimi/{sessions,logs}`, `~/.kimi-code/{sessions,logs}`, `%LOCALAPPDATA%/kimi-code`, env overrides).
#[derive(Debug, Default, Clone)]
pub struct KimiCodeParser;

impl KimiCodeParser {
    pub fn new() -> Self {
        Self
    }
}

fn push_kimi_base_dir(roots: &mut BTreeSet<PathBuf>, base: PathBuf) {
    roots.insert(base.join("sessions"));
    roots.insert(base.join("logs"));
}

fn read_session_metadata(
    meta_path: &Path,
) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    let Ok(meta) = std::fs::metadata(meta_path) else {
        return (None, None, None, None);
    };
    if !meta.is_file() || meta.len() > 65_536 {
        return (None, None, None, None);
    }
    let Ok(raw) = std::fs::read_to_string(meta_path) else {
        return (None, None, None, None);
    };
    let Ok(val) = serde_json::from_str::<Value>(&raw) else {
        return (None, None, None, None);
    };

    let non_empty = |keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| {
            val.get(*k)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
        })
    };

    let session_id = non_empty(&["session_id", "id"]);
    let work_dir = non_empty(&[
        "work_dir",
        "cwd",
        "workspace",
        "workspace_dir",
        "project_dir",
        "project_root",
    ]);
    let model = non_empty(&["model", "model_id", "model_name", "modelName"]);
    let title = non_empty(&["title", "name", "summary", "session_title", "topic"]);
    (session_id, work_dir, model, title)
}

fn read_kimi_sibling_prompt(session_dir: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    for filename in ["context.jsonl", "wire.jsonl"] {
        let p = session_dir.join(filename);
        let Ok(file) = crate::watcher::open_shared_readonly(&p) else {
            continue;
        };
        let reader = BufReader::new(file);
        for line_res in reader.lines().take(32) {
            let Ok(line) = line_res else {
                break;
            };
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Ok(val) = serde_json::from_str::<Value>(trimmed) else {
                continue;
            };
            let role = val
                .get("role")
                .or_else(|| val.pointer("/message/type"))
                .or_else(|| val.pointer("/params/type"))
                .or_else(|| val.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if role.eq_ignore_ascii_case("user")
                || role.eq_ignore_ascii_case("turnbegin")
                || role.eq_ignore_ascii_case("turn_begin")
            {
                let candidate = val
                    .pointer("/message/payload/user_input")
                    .or_else(|| val.pointer("/params/payload/user_input"))
                    .or_else(|| val.pointer("/payload/user_input"))
                    .or_else(|| val.get("user_input"))
                    .or_else(|| val.get("content"))
                    .or_else(|| val.pointer("/message/content"));
                if let Some(text) = crate::providers::extract_text_from_content(candidate) {
                    return Some(text);
                }
            }
        }
    }
    None
}

fn is_named_dir(p: &Path, name: &str) -> bool {
    p.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(name))
}

impl ProviderLogParser for KimiCodeParser {
    fn provider_id(&self) -> &str {
        provider_ids::KIMI_CODE
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        let mut roots = BTreeSet::new();
        for env_key in ["KIMI_CODE_HOME", "KIMI_DATA_DIR", "KIMI_SHARE_DIR"] {
            if let Some(val) = std::env::var_os(env_key) {
                let p = PathBuf::from(val);
                if !p.as_os_str().is_empty() {
                    push_kimi_base_dir(&mut roots, p.clone());
                    roots.insert(p);
                }
            }
        }
        if let Some(home) = user_home_dir() {
            for sub in [".kimi", ".kimi-code"] {
                push_kimi_base_dir(&mut roots, home.join(sub));
            }
        }
        if let Some(app) = appdata_dir() {
            for sub in ["kimi", "kimi-code", "Kimi", "KimiCode"] {
                push_kimi_base_dir(&mut roots, app.join(sub));
            }
        }
        if let Some(local_app) = local_appdata_dir() {
            for sub in ["kimi", "kimi-code", "Kimi", "KimiCode"] {
                push_kimi_base_dir(&mut roots, local_app.join(sub));
            }
        }
        roots.into_iter().collect()
    }

    fn matches_file(&self, path: &Path) -> bool {
        let is_jsonl = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"));
        if !is_jsonl {
            return false;
        }
        // Avoid double-counting when Kimi CLI writes both `wire.jsonl` and `context.jsonl` in one session folder.
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("context.jsonl"))
            && path.with_file_name("wire.jsonl").exists()
        {
            return false;
        }
        true
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        let mut ctx =
            init_generic_context(path, device_id, "kimi-session", KIMI_PROFILE.default_model);

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(file_name.as_str(), "wire.jsonl" | "context.jsonl") {
            if let Some(parent) = path.parent() {
                if let Some(grandparent) = parent.parent() {
                    if is_named_dir(grandparent, "agents") {
                        // Layout: .../<ws>/<session-id>/agents/<agent-id>/wire.jsonl
                        let agent_id = parent
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("subagent");
                        if let Some(session_dir) = grandparent.parent() {
                            if let Some(ws_dir) = session_dir.parent() {
                                if !is_named_dir(ws_dir, "sessions")
                                    && !is_named_dir(ws_dir, "logs")
                                {
                                    ctx.project_label =
                                        sanitize_project_label(ws_dir.file_name().and_then(|n| n.to_str()));
                                }
                            }
                            let (meta_sid, meta_work_dir, meta_model, meta_title) =
                                read_session_metadata(&session_dir.join("metadata.json"));
                            let base_sid = meta_sid.or_else(|| {
                                session_dir
                                    .file_name()
                                    .and_then(|n| n.to_str())
                                    .filter(|n| {
                                        !n.eq_ignore_ascii_case("sessions")
                                            && !n.eq_ignore_ascii_case("logs")
                                    })
                                    .map(ToString::to_string)
                            });
                            if let Some(sid) = base_sid {
                                ctx.session_id = format!("{sid}/{agent_id}");
                            }
                            if let Some(wd) = meta_work_dir {
                                ctx.project_label = sanitize_project_label(Some(&wd));
                            }
                            if let Some(m) = meta_model {
                                ctx.current_model = Some(m);
                            }
                            if let Some(t) = meta_title {
                                ctx.record_session_title(&t, "explicit", now_ms());
                            } else if let Some(prompt) = read_kimi_sibling_prompt(session_dir) {
                                ctx.record_session_title(&prompt, "user_prompt", 1);
                            }
                        }
                    } else {
                        // Layout: .../<ws>/<session-id>/wire.jsonl
                        if let Some(sid) = parent.file_name().and_then(|n| n.to_str()) {
                            if !sid.eq_ignore_ascii_case("sessions")
                                && !sid.eq_ignore_ascii_case("logs")
                            {
                                ctx.session_id = sid.to_string();
                            }
                        }
                        if !is_named_dir(grandparent, "sessions")
                            && !is_named_dir(grandparent, "logs")
                        {
                            ctx.project_label = sanitize_project_label(
                                grandparent.file_name().and_then(|n| n.to_str()),
                            );
                        }
                    }
                }
                let (meta_sid, meta_work_dir, meta_model, meta_title) =
                    read_session_metadata(&parent.join("metadata.json"));
                if let Some(sid) = meta_sid {
                    ctx.session_id = sid;
                }
                if let Some(wd) = meta_work_dir {
                    ctx.project_label = sanitize_project_label(Some(&wd));
                }
                if let Some(m) = meta_model {
                    ctx.current_model = Some(m);
                }
                if let Some(t) = meta_title {
                    ctx.record_session_title(&t, "explicit", now_ms());
                } else if ctx.session_title.is_none() {
                    if let Some(prompt) = read_kimi_sibling_prompt(parent) {
                        ctx.record_session_title(&prompt, "user_prompt", 1);
                    }
                }
            }
        }

        ctx
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        if let Some(parsed) = parse_compatible_jsonl_line(line, ctx, KIMI_PROFILE) {
            return Some(parsed);
        }

        // Fallback for legacy Kimi CLI `context.jsonl` entries: `{"role":"_usage","token_count":1500}`
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let val: Value = serde_json::from_str(trimmed).ok()?;
        let is_usage_role = val
            .get("role")
            .and_then(|v| v.as_str())
            .is_some_and(|r| r.eq_ignore_ascii_case("_usage"));
        if !is_usage_role {
            return None;
        }
        let count = val
            .get("token_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            .max(0);
        if count <= 0 {
            return None;
        }
        let delta = match ctx.last_context_tokens {
            Some(prev) if count >= prev => count - prev,
            _ => count,
        };
        ctx.last_context_tokens = Some(count);
        if delta <= 0 {
            return None;
        }

        let ts_ms = parse_timestamp_ms(
            val.get("timestamp")
                .or_else(|| val.get("created_at"))
                .or_else(|| val.get("time")),
        )
        .unwrap_or_else(now_ms);
        let model_id = val
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .or_else(|| ctx.current_model.clone())
            .unwrap_or_else(|| KIMI_PROFILE.default_model.to_string());
        let event_id = compute_event_id(
            provider_ids::KIMI_CODE,
            &ctx.session_id,
            &format!("ctx-line-{}", ctx.line_number),
            ts_ms,
        );

        Some(ParsedLineResult {
            event: Some(NormalizedUsageEvent {
                event_id,
                device_id: ctx.device_id.clone(),
                provider_id: provider_ids::KIMI_CODE.to_string(),
                session_id: ctx.session_id.clone(),
                project_label: ctx.project_label.clone(),
                model_id,
                timestamp_ms: ts_ms,
                input_tokens: delta,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                observable_only: false,
                duration_ms: None,
                ttft_ms: None,
                output_tps: None,
                estimated_cost_micro_usd: 0,
                cache_saved_micro_usd: 0,
            }),
            rate_limits: Vec::new(),
            balance: None,
        })
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
