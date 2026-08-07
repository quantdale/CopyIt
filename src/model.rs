use serde::{Deserialize, Serialize};

/// A single stored item: a script or a prompt.
/// The `id` field is stable across edits/saves (never reassigned) and is used as a unique key
/// for the Editor modal and drag-and-drop operations. Snippets are serialized to JSON in
/// insertion order (they remain in self.snippets in whatever order the user drags them to).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Snippet {
    pub id: u64,          // Stable unique identifier; unique among currently stored snippets
    pub title: String,    // Display name of the snippet
    pub category: String, // User-defined category (normalized to title-case)
    pub body: String,     // The actual content to copy to clipboard
    /// When `Some`, the snippet is password-protected: `body` is empty on disk and the
    /// real text lives only as ciphertext under the vault key (see `crate::vault`).
    /// `#[serde(default)]` keeps files written by older versions loading unchanged.
    #[serde(default)]
    pub protection: Option<Protection>,
}

/// The on-disk representation of a protected snippet body: the hint (at most 5 cleartext
/// chars, fixed at protect time), plus the base64 nonce and ciphertext of the
/// XChaCha20-Poly1305 encryption of the body under the vault key. The plaintext never
/// lives here.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Protection {
    /// First 5 characters of the body at protect time when it was >= 12 chars long,
    /// else empty. Never recomputed from the ciphertext.
    pub hint: String,
    /// Base64-encoded 24-byte XChaCha20 nonce.
    pub nonce: String,
    /// Base64-encoded ciphertext.
    pub ciphertext: String,
}
