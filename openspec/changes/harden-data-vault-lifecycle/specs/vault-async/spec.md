# Spec: vault-async

Off-thread, bounded, cancellable vault KDF execution that preserves determinism.

## ADDED Requirements

### Requirement: Vault KDF runs off the UI thread behind a bounded single-job state machine

Vault `unlock` and `create` (Argon2id) SHALL run off the `egui` frame thread via a bounded state machine that allows at most one job at a time. The creation path SHALL persist `VaultMeta` atomically and roll back in-memory `vault_meta` and the session key on persistence failure. The modal SHALL enter a busy state while work is outstanding and SHALL reject duplicate submit clicks without spawning a second job, while still allowing cancel.

#### Scenario: Busy rejects duplicate submit

- **WHEN** a vault job is already running and the user clicks Unlock/Create again
- **THEN** no second job is spawned and the modal remains in the busy state

### Requirement: Stale and canceled results are discarded safely

A result that arrives after the user canceled the prompt or after a newer submission (different generation) SHALL NOT unlock the vault, SHALL NOT execute a pending `Copy`/`Edit`/`ProtectSave` action, and SHALL NOT persist `VaultMeta`. A canceled `ProtectSave` SHALL restore the editor draft that was parked in the pending action.

#### Scenario: Cancel discards stale result and restores editor

- **WHEN** the user creates a vault for a pending protected save and cancels before the KDF finishes
- **THEN** the editor is restored, the vault stays locked, and the later KDF result is discarded without persisting or copying

### Requirement: Vault stays locked until verified success

The vault SHALL remain `Locked` until a verified success result for the current generation is polled and accepted. A wrong password or corrupt canary result SHALL keep the prompt open with the appropriate inline error and SHALL retain the pending action for retry.

#### Scenario: Wrong password keeps prompt open with error

- **WHEN** KDF finishes with a wrong password
- **THEN** the prompt remains open showing “Wrong password. Try again.”, the pending action is retained, and the vault stays locked

### Requirement: Zeroized passwords and polite polling without a busy loop

Password material owned by the worker SHALL be zeroized (`zeroize`) after derivation. The UI SHALL poll the worker with `try_recv` once per frame and `request_repaint()` while a job is outstanding, never a busy loop.

#### Scenario: Worker zeroizes password after derivation

- **WHEN** a vault job completes (success or failure)
- **THEN** the worker-side password buffer is zeroized before sending the result

### Requirement: Deterministic simulation

In `cfg(any(test, feature = "sim"))` the KDF execution SHALL be deterministic via an immediate executor (synchronous completion on the same `handle_vault_prompt` call) so headless journeys remain byte-identical for a fixed seed. Production SHALL use a real thread. The behavior/state machine (busy, stale discard, rollback, pending dispatch) SHALL be identical in both modes.

#### Scenario: Same journey + seed is byte-identical

- **WHEN** a journey is run twice with the same seed under the headed or headless harness
- **THEN** the normalized event log is byte-identical
