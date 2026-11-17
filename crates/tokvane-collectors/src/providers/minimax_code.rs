use std::path::{Path, PathBuf};

use tokvane_core::domain::{provider_ids, NormalizedUsageEvent};

use crate::providers::compatible_common::{
    default_home_and_appdata_roots, init_generic_context, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, FileParseContext, ParsedLineResult, ProviderLogParser,
};

const MINIMAX_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::MINIMAX_CODE,
    default_model: "MiniMax-M2.5",
    prefer_routed_model: false,
};

/// Read-only parser for MiniMax Code CLI JSONL session logs (Anthropic-compatible & OpenAI-compatible usage).
#[derive(Debug, Default, Clone)]
pub struct MiniMaxCodeParser;

impl MiniMaxCodeParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for MiniMaxCodeParser {
    fn provider_id(&self) -> &str {
        provider_ids::MINIMAX_CODE
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        default_home_and_appdata_roots(
            &[
                ".minimax/sessions",
                ".minimax/logs",
                ".minimax-code/sessions",
                ".minimax-code/logs",
            ],
            &["minimax/logs"],
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
            "minimax-session",
            MINIMAX_PROFILE.default_model,
        )
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        parse_compatible_jsonl_line(line, ctx, MINIMAX_PROFILE)
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
