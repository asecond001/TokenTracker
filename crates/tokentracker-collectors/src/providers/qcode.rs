use std::path::{Path, PathBuf};

use tokentracker_core::domain::{provider_ids, NormalizedUsageEvent};

use crate::providers::compatible_common::{
    default_home_and_appdata_roots, init_generic_context, parse_compatible_jsonl_line,
    CompatibleParserProfile,
};
use crate::providers::{
    collapse_duplicate_events_keep_last, FileParseContext, ParsedLineResult, ProviderLogParser,
};

const QCODE_PROFILE: CompatibleParserProfile = CompatibleParserProfile {
    provider_id: provider_ids::QCODE,
    default_model: "claude-sonnet-4-5",
    // Preserve true gateway target model (claude / gpt / deepseek / qwen) under provider_id="qcode"
    prefer_routed_model: true,
};

/// Read-only parser for QCode (`QCode.cc`) compatible gateway session logs.
#[derive(Debug, Default, Clone)]
pub struct QCodeParser;

impl QCodeParser {
    pub fn new() -> Self {
        Self
    }
}

impl ProviderLogParser for QCodeParser {
    fn provider_id(&self) -> &str {
        provider_ids::QCODE
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        default_home_and_appdata_roots(
            &[".qcode/sessions", ".qcode/logs", ".qwen-code/sessions"],
            &["qcode/logs"],
        )
    }

    fn matches_file(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        init_generic_context(path, device_id, "qcode-session", QCODE_PROFILE.default_model)
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        parse_compatible_jsonl_line(line, ctx, QCODE_PROFILE)
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
