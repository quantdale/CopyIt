//! The vault behind password-protected snippets: key derivation (Argon2id),
//! authenticated encryption (XChaCha20-Poly1305), the canary that verifies a
//! candidate password, the session lock/unlock state, and the hint/masking
//! helpers the censored cards use.
//!
//! Everything here is pure and returns `Result` — no `unwrap`, no panics, no I/O.
//! The encrypted bytes and the KDF salt are transported as base64 strings so the
//! data model (`VaultMeta` and `Protection`) is stored in SQLite text columns;
//! the same values can be read from legacy JSON during migration.
//!
//! Security model: the derived key exists only while the state is `Unlocked` and
//! is securely zeroized by `Drop` (and by `lock()`) using the `zeroize` crate.
//! The password itself is never stored in any form; a candidate password is
//! verified by attempting to decrypt the canary — correct iff the authenticated
//! decryption succeeds.

use crate::model::Protection;
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chacha20poly1305::aead::{Aead, Key, Nonce};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// Fixed plaintext used as the canary. It must never change: a canary written by
/// an old build has to decode with the current one, or every unlock would fail.
pub const CANARY_PLAINTEXT: &[u8] = b"copyit-vault-canary-v1";

/// KDF salt length (16 random bytes, stored in the SQLite app_config row).
pub const SALT_LEN: usize = 16;
/// Fresh per-encryption nonce length for XChaCha20.
pub const NONCE_LEN: usize = 24;
/// Derived key length.
pub const KEY_LEN: usize = 32;
/// Body length threshold (in characters) at which a hint is stored.
pub const HINT_MIN_BODY_CHARS: usize = 12;
/// Characters of the body exposed as a hint (when the body is long enough).
pub const HINT_CHARS: usize = 5;
/// Masking suffix appended to a hint on censored cards.
pub const MASKED_SUFFIX: &str = "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}";
/// Minimum vault password length (enforced only at creation time; unlock works
/// with any length to avoid breaking vaults created by older versions).
pub const MIN_VAULT_PASSWORD_LEN: usize = 8;

/// Error type for every vault operation. Each variant carries a human-readable
/// detail; the app surfaces these (inline in the unlock modal or in the top-bar
/// warning banner) and never panics.
#[derive(Debug)]
pub enum VaultError {
    /// Argon2 key derivation failed.
    KeyDerivation(String),
    /// Base64 encode/decode failed (corrupt JSON or nonce/ciphertext).
    Encoding(String),
    /// Authenticated encryption failed.
    Encryption(String),
    /// Authenticated decryption failed (wrong key, tampered ciphertext, ...).
    Decryption(String),
    /// Password is too short for vault creation.
    WeakPassword(String),
}

impl std::fmt::Display for VaultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VaultError::KeyDerivation(e) => write!(f, "key derivation failed ({e})"),
            VaultError::Encoding(e) => write!(f, "vault data is corrupt ({e})"),
            VaultError::Encryption(e) => write!(f, "encryption failed ({e})"),
            VaultError::Decryption(e) => write!(f, "wrong password or damaged data ({e})"),
            VaultError::WeakPassword(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for VaultError {}

/// Vault metadata persisted in `config.json` under the `vault` key. Holds exactly
/// what unlocking needs (the KDF salt) and nothing that reveals the password: the
/// canary is just ciphertext of a fixed known string.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultMeta {
    /// Base64-encoded 16-byte Argon2id salt.
    pub salt: String,
    /// Base64-encoded 24-byte nonce used for the canary ciphertext.
    pub nonce: String,
    /// Base64-encoded canary ciphertext.
    pub canary: String,
}

/// Session lock state. The key (`[u8; 32]`) is memory-only: `lock()` zeroizes it
/// and the vault starts `Locked` on every launch.
#[derive(Clone, PartialEq)]
pub enum VaultState {
    /// No key in memory; protected actions prompt for the password.
    Locked,
    /// The derived key is cached for the rest of the session.
    Unlocked([u8; 32]),
}

impl Drop for VaultState {
    fn drop(&mut self) {
        if let VaultState::Unlocked(key) = self {
            key.zeroize();
        }
    }
}

impl VaultState {
    /// A fresh, locked state (always the starting state on app launch).
    pub fn new() -> Self {
        VaultState::Locked
    }

    pub fn is_locked(&self) -> bool {
        matches!(self, VaultState::Locked)
    }

    pub fn is_unlocked(&self) -> bool {
        matches!(self, VaultState::Unlocked(_))
    }

    /// The cached key, when unlocked.
    pub fn key(&self) -> Option<&[u8; KEY_LEN]> {
        match self {
            VaultState::Unlocked(k) => Some(k),
            VaultState::Locked => None,
        }
    }

    /// Zeroizes the cached key, then locks the vault.
    pub fn lock(&mut self) {
        if let VaultState::Unlocked(key) = self {
            key.zeroize();
        }
        *self = VaultState::Locked;
    }

    /// Verifies `password` against the canary described by `meta`; on success
    /// caches the derived key and marks the vault unlocked. A wrong password (or
    /// any verification failure) leaves the state unchanged — still `Locked`.
    #[allow(dead_code)]
    pub fn unlock(&mut self, password: &str, meta: &VaultMeta) -> Result<(), VaultError> {
        let key = verify_password(password, meta)?;
        self.key_from_verified(key)
    }

    /// Installs an already-verified key (used by vault creation, which derives the
    /// key directly from the fresh password instead of re-verifying).
    pub fn unlock_with_key(&mut self, key: [u8; KEY_LEN]) {
        *self = VaultState::Unlocked(key);
    }

    #[allow(dead_code)]
    fn key_from_verified(&mut self, key: [u8; KEY_LEN]) -> Result<(), VaultError> {
        *self = VaultState::Unlocked(key);
        Ok(())
    }
}

impl Default for VaultState {
    fn default() -> Self {
        VaultState::new()
    }
}

/// Fills a `len`-byte buffer with cryptographically secure random bytes. OsRng's
/// fill is infallible; the Result wrapper keeps every vault call fallible by shape.
fn random_bytes<const N: usize>() -> Result<[u8; N], VaultError> {
    let mut buf = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    Ok(buf)
}

/// Derives the 32-byte vault key from `password` and `salt` with Argon2id
/// (RFC 9106's moderate parameters: m = 19 MiB, t = 2, p = 1).
pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; KEY_LEN], VaultError> {
    let params =
        Params::new(19 * 1024, 2, 1, None).map_err(|e| VaultError::KeyDerivation(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| VaultError::KeyDerivation(e.to_string()))?;
    Ok(out)
}

/// Encrypts `plaintext` under `key` with a fresh random nonce. Returns
/// `(nonce_base64, ciphertext_base64)`.
pub fn encrypt(key: &[u8; KEY_LEN], plaintext: &[u8]) -> Result<(String, String), VaultError> {
    let nonce_bytes = random_bytes::<NONCE_LEN>()?;
    let cipher = XChaCha20Poly1305::new(Key::<XChaCha20Poly1305>::from_slice(key));
    let ciphertext = cipher
        .encrypt(
            Nonce::<XChaCha20Poly1305>::from_slice(&nonce_bytes),
            plaintext,
        )
        .map_err(|e| VaultError::Encryption(e.to_string()))?;
    Ok((encode(&nonce_bytes), encode(&ciphertext)))
}

/// Decrypts a base64 nonce/ciphertext pair under `key`. Any authenticated
/// decryption failure (wrong key, tampered bytes) is an error, never data.
pub fn decrypt(
    key: &[u8; KEY_LEN],
    nonce_b64: &str,
    ciphertext_b64: &str,
) -> Result<Vec<u8>, VaultError> {
    let nonce = decode(nonce_b64)?;
    if nonce.len() != NONCE_LEN {
        return Err(VaultError::Decryption(format!(
            "nonce is {} bytes, expected {NONCE_LEN}",
            nonce.len()
        )));
    }
    let ciphertext = decode(ciphertext_b64)?;
    let cipher = XChaCha20Poly1305::new(Key::<XChaCha20Poly1305>::from_slice(key));
    let plaintext = cipher
        .decrypt(
            Nonce::<XChaCha20Poly1305>::from_slice(&nonce),
            ciphertext.as_ref(),
        )
        .map_err(|e| VaultError::Decryption(e.to_string()))?;
    Ok(plaintext)
}

/// Encrypts a snippet body into its on-disk `Protection` (fresh nonce, hint
/// fixed at this moment).
pub fn encrypt_body(key: &[u8; KEY_LEN], body: &str) -> Result<Protection, VaultError> {
    let hint = hint_for(body);
    let (nonce, ciphertext) = encrypt(key, body.as_bytes())?;
    Ok(Protection {
        hint,
        nonce,
        ciphertext,
    })
}

/// Decrypts a protected snippet body back to its UTF-8 plaintext.
pub fn decrypt_body(key: &[u8; KEY_LEN], protection: &Protection) -> Result<String, VaultError> {
    let bytes = decrypt(key, &protection.nonce, &protection.ciphertext)?;
    String::from_utf8(bytes).map_err(|e| VaultError::Encoding(e.to_string()))
}

/// The hint rule: after skipping leading whitespace, the body's first 5
/// characters when the significant portion is at least 12 characters long (so a
/// 7-char password doesn't give away most of itself), else the empty string.
pub fn hint_for(body: &str) -> String {
    let trimmed = body.trim_start();
    if trimmed.chars().count() >= HINT_MIN_BODY_CHARS {
        trimmed.chars().take(HINT_CHARS).collect()
    } else {
        String::new()
    }
}

/// The masked preview a protected card always shows: `hint + "••••••"` (bullets
/// only when the hint is empty). Never displays body content.
pub fn masked_preview(hint: &str) -> String {
    let mut out = String::with_capacity(hint.chars().count() + MASKED_SUFFIX.chars().count());
    out.push_str(hint);
    out.push_str(MASKED_SUFFIX);
    out
}

/// Creates a brand-new vault from a chosen password: fresh salt, derived key, and
/// the encrypted canary. Returns the `VaultMeta` to persist in `config.json` and
/// the key to keep in memory.
pub fn create_vault(password: &str) -> Result<(VaultMeta, [u8; KEY_LEN]), VaultError> {
    if password.len() < MIN_VAULT_PASSWORD_LEN {
        return Err(VaultError::WeakPassword(format!(
            "Password must be at least {MIN_VAULT_PASSWORD_LEN} characters"
        )));
    }
    let salt = random_bytes::<SALT_LEN>()?;
    let key = derive_key(password, &salt)?;
    let (nonce, canary) = encrypt(&key, CANARY_PLAINTEXT)?;
    Ok((
        VaultMeta {
            salt: encode(&salt),
            nonce,
            canary,
        },
        key,
    ))
}

/// Verifies a candidate password: derives a key with the stored salt and attempts
/// to decrypt the canary. Returns the derived key on success; any failure means
/// the password is wrong (or the metadata is corrupt) and yields an error.
pub fn verify_password(password: &str, meta: &VaultMeta) -> Result<[u8; KEY_LEN], VaultError> {
    let salt = decode(&meta.salt)?;
    let key = derive_key(password, &salt)?;
    let plaintext = decrypt(&key, &meta.nonce, &meta.canary)?;
    if plaintext.as_slice() != CANARY_PLAINTEXT {
        return Err(VaultError::Decryption(
            "canary decrypted to unexpected plaintext".to_string(),
        ));
    }
    Ok(key)
}

fn encode(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}

fn decode(s: &str) -> Result<Vec<u8>, VaultError> {
    BASE64
        .decode(s.trim())
        .map_err(|e| VaultError::Encoding(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_round_trips() {
        let key = [7u8; KEY_LEN];
        let (nonce, ciphertext) = encrypt(&key, b"hello vault").unwrap();
        assert_ne!(
            ciphertext,
            encode(b"hello vault"),
            "must not be plaintext in base64"
        );
        let plaintext = decrypt(&key, &nonce, &ciphertext).unwrap();
        assert_eq!(plaintext, b"hello vault");
    }

    #[test]
    fn every_encryption_uses_a_fresh_nonce() {
        let key = [9u8; KEY_LEN];
        let (n1, c1) = encrypt(&key, b"same").unwrap();
        let (n2, c2) = encrypt(&key, b"same").unwrap();
        assert_ne!(n1, n2, "nonce must be fresh per call");
        assert_ne!(c1, c2, "ciphertext must differ with a fresh nonce");
        assert_eq!(decrypt(&key, &n1, &c1).unwrap(), b"same");
        assert_eq!(decrypt(&key, &n2, &c2).unwrap(), b"same");
    }

    #[test]
    fn wrong_key_fails_cleanly() {
        let key = [1u8; KEY_LEN];
        let other = [2u8; KEY_LEN];
        let (nonce, ciphertext) = encrypt(&key, b"secret").unwrap();
        assert!(decrypt(&other, &nonce, &ciphertext).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = [3u8; KEY_LEN];
        let (nonce, ciphertext) = encrypt(&key, b"secret").unwrap();
        let mut bytes = decode(&ciphertext).unwrap();
        bytes[3] ^= 0xff;
        let tampered = encode(&bytes);
        assert!(decrypt(&key, &nonce, &tampered).is_err());
    }

    #[test]
    fn tampered_nonce_fails() {
        let key = [4u8; KEY_LEN];
        let (nonce, ciphertext) = encrypt(&key, b"secret").unwrap();
        let mut bytes = decode(&nonce).unwrap();
        bytes[0] ^= 0x01;
        let tampered = encode(&bytes);
        assert!(decrypt(&key, &tampered, &ciphertext).is_err());
    }

    #[test]
    fn encrypt_body_round_trips_and_canary_matches() {
        let (meta, key) = create_vault("hunter2x").unwrap();
        assert_eq!(verify_password("hunter2x", &meta).unwrap(), key);
        let protection = encrypt_body(&key, "a body long enough to have a hint").unwrap();
        assert_eq!(protection.hint, "a bod");
        assert_eq!(
            decrypt_body(&key, &protection).unwrap(),
            "a body long enough to have a hint"
        );
    }

    #[test]
    fn derive_key_is_deterministic_per_salt_and_differs_across_salts() {
        let k1 = derive_key("pw", &[0u8; SALT_LEN]).unwrap();
        let k1_again = derive_key("pw", &[0u8; SALT_LEN]).unwrap();
        let k2 = derive_key("pw", &[1u8; SALT_LEN]).unwrap();
        assert_eq!(k1, k1_again);
        assert_ne!(k1, k2);
        assert_ne!(
            derive_key("pw", &[0u8; SALT_LEN]).unwrap(),
            derive_key("other", &[0u8; SALT_LEN]).unwrap()
        );
    }

    #[test]
    fn canary_accepts_the_right_password_only() {
        let (meta, _) = create_vault("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &meta).is_ok());
        assert!(verify_password("wrong", &meta).is_err());
        assert!(verify_password("", &meta).is_err());
        // The stored meta must not contain the password or anything readable.
        let packed = serde_json::to_string(&meta).unwrap();
        assert!(!packed.contains("correct"));
    }

    #[test]
    fn create_vault_uses_a_fresh_salt_each_time() {
        let (m1, _) = create_vault("password1").unwrap();
        let (m2, _) = create_vault("password1").unwrap();
        assert_ne!(m1.salt, m2.salt);
        assert_ne!(m1.nonce, m2.nonce);
        assert_ne!(m1.canary, m2.canary);
        assert!(verify_password("password1", &m1).is_ok());
        assert!(verify_password("password1", &m2).is_ok());
    }

    #[test]
    fn create_vault_rejects_short_passwords() {
        assert!(
            matches!(create_vault(""), Err(VaultError::WeakPassword(_))),
            "empty password should be rejected"
        );
        assert!(
            matches!(create_vault("short"), Err(VaultError::WeakPassword(_))),
            "7-char password should be rejected"
        );
        assert!(
            create_vault("12345678").is_ok(),
            "8-char password should be accepted"
        );
        assert!(
            create_vault("password1").is_ok(),
            "long password should be accepted"
        );
    }

    #[test]
    fn hint_boundaries() {
        // 11 chars: no hint; 12 chars: hint appears.
        assert_eq!(hint_for("12345678901"), "");
        assert_eq!(hint_for("123456789012"), "12345");
        // Empty body.
        assert_eq!(hint_for(""), "");
        // Truncation is done by characters, not bytes.
        assert_eq!(hint_for("ééééééééééééé"), "ééééé");
        assert_eq!(hint_for("ééééééééééé"), "");
    }

    #[test]
    fn hint_for_skips_leading_whitespace() {
        // Leading whitespace is trimmed before counting and extracting.
        assert_eq!(hint_for("  123456789012"), "12345");
        assert_eq!(hint_for("  \t\n123456789012"), "12345");
        // 11 significant chars after trim → no hint.
        assert_eq!(hint_for("   12345678901"), "");
        // Body that is only whitespace.
        assert_eq!(hint_for("   "), "");
    }

    #[test]
    fn masked_preview_follows_the_hint_rule() {
        assert_eq!(
            masked_preview("ghp_x"),
            "ghp_x\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"
        );
        assert_eq!(
            masked_preview(""),
            "\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}\u{2022}"
        );
        assert!(
            !masked_preview("ghp_x").contains("ghp_x1"),
            "must never add body content"
        );
    }

    #[test]
    fn vault_state_starts_locked_and_wrong_password_keeps_it_locked() {
        let (meta, _) = create_vault("password1").unwrap();
        let mut state = VaultState::new();
        assert!(state.is_locked());
        assert!(!state.is_unlocked());
        assert!(state.key().is_none());

        assert!(state.unlock("nope", &meta).is_err());
        assert!(
            state.is_locked(),
            "wrong password must not change the state"
        );
        assert!(state.key().is_none());

        assert!(state.unlock("password1", &meta).is_ok());
        assert!(state.is_unlocked());
        assert_eq!(state.key().unwrap().len(), KEY_LEN);

        state.lock();
        assert!(state.is_locked());
        assert!(state.key().is_none());
    }

    #[test]
    fn vault_state_unlock_fails_on_a_corrupt_canary() {
        let mut meta = VaultMeta {
            salt: encode(&[5u8; SALT_LEN]),
            nonce: encode(&[6u8; NONCE_LEN]),
            canary: encode(b"garbage not ciphertext"),
        };
        let mut state = VaultState::new();
        assert!(state.unlock("pw", &meta).is_err());
        assert!(state.is_locked());

        meta.salt = "not base64!!".to_string();
        assert!(state.unlock("pw", &meta).is_err());
        assert!(state.is_locked());
    }

    #[test]
    fn vault_meta_round_trips_through_json() {
        let (meta, key) = create_vault("password1").unwrap();
        let json = serde_json::to_string(&meta).unwrap();
        let back: VaultMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(back.salt, meta.salt);
        assert_eq!(back.nonce, meta.nonce);
        assert_eq!(back.canary, meta.canary);
        assert_eq!(verify_password("password1", &back).unwrap(), key);
    }

    /// Cross-compatibility with the browser-extension native host.
    ///
    /// `test-vectors/vault-vector.json` was generated by the host's crypto; the
    /// desktop must derive the *identical* key and decrypt the host's canary and
    /// body, proving the two codebases share one vault contract (Argon2id
    /// m=19*1024 t=2 p=1 out=32, XChaCha20-Poly1305, base64 STANDARD, canary
    /// "copyit-vault-canary-v1"). That is what lets a snippet protected in the
    /// desktop be unlocked by the extension, and vice versa.
    #[test]
    fn desktop_vault_matches_native_host_test_vector() {
        let json = include_str!("../test-vectors/vault-vector.json");
        let v: serde_json::Value = serde_json::from_str(json).expect("valid vector json");
        let password = v["password"].as_str().unwrap();
        let salt_b64 = v["inputs"]["saltB64"].as_str().unwrap();
        let canary_nonce_b64 = v["inputs"]["canaryNonceB64"].as_str().unwrap();
        let canary_cipher_b64 = v["expected"]["canaryCiphertextB64"].as_str().unwrap();
        let body_nonce_b64 = v["inputs"]["nonceB64"].as_str().unwrap();
        let plaintext_body = v["inputs"]["plaintextBody"].as_str().unwrap();
        let body_cipher_b64 = v["expected"]["ciphertextB64"].as_str().unwrap();
        let key_hex = v["expected"]["keyHex"].as_str().unwrap();

        // The host's stored vault metadata: salt + canary (nonce + ciphertext).
        let meta = VaultMeta {
            salt: salt_b64.to_string(),
            nonce: canary_nonce_b64.to_string(),
            canary: canary_cipher_b64.to_string(),
        };
        // Desktop derives the same key and decrypts the host's canary iff its
        // Argon2id params + XChaCha20-Poly1305 + base64 match the host's.
        let key = verify_password(password, &meta)
            .expect("desktop must derive the host's key and decrypt its canary");
        let expected_key = hex::decode(key_hex).expect("valid keyHex");
        assert_eq!(
            key.to_vec(),
            expected_key,
            "derived key must match the host vector"
        );

        // Desktop can decrypt a body the host encrypted.
        let body = decrypt(&key, body_nonce_b64, body_cipher_b64)
            .expect("desktop must decrypt the host-encrypted body");
        assert_eq!(String::from_utf8(body).unwrap(), plaintext_body);
    }
}
