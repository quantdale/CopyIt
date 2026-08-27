# Design: harden-data-vault-lifecycle

## Context

The campaign starts from `main` at `7b8807e` (planning) and reconciles at `39fddd2`. Prior audits left `refuse_overwrite` dead (single bool, never checked beyond init), `save_config` errors discarded, `load_json` reading whole file before size check, `migrate_legacy` silent, protected clipboard indefinite, and vault KDF synchronous with dead `vault_work` receiver.

## Goals / Non-Goals

- Goals: close the data-loss overwrite path first, make every persistence failure visible and durable, bound reads before allocation, surface migration/dir failures, bound protected-clipboard lifetime with sequence guard, move KDF off frame thread with deterministic sim, and verify dependency posture.
- Non-Goals: new DB/storage format, cipher/KDF param change, password recovery, UI redesign, theme rework, broad egui 0.27→0.x migration, cloud sync.

## Decisions

### D1 — Per-file refuse flags, not a single global

`refuse_snippets_overwrite` and `refuse_config_overwrite` computed in `from_store` as `!backed_up` on `Load::Corrupt`. Each guards only its own `save_*` path via `refuse_overwrite_error()` and `push_save_error_once()`. Global flag would conflate the two files; per-file keeps the healthy file writable and tests deterministic without OS-permission tricks. Recovery banner is non-dismissable and per-file.

### D2 — Deduplicated banner for settings failures

`push_save_error_once` plus `clear_save_error(prefix)` on success ensures repeated frames don't grow duplicates while a later successful config save retires only its own prefix, not snippet/recovery notices. Theme/category keep data in-memory but report “will revert on restart” via the banner (truthful, pinned by tests). Vault metadata retains strict rollback (today's behavior).

### D3 — Bounded read with limit+1 + metadata preflight

`read_bounded(path, limit)` reads in 64 KiB chunks until `limit+1` bytes, returning `None` for oversize. `load_json_limited(path, limit)` preflights with `metadata().len()` for early exact-size error, then relies on `read_bounded` for correctness between metadata and read (no TOCTOU). Invalid UTF-8 surfaces as `Corrupt` via `String::from_utf8`. Testable with tiny injected limit; zero-byte/whitespace → `Missing` preserved.

### D4 — Explicit migration outcomes + data-dir init

`store.rs` gains `MigrationOutcome::NotNeeded|NoSource|Migrated|Blocked` and `LegacyMigration { snippets, config }`. `migrate_path_from` distinguishes NotNeeded (stable exists), NoSource (no readable non-empty candidate), Migrated (write+verify+rename), Blocked (read error on existing file or write/verify failure). `Store::open_initialized()` reports `ensure_data_dir()` failure; `new()` surfaces it and any Blocked outcomes in the banner before instance lock.

### D5 — Clipboard sequence guard, not plaintext compare

New `clipboard` module: `ClipboardBackend { set_text, clear, sequence }`. Windows uses `GetClipboardSequenceNumber()` + `OpenClipboard`/`EmptyClipboard` with retry; Sim backend uses `Rc<RefCell<SimState { text, seq, fail_writes }>>` plus shared `SimHandle { overwrite, set_fail_writes, text, sequence }`. `ProtectedCopyGuard { expires_at, sequence }` stores only the sequence token, never plaintext. `tick_clipboard(now)` and `lock_vault()` clear only when `sequence` still matches (user/other-app overwrite bumps the generation, so newer content is never clobbered). Failure is surfaced once, never fatal. In `cfg(test|feature=sim)` every copy routes through the backend for determinism; production unprotected copies keep `copied_text`.

### D6 — Single-job VaultWork with generation

`VaultWork { gen, kind: Unlock|Create, pending: Option<PendingVaultAction>, rx }` + `VaultWorkOutput { Unlocked([u8;32]), Created(VaultMeta, [u8;32]) }`. `spawn_vault_unlock/create` channel the result; password `String` is moved into the thread and `zeroize()`d after derive. `handle_vault_prompt` spawns at most one job (duplicate submit ignored while `vault_work.is_some()`), returns `Some(prompt)` (busy) immediately, requests repaint. `poll_vault_work` in `ui()` does `try_recv`, discards stale `gen` or canceled prompt (carrying stale result without unlocking/pending), handles persistence failure on Create (rollback meta+key, surface error, keep pending for retry), and closes the prompt on success. Busy disables Unlock/Create submit, spinner text “Working…”, Cancel restores `ProtectSave` editor draft from either prompt or work.pending.

## Risks / Trade-offs

- `windows-sys` added only on `cfg(target_os="windows")` (already in dep graph via winit); minimal unsafe surface (OpenClipboard/EmptyClipboard/SetClipboardData/GlobalAlloc path, single `unsafe` block with cleanup, retry on busy). Sim backend avoids real-clipboard use in tests/sim.
- Async KDF adds a thread per job in production; tests run immediate (spawn function checks `cfg!(test|feature=sim)`), preserving sim determinism and avoiding second-instance/thread flakes.
- Per-file refuse may surprise (visible fallback library labeled “Recovery mode”); alternative “read-only app” is heavier and deferred as non-Critical.

## Migration Plan

- No storage format change; JSON backward-compat preserved.
- Corrupt files already renamed to `.corrupt` continue loading as corrupt; new Blocked state adds refuse + banner paths.
- Existing vaults load unchanged; async path uses same `derive_key`/`verify_password`.

## Open Questions

- None — all workstreams have concrete acceptance criteria and regression tests (14 required scenarios).
