use std::path::{Path, PathBuf};

use tokentracker_core::domain::{provider_ids, NormalizedUsageEvent};

use crate::providers::compatible_common::{
    default_home_and_appdata_roots, init_generic_context, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, FileParseContext, ParsedLineResult, ProviderLogParser,
};

const KIMI_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::KIMI_CODE,
    default_model: "kimi-k2-0905-preview",
    prefer_routed_model: false,
};

/// Read-only parser for local Kimi Code / Kimi CLI JSONL logs (`~/.kimi/sessions` & `~/.kimi/logs`).
#[derive(Debug, Default, Clone)]
pub struct KimiCodeParser;

impl KimiCodeParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for KimiCodeParser {
    fn provider_id(&self) -> &str {
        provider_ids::KIMI_CODE
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        default_home_and_appdata_roots(
            &[".kimi/sessions", ".kimi/logs"],
            &["kimi/sessions", "kimi/logs"],
        )
    }

    fn matches_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        init_generic_context(path, device_id, "kimi-session", KIMI_PROFILE.default_model)
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        parse_compatible_jsonl_line(line, ctx, KIMI_PROFILE)
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
