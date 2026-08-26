# Spec: data-recovery

Per-file protection of corrupt originals whose backup rename failed, with truthful recovery UI.

## ADDED Requirements

### Requirement: Per-file write protection after unrecoverable backup failure

If backing up a corrupt `snippets.json` or `config.json` fails, the app SHALL refuse to write, truncate, or replace that specific file for the rest of the process, on every persistence path (seeding, canonicalization, add/edit/delete/reorder, category creation, theme selection, vault metadata updates). The other file SHALL remain writable. The protection SHALL be lifted only by restarting after the underlying problem is corrected.

#### Scenario: Blocked snippets file survives later mutations

- **WHEN** a corrupt snippets file could not be backed up and the user adds, edits, deletes, or reorders snippets
- **THEN** every automatic save refuses to touch the file and its original bytes remain byte-for-byte unchanged

#### Scenario: Blocked config file survives settings changes

- **WHEN** a corrupt config file could not be backed up and the user changes theme, categories, or vault metadata
- **THEN** config saves are refused and the original bytes remain byte-for-byte unchanged

#### Scenario: Files block independently

- **WHEN** both files are blocked simultaneously
- **THEN** neither file's protection is affected by the other's save outcomes

### Requirement: Truthful recovery state in the UI

While any file is write-blocked, the app SHALL display a persistent recovery notice naming the blocked file, explaining that the visible library (or settings) is fallback data not backed by disk, and describing how to recover. The notice SHALL NOT be dismissable for the session, and ordinary successful saves of unrelated work SHALL NOT clear it.

#### Scenario: Recovery notice persists

- **WHEN** snippets writes are blocked and an unrelated operation succeeds
- **THEN** the recovery notice remains visible
