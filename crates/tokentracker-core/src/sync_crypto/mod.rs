use std::collections::HashMap;
use std::sync::atomic::{compiler_fence, Ordering};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use sha2::{Digest, Sha256};

use crate::domain::{EncryptedSyncEnvelopeV1, SyncEnvelopeV1};
use crate::error::{CoreError, CoreResult};

pub const PBKDF2_ITERATIONS: u32 = 210_000;
pub const E2EE_SALT_LEN: usize = 16;
pub const E2EE_NONCE_LEN: usize = 12;

/// Generate a cryptographically random 16-byte KDF salt encoded as lowercase hex.
pub fn generate_kdf_salt_hex() -> CoreResult<String> {
    let mut salt = [0u8; E2EE_SALT_LEN];
    getrandom::getrandom(&mut salt)
        .map_err(|e| CoreError::Validation(format!("Failed to generate E2EE salt: {e}")))?;
    Ok(hex::encode(salt))
}

/// Derive a 256-bit AES-GCM key from a user passphrase and 16-byte hex salt using PBKDF2-HMAC-SHA256.
pub fn derive_e2ee_key(passphrase: &str, kdf_salt_hex: &str) -> CoreResult<[u8; 32]> {
    if passphrase.trim().is_empty() {
        return Err(CoreError::Validation(
            "E2EE passphrase cannot be empty".to_string(),
        ));
    }
    let salt = hex::decode(kdf_salt_hex.trim())
        .map_err(|e| CoreError::Validation(format!("Invalid E2EE KDF salt hex: {e}")))?;
    if salt.len() != E2EE_SALT_LEN {
        return Err(CoreError::Validation(format!(
            "Invalid E2EE KDF salt length: expected {E2EE_SALT_LEN} bytes, got {}",
            salt.len()
        )));
    }
    let mut key = [0u8; 32];
    pbkdf2_hmac::<Sha256>(passphrase.as_bytes(), &salt, PBKDF2_ITERATIONS, &mut key);
    Ok(key)
}

fn envelope_aad(device_id: &str, seq_ms: i64) -> String {
    format!("v1:{device_id}:{seq_ms}")
}

/// Encrypt a `SyncEnvelopeV1` into an opaque `EncryptedSyncEnvelopeV1` payload.
pub fn encrypt_sync_envelope_v1(
    envelope: &SyncEnvelopeV1,
    passphrase: &str,
    kdf_salt_hex: Option<&str>,
    seq_ms: i64,
) -> CoreResult<EncryptedSyncEnvelopeV1> {
    if envelope.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported SyncEnvelope schema_version {}",
            envelope.schema_version
        )));
    }
    let salt_hex = match kdf_salt_hex.map(str::trim).filter(|s| !s.is_empty()) {
        Some(existing) => existing.to_string(),
        None => generate_kdf_salt_hex()?,
    };
    let key = derive_e2ee_key(passphrase, &salt_hex)?;

    let mut nonce_bytes = [0u8; E2EE_NONCE_LEN];
    getrandom::getrandom(&mut nonce_bytes)
        .map_err(|e| CoreError::Validation(format!("Failed to generate AES-GCM nonce: {e}")))?;

    let plaintext = serde_json::to_vec(envelope)?;
    let aad = envelope_aad(&envelope.device_id, seq_ms);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| CoreError::Validation(format!("Failed to initialize AES-256-GCM: {e}")))?;

    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| CoreError::Validation("E2EE encryption failed".to_string()))?;

    Ok(EncryptedSyncEnvelopeV1 {
        schema_version: 1,
        device_id: envelope.device_id.clone(),
        seq_ms,
        kdf_salt_hex: salt_hex,
        nonce_hex: hex::encode(nonce_bytes),
        ciphertext_b64: STANDARD.encode(ciphertext),
    })
}

fn e2ee_cache_key(passphrase: &str, kdf_salt_hex: &str) -> String {
    let passphrase_digest = Sha256::digest(passphrase.as_bytes());
    let salt_norm = kdf_salt_hex.trim().to_ascii_lowercase();
    format!("{}:{}", hex::encode(passphrase_digest), salt_norm)
}

/// Zero all derived keys in an E2EE batch decryption cache and clear the map.
pub fn clear_key_cache(key_cache: &mut HashMap<String, [u8; 32]>) {
    for key in key_cache.values_mut() {
        key.fill(0);
    }
    compiler_fence(Ordering::SeqCst);
    key_cache.clear();
}

/// Decrypt and authenticate an `EncryptedSyncEnvelopeV1` using an already-derived 256-bit key.
pub fn decrypt_sync_envelope_v1_with_key(
    encrypted: &EncryptedSyncEnvelopeV1,
    key: &[u8; 32],
) -> CoreResult<SyncEnvelopeV1> {
    if encrypted.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported encrypted envelope schema_version {}",
            encrypted.schema_version
        )));
    }
    let nonce_bytes = hex::decode(encrypted.nonce_hex.trim())
        .map_err(|e| CoreError::Validation(format!("Invalid E2EE nonce hex: {e}")))?;
    if nonce_bytes.len() != E2EE_NONCE_LEN {
        return Err(CoreError::Validation(format!(
            "Invalid E2EE nonce length: expected {E2EE_NONCE_LEN} bytes, got {}",
            nonce_bytes.len()
        )));
    }
    let ciphertext = STANDARD
        .decode(encrypted.ciphertext_b64.trim())
        .map_err(|e| CoreError::Validation(format!("Invalid E2EE ciphertext base64: {e}")))?;

    let aad = envelope_aad(&encrypted.device_id, encrypted.seq_ms);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|e| CoreError::Validation(format!("Failed to initialize AES-256-GCM: {e}")))?;

    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: &ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| {
            CoreError::Validation(
                "E2EE decryption failed: wrong passphrase or tampered ciphertext".to_string(),
            )
        })?;

    let envelope: SyncEnvelopeV1 = serde_json::from_slice(&plaintext)?;
    if envelope.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported inner SyncEnvelope schema_version {}",
            envelope.schema_version
        )));
    }
    if envelope.device_id != encrypted.device_id {
        return Err(CoreError::Validation(
            "E2EE envelope device_id mismatch between outer header and decrypted payload"
                .to_string(),
        ));
    }
    Ok(envelope)
}

/// Decrypt an `EncryptedSyncEnvelopeV1`, reusing previously derived keys within a batch.
pub fn decrypt_sync_envelope_v1_cached(
    encrypted: &EncryptedSyncEnvelopeV1,
    passphrase: &str,
    key_cache: &mut HashMap<String, [u8; 32]>,
) -> CoreResult<SyncEnvelopeV1> {
    if encrypted.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported encrypted envelope schema_version {}",
            encrypted.schema_version
        )));
    }
    if passphrase.trim().is_empty() {
        return Err(CoreError::Validation(
            "E2EE passphrase cannot be empty".to_string(),
        ));
    }

    let cache_key = e2ee_cache_key(passphrase, &encrypted.kdf_salt_hex);
    if let Some(cached_key) = key_cache.get(&cache_key) {
        return decrypt_sync_envelope_v1_with_key(encrypted, cached_key);
    }

    let mut derived_key = derive_e2ee_key(passphrase, &encrypted.kdf_salt_hex)?;
    let result = decrypt_sync_envelope_v1_with_key(encrypted, &derived_key);
    if result.is_ok() {
        key_cache.insert(cache_key, derived_key);
    }
    derived_key.fill(0);
    compiler_fence(Ordering::SeqCst);
    result
}

/// Decrypt and authenticate an `EncryptedSyncEnvelopeV1` using the shared user passphrase.
pub fn decrypt_sync_envelope_v1(
    encrypted: &EncryptedSyncEnvelopeV1,
    passphrase: &str,
) -> CoreResult<SyncEnvelopeV1> {
    if encrypted.schema_version != 1 {
        return Err(CoreError::Validation(format!(
            "Unsupported encrypted envelope schema_version {}",
            encrypted.schema_version
        )));
    }
    let mut key = derive_e2ee_key(passphrase, &encrypted.kdf_salt_hex)?;
    let result = decrypt_sync_envelope_v1_with_key(encrypted, &key);
    key.fill(0);
    compiler_fence(Ordering::SeqCst);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SyncHourlyDelta, SyncSubscriptionSnapshot};

    fn sample_envelope() -> SyncEnvelopeV1 {
        SyncEnvelopeV1 {
            schema_version: 1,
            device_id: "dev-abc-123".to_string(),
            device_display_name: "win-desktop".to_string(),
            generated_at_ms: 1_760_000_000_000,
            since_watermark_ms: 1_759_900_000_000,
            hourly_deltas: vec![SyncHourlyDelta {
                provider_id: "kimi_code".to_string(),
                model_id: "kimi-k2".to_string(),
                bucket_start_ms: 1_760_000_000_000,
                request_count: 4,
                observable_step_count: 0,
                input_tokens: 12_000,
                output_tokens: 3_000,
                cache_read_tokens: 6_000,
                cache_write_tokens: 0,
                reasoning_tokens: 0,
                estimated_cost_micro_usd: 15_600,
                cache_saved_micro_usd: 2_700,
                total_duration_ms: 8_000,
                tps_sample_count: 4,
                tps_sum: 240.0,
                max_output_tps: 75.0,
                updated_at_ms: 1_760_000_005_000,
            }],
            subscription_snapshots: vec![SyncSubscriptionSnapshot {
                plan_id: "minimax_coding_plan".to_string(),
                provider_id: "minimax_code".to_string(),
                plan_name: "MiniMax Coding Plan".to_string(),
                window_id: "5h".to_string(),
                window_label: "5h 滚动窗口".to_string(),
                used_percent: 38.5,
                resets_at_ms: Some(1_760_010_000_000),
                source: "local_estimation".to_string(),
            }],
        }
    }

    #[test]
    fn encrypt_decrypt_roundtrip_and_tamper_detection() {
        let env = sample_envelope();
        let passphrase = "correct-horse-battery-staple";

        let encrypted =
            encrypt_sync_envelope_v1(&env, passphrase, None, 1_760_000_010_000).expect("encrypt");
        assert_eq!(encrypted.schema_version, 1);
        assert_eq!(encrypted.device_id, env.device_id);
        assert!(!encrypted.ciphertext_b64.contains("kimi-k2"));

        let decrypted = decrypt_sync_envelope_v1(&encrypted, passphrase).expect("decrypt");
        assert_eq!(decrypted, env);

        // Wrong passphrase must fail authentication
        assert!(decrypt_sync_envelope_v1(&encrypted, "wrong-passphrase").is_err());

        // Flipping one ciphertext byte must fail AES-GCM tag verification
        let mut raw_ct = STANDARD
            .decode(&encrypted.ciphertext_b64)
            .expect("decode b64");
        raw_ct[0] ^= 0x55;
        let tampered_ct = EncryptedSyncEnvelopeV1 {
            ciphertext_b64: STANDARD.encode(raw_ct),
            ..encrypted.clone()
        };
        assert!(decrypt_sync_envelope_v1(&tampered_ct, passphrase).is_err());

        // Modifying outer AAD (`seq_ms` or `device_id`) must also fail verification
        let tampered_seq = EncryptedSyncEnvelopeV1 {
            seq_ms: encrypted.seq_ms + 1,
            ..encrypted
        };
        assert!(decrypt_sync_envelope_v1(&tampered_seq, passphrase).is_err());
    }

    #[test]
    fn cached_decryption_reuses_derived_key_and_rejects_wrong_passphrase() {
        let env = sample_envelope();
        let passphrase = "cached-batch-passphrase";
        let salt = "0123456789abcdef0123456789abcdef";

        let enc_1 =
            encrypt_sync_envelope_v1(&env, passphrase, Some(salt), 1_760_000_010_000)
                .expect("encrypt 1");
        let enc_2 =
            encrypt_sync_envelope_v1(&env, passphrase, Some(salt), 1_760_000_020_000)
                .expect("encrypt 2");

        let mut cache = HashMap::new();
        assert!(decrypt_sync_envelope_v1_cached(&enc_1, "wrong-pass", &mut cache).is_err());
        assert!(cache.is_empty());

        let dec_1 =
            decrypt_sync_envelope_v1_cached(&enc_1, passphrase, &mut cache).expect("dec 1");
        assert_eq!(dec_1, env);
        assert_eq!(cache.len(), 1);

        let dec_2 =
            decrypt_sync_envelope_v1_cached(&enc_2, passphrase, &mut cache).expect("dec 2");
        assert_eq!(dec_2, env);
        assert_eq!(cache.len(), 1);

        // Warm cache for the valid passphrase must never authorize a wrong passphrase.
        assert!(decrypt_sync_envelope_v1_cached(&enc_2, "wrong-pass", &mut cache).is_err());

        // Direct key helper also rejects tampered ciphertext.
        let key = derive_e2ee_key(passphrase, salt).expect("derive");
        let mut raw_ct = STANDARD.decode(&enc_1.ciphertext_b64).expect("decode b64");
        raw_ct[0] ^= 0x11;
        let tampered = EncryptedSyncEnvelopeV1 {
            ciphertext_b64: STANDARD.encode(raw_ct),
            ..enc_1
        };
        assert!(decrypt_sync_envelope_v1_with_key(&tampered, &key).is_err());

        clear_key_cache(&mut cache);
        assert!(cache.is_empty());
    }
}
