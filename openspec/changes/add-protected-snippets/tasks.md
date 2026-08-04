# Tasks: add-protected-snippets

## 1. Dependencies and vault core

- [x] 1.1 Add `argon2`, `chacha20poly1305`, `base64`, and `rand` to `Cargo.toml` (pure-Rust crates only; no system libs)
- [x] 1.2 Create `src/vault.rs`: Argon2id key derivation (m = 19 MiB, t = 2, p = 1, 16-byte random salt), XChaCha20-Poly1305 encrypt/decrypt with fresh 24-byte nonce per call, base64 helpers, and a canary create/verify pair — all pure functions returning `Result`, no `unwrap`
- [x] 1.3 Unit-test `vault.rs`: encrypt/decrypt round-trip, wrong key fails cleanly, tampered ciphertext fails, canary accepts the right password and rejects a wrong one
- [x] 1.4 Add `VaultState` (`Locked` / `Unlocked(key)`) to `vault.rs` with unlock/lock methods; test state transitions including wrong-password unlock keeping the Locked state

## 2. Data model and persistence

- [x] 2.1 Add `protection: Option<Protection>` (`hint`, `nonce`, `ciphertext`) to `Snippet` in `src/model.rs` with `#[serde(default)]`; add `vault: Option<VaultMeta>` (`salt`, `nonce`, `canary`) to `Config` in `src/storage.rs` with `#[serde(default)]`
- [x] 2.2 Update all `Snippet` construction sites (`seed.rs`, `apply_save` in `app.rs`, test helpers) for the new field
- [x] 2.3 Add hint derivation (first 5 chars when body length ≥ 12, else empty) to `vault.rs` and test the boundary cases (11/12 chars, empty body)
- [x] 2.4 Test that old `snippets.json`/`config.json` files (no `protection`/`vault` fields) load unchanged, and that a protected snippet round-trips through `Store::at(temp)` save/load with an empty `body` on disk

## 3. Censored rendering and search

- [x] 3.1 Update `Derived::new` in `src/app.rs`: protected snippets get empty `body_lower` and a masked preview (`hint + "••••••"` or bullets only); add a lock indicator to `card()`
- [x] 3.2 Test that search never matches protected bodies but still matches their title/category, and that the masked preview follows the hint rule (hint vs. no-hint cards)
- [x] 3.3 Test that protected cards render masked even while the vault is unlocked

## 4. Gating copy/edit/delete and the unlock modal

- [x] 4.1 Add `vault: VaultState` to `CopyIt`; gate `Action::Copy`/`Action::Edit` dispatch in `update()`: locked + protected → open the unlock modal with the pending action instead of copying/editing
- [x] 4.2 Build the unlock modal (password field, inline wrong-password error, cancel) plus the set-vault-password variant (password + confirmation) used when no vault exists; wire successful unlock to proceed with the pending action
- [x] 4.3 On unlocked copy, decrypt the body to the clipboard; on unlocked edit, open the editor with the decrypted body; delete stays gated behind the editor
- [x] 4.4 Test the gating matrix in `app.rs` tests (using `test_app()`): locked copy/edit prompts and does not copy/open; unlocked copy places plaintext on the (captured) clipboard; unlocked edit opens with decrypted content; wrong password changes nothing; AEAD failure with valid canary shows the warning banner and leaves the file untouched

## 5. Editor protect/unprotect and top-bar lock control

- [x] 5.1 Add a "Protect this snippet" checkbox to the editor (state in `Editor`, enabled only while unlocked); extend `apply_save` to encrypt (fresh nonce, empty plaintext body) when checked and store plaintext when unchecked
- [x] 5.2 Trigger vault password creation on the first-ever protect (before save); refuse protecting while locked with the unlock prompt
- [x] 5.3 Add the vault state indicator and Lock button to the top bar (visible only when protected snippets exist)
- [x] 5.4 Test protect → card renders censored and file contains ciphertext; unprotect → plaintext restored; first protect creates the `vault` section in `config.json`; Lock button re-locks and drops the key

## 6. Verification and docs

- [x] 6.1 Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` (close any running CopyIt first — the exe is locked while running)
- [x] 6.2 Manual smoke pass: protect a card, restart the app, unlock, copy, verify clipboard; wrong-password path; lock/unlock cycle; old library untouched
  - Verified via the automated simulation suite (vault journeys: unlock+copy, wrong-password, censored-while-locked, lock/unlock cycle — each starts from a restarted-app state, i.e. a fixture with an existing vault, and asserts clipboard + on-disk state) plus one headed run (`cargo run --features sim -- --simulate vault-happy-unlock-copy`) against the real window with screenshot capture. No real `%APPDATA%\CopyIt` data involved.
- [x] 6.3 Update `README.md` and `AGENTS.md`: protected cards are encrypted at rest (XChaCha20-Poly1305/Argon2id), the "do not store credentials" warning is scoped to unprotected cards, no password recovery, hint/metadata-leak notes, downgrade caveat
- [x] 6.4 Run `openspec validate add-protected-snippets --strict`
