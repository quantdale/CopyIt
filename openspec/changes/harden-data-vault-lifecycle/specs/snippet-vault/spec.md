# Spec: snippet-vault

Delta: vault key derivation moves off the UI thread behind a bounded, cancellable work state machine.

## ADDED Requirements

### Requirement: Non-blocking, bounded key-derivation execution

Vault create and unlock SHALL derive keys off the UI thread through a bounded state machine: at most one derivation job may run at a time, the prompt SHALL show a busy state that rejects duplicate submissions while a job runs, the vault SHALL stay locked until a verified success for the *current* request is accepted, and stale or canceled results SHALL NOT execute pending actions. Password material owned by the worker SHALL be zeroized after use. Production uses a worker thread; tests and simulation use a deterministic executor seam.

#### Scenario: Duplicate submissions are rejected

- **WHEN** the user clicks submit again while a derivation is already running
- **THEN** no second job starts and the UI shows the busy state

#### Scenario: Stale result is discarded safely

- **WHEN** the user cancels the prompt and its derivation result arrives afterward
- **THEN** the result is dropped without unlocking or running any pending action

#### Scenario: Wrong password leaves the vault locked

- **WHEN** an asynchronous unlock fails verification
- **THEN** the modal shows the inline error and the vault remains locked

#### Scenario: Create persists metadata before protected saves proceed

- **WHEN** an asynchronous create succeeds but persisting vault metadata fails
- **THEN** the in-memory key/meta roll back and the error is surfaced
