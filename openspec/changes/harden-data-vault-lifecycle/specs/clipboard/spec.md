# Spec: clipboard

Bounded, sequence-guarded lifetime for protected copies without clobbering newer clipboard data.

## ADDED Requirements

### Requirement: Protected copies have a bounded best-effort lifetime

Copying a protected snippet SHALL place its plaintext on the clipboard immediately and schedule a best-effort clear after a fixed security window (30 seconds). Ordinary unprotected copies SHALL retain persistent clipboard behavior unless explicitly documented otherwise.

#### Scenario: Protected copy appears immediately

- **WHEN** a protected snippet is copyable (vault unlocked) and the user copies it
- **THEN** the clipboard holds the plaintext body immediately

#### Scenario: Unprotected copy persists

- **WHEN** an unprotected snippet is copied
- **THEN** the clipboard holds its body with no scheduled removal

### Requirement: Clipboard clearing is sequence-guarded and never retains plaintext

The system SHALL NOT retain the protected plaintext to compare later. Instead it SHALL record only a clipboard sequence token (e.g., `GetClipboardSequenceNumber`) at copy time. At expiry, the system SHALL clear the clipboard only if the sequence is still equal to the recorded value; if the user or any other application has changed the clipboard since, the system SHALL do nothing. The sequence check MAY be `None` on platforms without it, in which case the system SHOULD treat the clipboard as changed (do not clear). Clearing SHALL be best-effort and failure-safe; contention SHALL NOT crash or freeze the app.

#### Scenario: Still-current protected copy is cleared at expiry

- **WHEN** the security window elapses and the clipboard sequence has not changed since CopyIt placed the protected copy
- **THEN** the clipboard is cleared

#### Scenario: Newer clipboard content is never clobbered

- **WHEN** the user or any other application writes the clipboard before the window elapses
- **THEN** CopyIt does not clear the clipboard at expiry

### Requirement: Vault lock clears early and failures are non-fatal

Locking the vault SHALL attempt an immediate, sequence-guarded clear of a still-current protected copy. Any clipboard backend failure (set or clear) SHALL surface a single deduplicated warning and SHALL NOT crash; a failed protected copy SHALL NOT schedule a clear. The system SHALL never write protected plaintext to logs, reports, error strings, or `config.json`.

#### Scenario: Lock clears the current protected copy

- **WHEN** the vault is locked while a protected copy's window is still pending and the clipboard sequence is unchanged
- **THEN** the clipboard is cleared immediately

#### Scenario: Backend failure is non-fatal and surfaced

- **WHEN** the clipboard backend returns an error for a protected copy
- **THEN** no clear is scheduled, the clipboard remains empty, and a single warning is surfaced
