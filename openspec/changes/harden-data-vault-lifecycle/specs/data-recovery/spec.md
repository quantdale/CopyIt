# Spec: data-recovery

Per-file recovery/write-block state for corrupt originals whose backup failed, with bounded loads, explicit migration outcomes, and surfaced init failures.

## ADDED Requirements

### Requirement: Per-file write protection after unrecoverable backup failure

The system SHALL refuse every later write that would overwrite a corrupt original file (`snippets.json` or `config.json`) when that file could not be backed up at startup, for the rest of the process. The protection SHALL be per-file and SHALL cover every persistence path (initial seeding/canonicalization, add/edit/delete/reorder, category creation, theme selection, vault metadata create/update, and any future save helper). A corrupt file that was successfully renamed aside SHALL remain writable via normal replacement/seeding. The protection SHALL be lifted only by a deliberately verified recovery/reload or by restart after the underlying problem is corrected; the system SHALL never infer safety from a failed write.

#### Scenario: Corrupt snippets file survives every later library mutation

- **WHEN** `snippets.json` was corrupt and its backup rename failed at startup
- **THEN** a subsequent `add`, `edit`, `delete`, or `reorder` is applied in-memory but `save_snippets` refuses to write and the original corrupt file remains byte-for-byte unchanged on disk

#### Scenario: Corrupt config file survives settings changes

- **WHEN** `config.json` was corrupt and its backup rename failed at startup
- **THEN** a subsequent theme/category/vault operation is applied in-memory but `save_config` refuses to write and the original corrupt file remains byte-for-byte unchanged on disk

#### Scenario: Both files blocked independently

- **WHEN** both `snippets.json` and `config.json` are blocked simultaneously
- **THEN** a successful save of one file never clears the other file's recovery state and neither file's original bytes are overwritten

### Requirement: Truthful recovery UI and no silent success

While any file is write-blocked, the system SHALL display a non-dismissable recovery notice that names the blocked file and explains that fallback data is in-memory-only (or, for config, that settings will revert on restart). A later successful save of an unrelated file SHALL NOT clear the recovery notice, and a recovery notice SHALL NOT be retired except by a verified recovery or restart.

#### Scenario: Recovery banner persists across unrelated success

- **WHEN** snippets is blocked and a config write succeeds (or vice versa)
- **THEN** the recovery banner naming the blocked file remains visible

### Requirement: Bounded reads before allocation

`storage::load_json` SHALL read at most `limit + 1` bytes from disk (with an optional metadata preflight as an optimization only) before deciding to reject an oversized file, and SHALL NOT allocate capacity from untrusted file-length metadata. The limit SHALL be injectable for tests. Existing `Missing`/`Corrupt` semantics for zero-byte, whitespace-only, invalid UTF-8, and invalid JSON SHALL be preserved, and valid files within the limit SHALL load normally.

#### Scenario: Exact-limit load succeeds, limit+1 is rejected

- **WHEN** a file contains exactly `limit` bytes of valid JSON
- **THEN** it loads as `Loaded`
- **WHEN** any file contains `limit + 1` or more bytes
- **THEN** it is rejected as `Corrupt` after reading at most `limit + 1` bytes

### Requirement: Explicit migration and data-dir init states

Startup SHALL distinguish `NotNeeded` (stable data already present), `NoSource` (no legacy source), `Migrated` (copy verified then source renamed to `.migrated`), and `Blocked` (legacy source exists but could not be copied or verified, or the stable data directory could not be created). A `Blocked` outcome SHALL leave the legacy source untouched and SHALL surface the exact source path and failure reason in the UI; `Blocked` MUST NOT be presented as a clean first launch, and defaults MUST NOT be seeded over a destination that would hide a known blocked migration.

#### Scenario: Failed migration is surfaced, not hidden

- **WHEN** a non-empty legacy `snippets.json` or `config.json` exists but the copy to the stable location fails or cannot be verified
- **THEN** startup reports the blocked source path and reason and leaves the source byte-for-byte intact

#### Scenario: Data-directory initialization failure is surfaced

- **WHEN** the stable data directory (`%APPDATA%\CopyIt` or fallback) cannot be created or accessed at startup
- **THEN** the system reports the target path and failure reason in the UI rather than silently falling back or deferring to a later write failure
