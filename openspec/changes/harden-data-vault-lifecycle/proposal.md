# Proposal: harden-data-vault-lifecycle

## Why

A fresh audit of current `main` finds several reachable gaps that violate documented invariants: a corrupt `snippets.json`/`config.json` whose backup rename fails can still be overwritten by later add/edit/reorder/theme/category/vault actions; config-save failures are silently discarded; the 256 MiB load cap is checked only after the whole file is read; failed legacy migrations masquerade as clean first launches; protected-clipboard plaintext lives unbounded; and Argon2id KDF blocks the UI thread behind dead `vault_work` scaffolding.

## What Changes

- **Per-file write protection** after unrecoverable corruption-backup failure (separate refuse flags, save guards, recovery banner, no cross-clear).
- **First-class settings-persistence failures** (deduplicated, durable warnings; truthful in-memory state; vault-metadata rollback preserved).
- **Bounded storage load** (`read_bounded` limit+1, metadata preflight as optimization only, no untrusted allocation, testable injected limit).
- **Explicit legacy-migration and data-dir init states** (`MigrationOutcome`/`LegacyMigration`, verified copy, untouched source on Blocked, surfaced banner).
- **Protected clipboard bounded lifetime** (`clipboard` module, sequence-token guard, 30 s window, lock-clears-early, fake backend, best-effort failure handling).
- **Async vault KDF** (single-job `VaultWork` state machine, busy modal, stale/cancel discard, zeroized passwords, immediate executor in tests/sim, poll via `request_repaint`).
- **Docs/audit**: README security note on best-effort clipboard, dependency posture verified.

## Capabilities

### New Capabilities

- `data-recovery`: Per-file recovery/write-block state for corrupt originals whose backup failed.
- `clipboard`: Time-bounded, sequence-guarded clearing of protected copies.
- `vault-async`: Off-thread, bounded, cancellable vault KDF execution.
- `storage-bounded-load`: Size-bounded reads (partial, not in a user-visible spec but covered here).
- `legacy-migration`: Explicit migration outcomes (covered under data-recovery where visible).

### Modified Capabilities

- `snippet-vault`: Unlock/create now async via VaultWork; busy state, stale-result discard, zeroize.
- `protected-cards`: Copy now uses clipboard backend for lifetime semantics (behavioral, not data-model).

## Impact

- **Storage** (`storage.rs`, `store.rs`): Bounded reads, explicit migration outcomes, data-dir init surfacing.
- **App** (`app.rs`): Recovery banner, save guards, settings-persistence dedupe, clipboard scheduling (sequence-guarded), vault async state-machine, poll in `ui()`, busy modal.
- **Clipboard** (`clipboard.rs` new): trait + Windows backend (GetClipboardSequenceNumber) + Sim fake (shared state + sequence). Small `windows-sys` dep on Windows only.
- **Vault** (`vault.rs`): No cipher/KDF param change; only `unlock` path now also via async wrapper (password zeroized after derive).
- **Sim** (`harness.rs`, `journey.rs`): Deterministic fake clipboard injection, clipboard expectations via SimHandle.
- **Tests**: 14 required regression scenarios added (see tasks).
- **Docs**: README/AGENTS security note on best-effort clipboard.
- **Compat**: JSON files remain hand-editable, older files load, simulation determinism preserved, single-exe architecture retained.
