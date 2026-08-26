# Proposal: harden-data-vault-lifecycle

## Why

A fresh audit of current `main` found a reachable data-loss path that contradicts the documented invariant "an unreadable file whose backup fails must never be overwritten": after `backup_corrupt` fails at startup, ordinary actions (add/edit/delete/reorder, theme/category changes) still call the normal save paths and replace the only remaining copy of the user's original bytes. Adjacent gaps compound the risk: config-save failures are silently discarded (`let _ = self.save_config()`), the 256 MiB load cap is checked only after the whole file has been read into memory, failed legacy migrations masquerade as clean first launches, protected clipboard plaintext lives forever, and Argon2id key derivation blocks the UI thread.

## What Changes

- **Per-file write protection after unrecoverable corruption-backup failure.** Separate `refuse_snippets_overwrite` / `refuse_config_overwrite` states; every persistence path for that file refuses to write while recovery state is shown in the UI.
- **Non-silent settings persistence.** Theme and category changes surface config-write failures as durable, deduplicated warnings instead of discarding them.
- **Bounded JSON loading.** `MAX_DATA_FILE_BYTES` becomes a real bound enforced during the read (limit+1 detection), not after a full slurp.
- **Explicit migration outcomes.** Startup distinguishes no-source / migrated / blocked / unnecessary; a known failed migration is never presented as first launch, and stable-directory initialization failures are surfaced.
- **Bounded protected-clipboard lifetime.** Copying a protected snippet schedules a best-effort clear (~30 s) that only fires if the clipboard still holds CopyIt's own copy; vault lock clears it early; unprotected copies are unchanged.
- **Off-thread vault KDF.** Create/unlock key derivation runs behind a bounded worker state machine (one job at a time, busy state, stale-result rejection) with a deterministic executor for tests/simulation.

## Capabilities

### New Capabilities

- `data-recovery`: Per-file protection of corrupt originals whose backup rename failed, including truthful recovery UI and mutation guarding.
- `settings-persistence`: First-class surfacing of config-save failures for theme/category changes, including deduplication and restart semantics.
- `bounded-storage-load`: Size-bounded reads of JSON data files before allocation/parsing.
- `legacy-migration`: Explicit outcomes for one-time legacy migration and stable-directory initialization failures.
- `protected-clipboard`: Time-bounded best-effort clearing of protected-copy plaintext without clobbering newer clipboard content.

### Modified Capabilities

- `snippet-vault`: Vault KDF execution moves off the UI thread behind a bounded, cancellable work state machine with deterministic testing seams.

## Impact

- **Code**: `src/app.rs` (recovery state, save guards, warning dedup, clipboard scheduling, vault work state machine), `src/storage.rs` (bounded loader), `src/store.rs` (migration outcomes, dir init), new `src/clipboard.rs` (clipboard abstraction), `src/vault.rs` (unchanged crypto).
- **Docs**: README/AGENTS security notes on best-effort clipboard clearing.
- **Compatibility**: JSON formats unchanged; older files keep loading; simulation determinism preserved via executor seam.
