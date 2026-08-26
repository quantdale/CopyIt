# Design: harden-data-vault-lifecycle

## Context

`CopyIt::from_store` already distinguishes `Loaded`/`Missing`/`Corrupt` and moves corrupt files aside with `backup_corrupt`. The P0 gap is that `refuse_overwrite` was dead: later saves ignored it. The working tree (pre-campaign) introduced per-file refusal flags; this campaign completes them into a real recovery state.

## Goals / Non-Goals

- Goals: never overwrite an un-backed-up corrupt original; surface every persistence failure; bound memory during load; make migration failures explicit; bound protected-clipboard lifetime; move KDF off the frame thread.
- Non-Goals: no egui upgrade, no new storage format, no password recovery, no cloud/network features, no UI redesign.

## Decisions

### D1: Per-file refusal flags, not a mode enum

Two independent bools (`refuse_snippets_overwrite`, `refuse_config_overwrite`) set only in `from_store` when `backup_corrupt` fails. They are process-lifetime; no code path clears them. `save_snippets`/`save_config` check the matching flag first and return a distinct error without touching disk. Rationale: the two files fail independently; an enum would imply global read-only semantics we don't want (the healthy file stays writable).

### D2: Recovery UI is a distinct banner, not a dismissible warning

When either flag is set, `ui` renders a dedicated recovery banner naming the file, explaining the visible data is a fallback (for snippets) or unsaved (for config), and stating that changes are not persisted. It is not dismissable for the session. Destructive library mutations are still allowed in-memory but each save attempt re-surfaces the refusal error; the banner prevents false confidence. For blocked config, theme/category changes apply in memory but the banner states they will revert on restart.

### D3: Config-save errors are durable and deduplicated

`save_config` failures push a message prefixed with `CONFIG_SAVE_ERROR`; a helper `push_save_error_once` dedupes by exact text so per-frame retries don't grow the banner. A later successful config save retires only messages starting with `CONFIG_SAVE_ERROR` (existing behavior), never recovery notices. Theme/category flows stop using `let _ =`.

### D4: Bounded loader via limit+1 read

`load_json` reads through a small helper `read_bounded(path, limit) -> io::Result<ReadBounded>` that copies at most `limit + 1` bytes from a `File::read` loop into a `Vec` capped at that size, then reports `TooLarge` if the stream wasn't exhausted. No allocation sized by untrusted metadata; metadata preflight is skipped entirely (TOCTOU-proof by construction). The helper takes the limit as a parameter so tests use tiny limits. Whitespace-only → `Missing`, invalid UTF-8/JSON → `Corrupt`, zero-byte → `Missing`: all preserved.

### D5: Migration returns an outcome

`migrate_legacy()` returns `MigrationOutcome { NoSourceNeeded, Migrated, Blocked { source, reason }, NotNeeded }` per file (aggregated). `from_store` maps `Blocked` to a save_error notice naming the source path and reason. The stable destination is still absent in that case (nothing can be destroyed there), so seeding proceeds — but the banner carries the migration-failure notice so the state is never mistaken for a clean first launch. Directory-creation failure surfaces as a startup error listing the target path.

After a successful copy, verify the stable copy loads (`storage::load` returns `Loaded`) before renaming the source to `.migrated`.

### D6: Clipboard module with generation-safe expiry

New `src/clipboard.rs` (cfg'd plain in production): `ClipboardBackend` trait with `set_text`, `read_text`, `clear_text`; Windows backend implemented via `clipboard-win` crate (small, maintained, pure API wrapper over user32); tests/sim use a fake backend. App state: `protected_clipboard: Option<ProtectedCopy { expires_at: f64 }>` storing **only the deadline** — never the plaintext. On expiry (checked each frame via `request_repaint_after`), the app calls `read_text` and clears only when the current clipboard content equals what CopyIt last wrote... which would require retaining plaintext. Instead: the Windows backend provides `owns_sequence(u64)` using `GetClipboardSequenceNumber` captured at copy time — compare sequence number at expiry; unchanged sequence ⇒ clipboard still holds our copy ⇒ clear. Fake backend mirrors the same protocol with a bump-on-write counter. Vault lock triggers the same guarded clear immediately. Backend failures are surfaced once via the banner and never panic.

Chosen window: 30 seconds (documented constant).

### D7: Vault KDF worker state machine

`VaultWork { Idle, Running { job: JobKind }, ResultReady { job, outcome } }` plus a boxed executor trait `KdfExecutor` (production: spawns `std::thread`, sends result over `mpsc`; test/sim: immediate or manually-completed executor). One job at a time; submit clicks while `Running` are rejected; closing/cancelling the prompt marks the job stale so a late result is dropped without running pending actions; password bytes are moved into the worker and zeroized after derivation (worker owns them). Polling happens in `ui` each frame; a repaint is requested when work completes. `vault_work` receiver field is replaced by the new machine.

## Risks / Trade-offs

- [clipboard-win dependency] → smallest maintained crate for sequence-number access; audit re-run covers it.
- [Thread-based KDF vs determinism] → executor seam keeps sim deterministic (immediate executor).
- [Refusal flags can frustrate] → banner explains exact remediation (copy the corrupt file elsewhere, restart).

## Migration Plan

No data-format changes. Older configs/files load unchanged. Flags start false everywhere except genuine backup failures.

## Open Questions

None.
