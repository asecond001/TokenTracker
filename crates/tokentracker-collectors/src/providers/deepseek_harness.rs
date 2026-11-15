use std::path::{Path, PathBuf};

use tokentracker_core::domain::{provider_ids, NormalizedUsageEvent};

use crate::providers::compatible_common::{
    default_home_and_appdata_roots, init_generic_context, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, FileParseContext, ParsedLineResult, ProviderLogParser,
};

const DEEPSEEK_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::DEEPSEEK_HARNESS,
    default_model: "deepseek-chat",
    prefer_routed_model: false,
};

/// Read-only parser for DeepSeek Harness (`dsh`, Cordis plugin runtime) JSONL session/telemetry logs.
/// Separates DeepSeek `prompt_cache_hit_tokens` from `prompt_cache_miss_tokens` for accurate Prefix Cache billing.
#[derive(Debug, Default, Clone)]
pub struct DeepSeekHarnessParser;

impl DeepSeekHarnessParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for DeepSeekHarnessParser {
    fn provider_id(&self) -> &str {
        provider_ids::DEEPSEEK_HARNESS
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        default_home_and_appdata_roots(
            &[
                ".dsh/sessions",
                ".dsh/logs",
                ".deepseek-harness/sessions",
                ".deepseek-harness/logs",
                ".deepseek/sessions",
            ],
            &["dsh/logs", "deepseek-harness/logs"],
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
            "dsh-session",
            DEEPSEEK_PROFILE.default_model,
        )
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        parse_compatible_jsonl_line(line, ctx, DEEPSEEK_PROFILE)
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
