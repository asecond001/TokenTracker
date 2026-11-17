use std::path::{Path, PathBuf};

use tokvane_core::declarative::{extract_custom_rule_line, resolve_custom_provider_id};
use tokvane_core::domain::{CustomProviderRule, NormalizedUsageEvent};

use crate::providers::compatible_common::init_generic_context;
use crate::providers::{
    collapse_duplicate_events_keep_last, now_ms, FileParseContext, ParsedLineResult,
    ProviderLogParser,
};

/// Dynamic read-only log parser driven by a user-defined [`CustomProviderRule`].
#[derive(Debug, Clone)]
pub struct CustomRuleLogParser {
    rule: CustomProviderRule,
    resolved_provider_id: String,
    extensions: Vec<String>,
}

impl CustomRuleLogParser {
    pub fn new(rule: CustomProviderRule) -> Self {
        let resolved_provider_id = resolve_custom_provider_id(&rule);
        let mut extensions: Vec<String> = rule
            .mapping_spec
            .file_extensions
            .iter()
            .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
        if extensions.is_empty() {
            extensions = vec!["jsonl".to_string(), "ndjson".to_string()];
        }
        Self {
            rule,
            resolved_provider_id,
            extensions,
        }
    }

    pub fn rule(&self) -> &CustomProviderRule {
        &self.rule
    }
}

impl ProviderLogParser for CustomRuleLogParser {
    fn provider_id(&self) -> &str {
        &self.resolved_provider_id
    }

    fn default_roots(&self) -> Vec<PathBuf> {
        self.rule
            .log_roots
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    fn matches_file(&self, path: &Path) -> bool {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return false;
        };
        let lower = ext.to_ascii_lowercase();
        self.extensions.iter().any(|allowed| allowed == &lower)
    }

    fn init_context(&self, path: &Path, device_id: &str) -> FileParseContext {
        init_generic_context(path, device_id, &self.resolved_provider_id, "custom-model")
    }

    fn parse_line(&self, line: &str, ctx: &mut FileParseContext) -> Option<ParsedLineResult> {
        ctx.line_number += 1;
        let event = extract_custom_rule_line(
            &self.rule,
            line,
            &ctx.device_id,
            &ctx.session_id,
            &ctx.project_label,
            now_ms(),
            ctx.line_number,
        )?;
        Some(ParsedLineResult {
            event: Some(event),
            rate_limits: Vec::new(),
            balance: None,
        })
    }

    fn finalize_batch(&self, events: Vec<NormalizedUsageEvent>) -> Vec<NormalizedUsageEvent> {
        collapse_duplicate_events_keep_last(events)
    }
}
