use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::error::{CoreError, CoreResult};

pub const DEFAULT_WORKER_SCRIPT_NAME: &str = "tokenvane-sync-worker";
pub const WORKER_COMPATIBILITY_DATE: &str = "2026-03-01";

pub const EMBEDDED_WORKER_INDEX_JS: &str =
    include_str!("../../../../services/sync-worker/bundle/index.js");
pub const EMBEDDED_WORKER_HUB_JS: &str =
    include_str!("../../../../services/sync-worker/bundle/hub.js");

/// Pre-assembled Cloudflare Workers v4 API multipart bundle ready for direct HTTPS `PUT`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudflareWorkerUploadBundle {
    pub worker_name: String,
    pub bearer_token: String,
    pub e2ee_enabled: bool,
    pub e2ee_passphrase: Option<String>,
    pub content_type: String,
    pub multipart_body_base64: String,
}

fn generate_hex_secret(byte_len: usize) -> CoreResult<String> {
    if byte_len == 0 {
        return Err(CoreError::Validation(
            "Secret byte length must be greater than zero".to_string(),
        ));
    }
    let mut bytes = vec![0u8; byte_len];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| CoreError::Validation(format!("Failed to generate random secret: {e}")))?;
    Ok(hex::encode(bytes))
}

pub fn normalize_worker_script_name(raw: Option<&str>) -> CoreResult<String> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_WORKER_SCRIPT_NAME.to_string());
    };
    let lower = trimmed.to_ascii_lowercase();
    if lower.len() > 63 {
        return Err(CoreError::Validation(
            "Worker 名称长度需控制在 63 个字符以内".to_string(),
        ));
    }
    let bytes = lower.as_bytes();
    let valid_edges = bytes
        .first()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && bytes
            .last()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let valid_chars = bytes
        .iter()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if !valid_edges || !valid_chars {
        return Err(CoreError::Validation(
            "Worker 名称仅允许小写字母、数字与连字符，且须以字母或数字开头和结尾".to_string(),
        ));
    }
    Ok(lower)
}

fn build_worker_metadata_json(bearer_token: &str, include_migration: bool) -> CoreResult<String> {
    let mut metadata = json!({
        "main_module": "index.js",
        "compatibility_date": WORKER_COMPATIBILITY_DATE,
        "bindings": [
            {
                "type": "durable_object_namespace",
                "name": "SYNC_HUB",
                "class_name": "SyncHubDO"
            },
            {
                "type": "secret_text",
                "name": "SYNC_TOKEN",
                "text": bearer_token
            }
        ]
    });

    if include_migration {
        if let Some(obj) = metadata.as_object_mut() {
            obj.insert(
                "migrations".to_string(),
                json!({
                    "new_tag": "v1",
                    "new_sqlite_classes": ["SyncHubDO"]
                }),
            );
        }
    }

    serde_json::to_string(&metadata).map_err(CoreError::from)
}

fn build_worker_multipart_body(
    boundary: &str,
    metadata_json: &str,
    index_js: &str,
    hub_js: &str,
) -> Vec<u8> {
    let mut body = Vec::with_capacity(
        metadata_json.len() + index_js.len() + hub_js.len() + boundary.len() * 4 + 256,
    );

    let push_part = |buf: &mut Vec<u8>, header: &str, content: &str| {
        buf.extend_from_slice(b"--");
        buf.extend_from_slice(boundary.as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(header.as_bytes());
        buf.extend_from_slice(b"\r\n\r\n");
        buf.extend_from_slice(content.as_bytes());
        buf.extend_from_slice(b"\r\n");
    };

    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"metadata\"\r\nContent-Type: application/json",
        metadata_json,
    );
    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"index.js\"; filename=\"index.js\"\r\nContent-Type: application/javascript+module",
        index_js,
    );
    push_part(
        &mut body,
        "Content-Disposition: form-data; name=\"hub.js\"; filename=\"hub.js\"\r\nContent-Type: application/javascript+module",
        hub_js,
    );

    body.extend_from_slice(b"--");
    body.extend_from_slice(boundary.as_bytes());
    body.extend_from_slice(b"--\r\n");
    body
}

/// Normalize Worker inputs, generate missing cryptographically random secrets, and build a
/// ready-to-upload Cloudflare v4 `multipart/form-data` ES Module Worker payload.
pub fn build_cloudflare_worker_upload_bundle(
    worker_name: Option<&str>,
    bearer_token: Option<&str>,
    e2ee_enabled: bool,
    e2ee_passphrase: Option<&str>,
    include_migration: bool,
) -> CoreResult<CloudflareWorkerUploadBundle> {
    let resolved_name = normalize_worker_script_name(worker_name)?;
    let resolved_token = match bearer_token.map(str::trim).filter(|s| !s.is_empty()) {
        Some(tok) => tok.to_string(),
        None => generate_hex_secret(24)?,
    };
    let resolved_passphrase = if e2ee_enabled {
        Some(
            match e2ee_passphrase.map(str::trim).filter(|s| !s.is_empty()) {
                Some(pass) => pass.to_string(),
                None => generate_hex_secret(16)?,
            },
        )
    } else {
        None
    };

    let boundary = format!("----TokenVaneWorkerBoundary{}", generate_hex_secret(8)?);
    let metadata_json = build_worker_metadata_json(&resolved_token, include_migration)?;
    let raw_body = build_worker_multipart_body(
        &boundary,
        &metadata_json,
        EMBEDDED_WORKER_INDEX_JS,
        EMBEDDED_WORKER_HUB_JS,
    );

    Ok(CloudflareWorkerUploadBundle {
        worker_name: resolved_name,
        bearer_token: resolved_token,
        e2ee_enabled,
        e2ee_passphrase: resolved_passphrase,
        content_type: format!("multipart/form-data; boundary={boundary}"),
        multipart_body_base64: STANDARD.encode(raw_body),
    })
}
