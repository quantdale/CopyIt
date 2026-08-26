# Tasks: harden-data-vault-lifecycle

## 1. P0: per-file write protection after corruption-backup failure
- [x] 1.1 Replace dead `refuse_overwrite` with per-file `refuse_snippets_overwrite` / `refuse_config_overwrite`, set only when the matching backup fails
- [x] 1.2 Guard `save_snippets` and `save_config`; refusal returns a distinct error and leaves disk untouched
- [ ] 1.3 Dedicated non-dismissible recovery banner naming each blocked file; states fallback-library truth for snippets and revert-on-restart for config
- [ ] 1.4 Tests: blocked snippets + add/edit/delete/reorder → original bytes unchanged; blocked config + theme/category/vault op → original bytes unchanged; both blocked simultaneously without cross-clearing

## 2. Non-silent settings persistence
- [ ] 2.1 Remove `let _ = self.save_config()` from category/theme flows; route failures through deduplicated save_error push
- [ ] 2.2 A successful config save retires only its own error class (existing behavior, pinned by tests)
- [ ] 2.3 Tests: theme-save failure shows durable warning; category-save failure consistent; no duplicate banner growth across frames

## 3. Bounded JSON loading
- [ ] 3.1 Add `read_bounded(path, limit)` limit+1 reader in storage.rs; wire into `load_json`
- [ ] 3.2 Preserve Missing/zero-byte/whitespace/UTF-8/serde semantics
- [ ] 3.3 Tests with small injected limit: exact-limit loads, limit+1 rejected, round-trip

## 4. Explicit migration / directory-init outcomes
- [ ] 4.1 `migrate_legacy` returns per-file outcomes (NoSourceNeeded/Migrated/Blocked/NotNeeded); verify stable copy before `.migrated` rename
- [ ] 4.2 Surface Blocked outcome + source path in startup warnings; surface stable-directory creation failure
- [ ] 4.3 Tests: legacy exists + destination write fails → source untouched, failure reported; success path renames source

## 5. Protected clipboard bounded lifetime
- [ ] 5.1 New `src/clipboard.rs`: backend trait, Windows backend (sequence-number guard), fake backend for tests/sim
- [ ] 5.2 Protected copy records deadline (30 s) + clipboard sequence; expiry clears only if sequence unchanged; lock clears early
- [ ] 5.3 Unprotected copy unchanged; backend failure surfaced once, never fatal
- [ ] 5.4 Tests: expiry clear; user overwrite before expiry left alone; lock early-clear; unprotected no timeout; failure non-fatal
- [ ] 5.5 Docs: README/AGENTS best-effort clearing note incl. clipboard-history caveat

## 6. Off-thread vault KDF
- [ ] 6.1 `VaultWork` state machine + `KdfExecutor` seam (thread executor in prod; immediate/manual in tests)
- [ ] 6.2 Busy state in vault prompt modal; duplicate submits rejected; cancel/stale results dropped without pending actions
- [ ] 6.3 Password zeroized in worker after derivation; vault stays locked until verified result accepted
- [ ] 6.4 Remove dead `vault_work` field
- [ ] 6.5 Tests: success, wrong password, corrupt metadata, duplicate submit, stale/cancel, create persistence-failure rollback, pending resumption

## 7. Validation & audit
- [ ] 7.1 Full gates green: fmt, clippy -D warnings, test --all-targets, sim journeys serial, release build, openspec validate --strict
- [ ] 7.2 Re-run dependency audit posture; update docs/dependency-audit-notes.md if graph changed
- [ ] 7.3 Cross-cutting regression review of affected interaction graph; simulation determinism intact; no plaintext in reports/bundles
