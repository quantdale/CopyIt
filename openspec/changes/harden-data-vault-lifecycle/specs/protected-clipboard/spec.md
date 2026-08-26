# Spec: protected-clipboard

Time-bounded, best-effort clearing of protected-copy plaintext on the clipboard.

## ADDED Requirements

### Requirement: Protected copies expire from the clipboard

Copying a protected snippet SHALL place plaintext on the clipboard immediately and SHALL schedule a best-effort clear after a fixed security window (30 seconds). The clear SHALL fire only if the app can prove the clipboard still holds the copy it placed there; if the clipboard changed since the copy, it SHALL do nothing. The app SHALL NOT retain the protected plaintext to make this comparison.

#### Scenario: Still-current protected copy is cleared at expiry

- **WHEN** the security window elapses and no application has written the clipboard since CopyIt's protected copy
- **THEN** the clipboard is cleared

#### Scenario: Newer clipboard content is never clobbered

- **WHEN** the user or another application copies something new before the window elapses
- **THEN** expiry does nothing

### Requirement: Vault lock clears the protected copy early

Locking the vault SHALL attempt the same guarded immediate clear of a still-current protected clipboard entry.

#### Scenario: Lock clears the current protected copy

- **WHEN** the user locks the vault while a protected copy is still pending expiry
- **THEN** the clipboard is cleared immediately if it still holds that copy

### Requirement: Ordinary copies are unchanged; failures are non-fatal

Unprotected snippet copies SHALL retain normal persistent clipboard behavior with no security timeout. Clipboard backend failures SHALL be surfaced once and non-fatal. Clearing is best-effort and cannot revoke content already captured by clipboard-history software.

#### Scenario: Unprotected copy persists

- **WHEN** an unprotected snippet is copied
- **THEN** the clipboard keeps the text indefinitely with no scheduled clear
