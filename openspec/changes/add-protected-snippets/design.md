# Design: add-protected-snippets

## Context

CopyIt stores every snippet as plaintext JSON in `%APPDATA%\CopyIt\snippets.json` (see `src/model.rs`, `src/storage.rs`). The documented security stance today is "no encrypted storage — do not store sensitive credentials in snippets". Users nevertheless want API keys and passwords as copyable tiles, so the stance must change from "forbidden" to "safe by construction".

Relevant seams in the current code (they are what make this change small):

- **Single copy/edit funnel**: card clicks become `Action::Copy(id)` / `Action::Edit(id)` and are dispatched in one place in `CopyIt::update` (`src/app.rs`). Gating copy/edit means gating that one match block.
- **Single save funnel**: every mutation goes through `CopyIt::snippets_changed()` → `store.save_snippets`. Encryption-on-save hooks into `apply_save` only.
- **Derived cache**: `Derived` (`src/app.rs`) holds the searchable lowercase text and the card preview per snippet. It is the one place that must never see protected plaintext.
- **Policy-out-of-UI pattern**: `src/editor.rs` keeps editor transitions pure/testable; `src/store.rs` keeps persistence behind an interface. The vault follows the same pattern as a new pure module, `src/vault.rs`.

## Goals / Non-Goals

**Goals:**

- A snippet marked protected has its body stored **only as ciphertext** on disk.
- One vault password gates copy and edit of all protected snippets; one entry unlocks the session.
- Locked cards never render, index, or leak body content beyond a ≤5-character hint.
- Wrong passwords and damaged ciphertext fail gracefully (inline error / warning banner), never panic, never overwrite data.
- Old `snippets.json`/`config.json` files keep loading without a migration step.

**Non-Goals:**

- Per-card passwords (one vault password covers all protected cards).
- Auto-lock on a timer; clipboard auto-clear after copying a secret.
- Encrypting titles, categories, or the hint — metadata stays cleartext.
- OS keychain/credential-manager integration, secure-enclave key storage.
- Memory-hardening beyond what the crypto crates do internally (editor/copy buffers are ordinary `String`s; this is a local GUI, not a multi-tenant service).
- Password recovery. A forgotten vault password means the protected bodies are unrecoverable, by design.

## Decisions

### 1. Real encryption, not UI masking

Protected bodies are encrypted at rest. The alternative — keeping plaintext in `snippets.json` and merely refusing to show/copy it without a password — was rejected: the data file is hand-editable JSON in a well-known folder, so masking would be a lock anyone with a text editor walks around, while *looking* like protection. The feature's purpose (API keys, passwords) demands at-rest confidentiality.

### 2. One vault password for all protected cards

A single app-wide password, set the first time the user protects a card. Alternatives considered: per-card passwords (N passwords to remember, N prompts, no real gain for a single-user desktop app) and no-password "master unlock" (pointless). The vault key is derived once per unlock and kept in memory for the session.

The password is verified through a **canary**: at password-set time the app encrypts a fixed known string with the vault key and stores it (with its nonce and the KDF salt) in `config.json` under `vault`. A candidate password is correct iff the canary decrypts and authenticates. No password hash, no plaintext, is ever stored.

### 3. Argon2id + XChaCha20-Poly1305, via RustCrypto crates

- KDF: **Argon2id** with RFC 9106's moderate parameters (m = 19 MiB, t = 2, p = 1) — ≈60 ms once per unlock on the UI thread; imperceptible next to a click, no async needed. Salt: 16 random bytes, stored in `config.json`.
- AEAD: **XChaCha20-Poly1305** — 24-byte random nonce per encryption makes nonce reuse a non-issue, and authentication means wrong passwords/tampering surface as a clean decrypt error rather than garbage plaintext.
- Encoding: base64 for salt/nonce/ciphertext in JSON.
- New deps: `argon2`, `chacha20poly1305`, `base64` (+ `rand` for salt/nonce generation). All pure-Rust: consistent with the single-exe, no-runtime-deps distribution model, and each is small under the size-tuned release profile.

### 4. Data model: `Snippet.protection: Option<Protection>`

```text
Snippet { id, title, category, body, protection }
Protection { hint, nonce, ciphertext }        // base64 for the byte fields
Config   { categories, theme, vault }         // vault: Option<{ salt, nonce, canary }>
```

- When `protection` is `Some`, `body` is **empty** on disk; the secret lives only in `ciphertext`.
- `#[serde(default)]` on all new fields: old files load with `protection: None` / `vault: None` — no migration, no format version bump.
- Each save of a protected snippet re-encrypts with a **fresh nonce**.
- **Hint rule**: `hint` holds the body's first 5 chars when the body is ≥12 chars long, otherwise the empty string (for a 7-char password, 5 chars would be most of the secret). Fixed at protect/edit time; never recomputed from ciphertext.

### 5. Plaintext only ever exists on demand

- `Derived::new` treats a protected body as empty: `body_lower = ""`, and the cached preview is the masked string (`hint + "••••••"` or just bullets). Search therefore never matches secrets and the grid never renders them — **even while unlocked**. Unlocking skips password prompts; it does not change what cards display.
- Decryption happens at two moments only: copying a protected card (plaintext goes straight to the clipboard string) and opening the editor on one (plaintext lives in the editor buffer, as it does for any snippet).

### 6. Unlock UX: modal prompt, session scope, manual lock

- `CopyIt` gains `vault: VaultState` (Locked / Unlocked(key)) — memory only.
- Copy/edit/delete of a protected card while Locked opens an **unlock modal** (password field, optional "set new vault password" variant when none exists yet). Success: key cached, pending action proceeds. Failure: inline "wrong password" error, state unchanged, retry allowed.
- The top bar shows a lock indicator; while unlocked it offers a **Lock** button that drops the key from memory.
- Delete is implicitly gated: it lives behind the editor, which requires unlock.
- Editing a protected card saves it back encrypted (with a fresh nonce) as long as it remains protected; unchecking "Protect this snippet" stores the body as plaintext again. The editor's protect checkbox is only enabled while the vault is unlocked.

### 7. Failure handling mirrors the existing banner pattern

- Wrong password → inline modal error; nothing else happens.
- Canary decrypts but a card body doesn't (damaged/tampered ciphertext) → the copy/edit is refused and the existing `save_error` warning banner reports the card; the file is left untouched. JSON-level corruption keeps the existing `Load::Corrupt` → `.corrupt` backup path; this design adds no new overwrite risk.
- KDF/AEAD failures are treated as `io`-style errors and surfaced, never `unwrap`ed.

## Risks / Trade-offs

- **Forgotten vault password → permanent data loss** (no recovery by design) → Documented prominently in README + shown when setting the password; the canary makes "did I type it right?" deterministic so typos at creation are caught via a confirm field.
- **Hint leaks a ≤5-char prefix** (e.g. `ghp_x`) → Deliberate: it's what distinguishes cards without unlocking. Bounded by the 12-char rule; documented in the spec and README.
- **Metadata stays cleartext** (titles like "Prod AWS key", categories) → Documented: users must keep secrets out of titles; encrypting metadata would break search/sort for negligible gain on a local file.
- **New dependencies grow the binary** → Three small pure-Rust crates under `opt-level = "z"`/`lto`; accepted cost for real encryption. No system libraries, so the portable-exe story is unchanged.
- **KDF blocks the UI thread ~60 ms per unlock** → Once per session, behind a modal; acceptable. Raising parameters later is a one-line change.
- **Downgrade path**: an older CopyIt reading a new file sees protected cards with empty bodies → Acceptable, additive-field JSON; documented in README ("don't downgrade after protecting cards"). Rollback of the feature itself = restore the pre-change data backup.
- **Session-long unlock widens exposure while the app runs** → Mitigated by the manual Lock button; auto-lock timer deliberately deferred to a follow-up change.

## Migration Plan

1. Ship additive-only: new optional fields, new module, new modal. No changes to existing files' schema beyond optional fields.
2. On first protect, the app creates `config.json`'s `vault` section via the normal atomic save path.
3. Existing snippets are untouched until a user explicitly protects them.
4. Rollback: unprotecting every card returns the file to the old shape; code rollback only requires deleting or hand-editing protected cards first (their bodies would otherwise be unreadable by the old build).

## Open Questions

- Auto-lock after N minutes / on window blur — follow-up change if users ask.
- "Reveal on card" toggle while unlocked (show full body on the card) — deferred; current design keeps cards always masked.
