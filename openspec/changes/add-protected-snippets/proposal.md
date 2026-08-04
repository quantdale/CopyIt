# Proposal: add-protected-snippets

## Why

CopyIt users want to keep API keys, passwords, and other secrets as copyable tiles, but today every snippet body is stored as plaintext in `%APPDATA%\CopyIt\snippets.json` — and the project's own docs warn users *not* to store credentials in snippets. A password-gated, encrypted card type turns CopyIt into a safe place for sensitive snippets instead of a place that forbids them.

## What Changes

- **New "protected" snippet kind.** A snippet can be marked protected (a checkbox in the editor). A protected snippet's body is stored **encrypted** in `snippets.json` (XChaCha20-Poly1305, key derived from a password with Argon2id) — never as plaintext.
- **Vault password.** One app-wide password protects all protected snippets. It is set the first time the user protects a card, stored nowhere in cleartext, and verified via an encrypted canary in `config.json`. One successful entry unlocks the vault for the rest of the session.
- **Censored cards.** A locked protected card never renders its body: the card preview shows at most a 5-character cleartext hint plus bullets (e.g. `ghp_x••••••••`), and search never matches protected bodies. Unlocking skips future password prompts but cards stay masked in the grid — bodies are decrypted only on demand for a copy or edit.
- **Password gate on copy and edit.** Copying or editing a protected card while the vault is locked opens an unlock prompt; wrong passwords are rejected. Deleting is implicitly gated because delete lives inside the editor.
- **Lock control.** The top bar shows the vault state and offers a manual "Lock" while unlocked. Unlock state lives in memory only; nothing sensitive is persisted beyond the ciphertext, nonce, hint, and KDF salt.
- **Docs.** The security stance in `README.md`/`AGENTS.md` flips: protected cards are encrypted at rest; the "do not store credentials" warning is scoped to unprotected cards.

## Capabilities

### New Capabilities

- `snippet-vault`: Password-based vault for protected snippets — key derivation, password verification, session lock/unlock state, and the encrypt/decrypt operations backing protected cards.
- `protected-cards`: The user-facing protected card behavior — censored card rendering and search exclusion, password-gated copy/edit/delete, protecting/unprotecting in the editor, and the encrypted on-disk representation.

### Modified Capabilities

<!-- No specs exist under openspec/specs/ yet; there is nothing to modify. -->

## Impact

- **Data model** (`src/model.rs`): `Snippet` gains an optional `protection` field (`hint`, `nonce`, `ciphertext`); old JSON files load unchanged via serde defaults (backward compatible). When protected, the plaintext `body` field is empty on disk.
- **New module** (`src/vault.rs`): pure crypto + vault state (Argon2id KDF, XChaCha20-Poly1305 AEAD, canary check, encrypt/decrypt), testable without a UI.
- **UI** (`src/app.rs`): unlock prompt modal, censored card preview, vault indicator + Lock button in the top bar, gated `Action::Copy`/`Action::Edit` dispatch.
- **Editor** (`src/editor.rs`): "Protect this snippet" checkbox and protection-aware save.
- **Config** (`config.json` via `src/storage.rs`): optional `vault` section (KDF salt + encrypted canary), serde-defaulted for old configs.
- **Dependencies** (`Cargo.toml`): `argon2`, `chacha20poly1305`, `base64` — all pure-Rust, no system libraries, consistent with the single-exe/no-runtime-deps distribution model.
- **Docs**: `README.md`, `AGENTS.md` security sections; `docs/` notes if needed.
