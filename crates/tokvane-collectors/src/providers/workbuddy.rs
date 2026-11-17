use std::path::{Path, PathBuf};

use tokvane_core::domain::{provider_ids, NormalizedUsageEvent};

use crate::providers::compatible_common::{
    default_home_and_appdata_roots, init_generic_context, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, FileParseContext, ParsedLineResult, ProviderLogParser,
};

const WORKBUDDY_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::WORKBUDDY,
    default_model: "kimi-k2-0905-preview",
    // Prioritize actual routed backend model (`routed_model` / `backend_model`) when switching models
    prefer_routed_model: true,
};

/// Read-only parser for local Workbuddy workspace/session logs with underlying backend model attribution.
#[derive(Debug, Default, Clone)]
pub struct WorkbuddyParser;

impl WorkbuddyParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for WorkbuddyParser {
    fn provider_id(&self) -> &str {
        provider_ids::WORKBUDDY
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        default_home_and_appdata_roots(
            &[".workbuddy/sessions", ".workbuddy/logs"],
            &["Workbuddy/logs", "Workbuddy/sessions"],
        )
    }

    fn matches_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        init_generic_context(
            path,
            device_id,
            "workbuddy-session",
            WORKBUDDY_PROFILE.default_model,
        )
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        parse_compatible_jsonl_line(line, ctx, WORKBUDDY_PROFILE)
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
