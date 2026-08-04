# Spec: snippet-vault

The password-based vault backing protected snippets: key derivation, password verification, session lock/unlock state, and encrypt/decrypt operations.

## ADDED Requirements

### Requirement: Vault password creation

The system SHALL create the vault the first time the user protects a snippet, deriving an encryption key from the chosen password with Argon2id (m = 19 MiB, t = 2, p = 1) and a fresh random 16-byte salt. The system SHALL store only the salt and an encrypted canary (a fixed known plaintext encrypted with the vault key, with its nonce) in `config.json`. The password itself SHALL never be stored in any form.

#### Scenario: First protection sets the vault password

- **WHEN** the user protects a snippet and no vault exists yet
- **THEN** the system prompts for a new password with a confirmation field, derives the vault key, and persists the salt and encrypted canary to `config.json` via the atomic save path

#### Scenario: Mismatched confirmation is rejected

- **WHEN** the user enters a new vault password whose confirmation does not match
- **THEN** the system shows an inline error and creates no vault

### Requirement: Password verification via canary

The system SHALL verify a candidate password by deriving a key with the stored salt and attempting to decrypt the canary. A successful authenticated decryption means the password is correct; any failure means it is wrong. Verification SHALL NOT consult or modify snippet data.

#### Scenario: Correct password unlocks

- **WHEN** the user enters the correct vault password in the unlock prompt
- **THEN** the system caches the derived key in memory, marks the vault unlocked, and proceeds with the pending action

#### Scenario: Wrong password is rejected

- **WHEN** the user enters an incorrect password
- **THEN** the system shows an inline "wrong password" error, keeps the vault locked, performs no copy/edit, and allows retrying

### Requirement: Session-scoped unlock state

The system SHALL start every app launch with the vault locked, SHALL keep the derived key only in memory while unlocked, and SHALL discard it when the user locks the vault manually or the app exits. Unlock state SHALL never be persisted to disk.

#### Scenario: Unlock persists for the session

- **WHEN** the vault has been unlocked during the session
- **THEN** subsequent copy/edit actions on protected cards proceed without another password prompt

#### Scenario: Manual lock drops the key

- **WHEN** the user clicks the Lock control while the vault is unlocked
- **THEN** the system discards the cached key and subsequent protected actions prompt for the password again

### Requirement: Authenticated encryption of protected bodies

The system SHALL encrypt a protected snippet's body with XChaCha20-Poly1305 using the vault key and a fresh random 24-byte nonce on every save, and SHALL store only the base64-encoded ciphertext and nonce (plus the hint, see protected-cards) in `snippets.json`. The plaintext body field of a protected snippet SHALL be empty on disk.

#### Scenario: Save encrypts with a fresh nonce

- **WHEN** a protected snippet is created or edited and saved
- **THEN** its body is encrypted under a newly generated nonce and the stored JSON contains ciphertext, nonce, and hint, with an empty `body` field

### Requirement: Decryption fails safely

The system SHALL treat any AEAD decryption failure as an error to report, never as data to use. A wrong key or tampered/corrupt ciphertext SHALL produce an error result; the app SHALL surface it (inline or via the existing warning banner), refuse the copy/edit, and leave the stored file untouched. Decryption failures SHALL NOT panic or overwrite data.

#### Scenario: Tampered ciphertext is refused

- **WHEN** the vault is unlocked (canary verified) but a protected card's ciphertext fails to decrypt
- **THEN** the system shows a warning naming the card, performs no copy/edit, and does not modify `snippets.json`
