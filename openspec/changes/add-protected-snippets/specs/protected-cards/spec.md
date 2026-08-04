# Spec: protected-cards

The user-facing behavior of protected snippet cards: censored rendering, search exclusion, password-gated copy/edit/delete, protect/unprotect in the editor, and the encrypted on-disk representation.

## ADDED Requirements

### Requirement: Censored card rendering

A protected card SHALL never render its body in the grid. Its preview line SHALL show the stored hint (at most 5 characters) followed by masking bullets, or only masking bullets when no hint exists — regardless of whether the vault is unlocked. A protected card SHALL display a lock indicator so it is visually distinguishable.

#### Scenario: Locked protected card shows a masked preview

- **WHEN** a protected snippet with hint `ghp_x` is displayed
- **THEN** the card preview shows `ghp_x••••••` and no other body content

#### Scenario: Protected card with no hint shows only bullets

- **WHEN** a protected snippet whose body was shorter than 12 characters (empty hint) is displayed
- **THEN** the card preview shows only masking bullets

#### Scenario: Unlocked cards stay masked

- **WHEN** the vault is unlocked and the grid repaints
- **THEN** protected cards still show the masked preview, not the decrypted body

### Requirement: Protected bodies excluded from search

The system SHALL exclude protected bodies from search matching: the derived lowercase body text of a protected snippet SHALL be empty. Title and category of a protected snippet SHALL remain searchable.

#### Scenario: Searching a secret finds nothing

- **WHEN** the user searches for a string that appears only in a protected snippet's body
- **THEN** that card is not shown in the results

#### Scenario: Searching a title still finds a protected card

- **WHEN** the user searches for a string in a protected snippet's title or category
- **THEN** the card appears with its masked preview

### Requirement: Password-gated copy

Copying a protected card while the vault is locked SHALL open the unlock prompt instead of copying. While unlocked, copying a protected card SHALL decrypt the body and place the plaintext on the clipboard.

#### Scenario: Copy while locked prompts first

- **WHEN** the user clicks Copy on a protected card and the vault is locked
- **THEN** an unlock prompt appears and nothing is copied until the correct password is entered

#### Scenario: Copy while unlocked places plaintext on the clipboard

- **WHEN** the user clicks Copy on a protected card and the vault is unlocked
- **THEN** the decrypted body is copied to the clipboard and the normal "Copied" feedback is shown

### Requirement: Password-gated edit and delete

Opening the editor on a protected card while the vault is locked SHALL open the unlock prompt first; the editor SHALL then show the decrypted body. Deleting a protected snippet SHALL require the same unlock, because delete is only reachable from within the editor.

#### Scenario: Edit while locked prompts first

- **WHEN** the user clicks Edit on a protected card and the vault is locked
- **THEN** an unlock prompt appears and the editor opens only after the correct password is entered

#### Scenario: Editor shows the decrypted body

- **WHEN** the editor is opened on a protected card while unlocked
- **THEN** the content field contains the decrypted plaintext body

### Requirement: Protecting and unprotecting in the editor

The editor SHALL offer a "Protect this snippet" control, enabled only while the vault is unlocked. Saving with protection enabled SHALL encrypt the body (fresh nonce) and store an empty plaintext body. Saving with protection disabled SHALL store the body as plaintext. Protecting a card when no vault exists SHALL first trigger vault password creation.

#### Scenario: Protect a plain snippet

- **WHEN** the user checks "Protect this snippet" on an unprotected snippet and saves
- **THEN** the snippet is stored encrypted with a hint, and its card renders censored

#### Scenario: Unprotect a protected snippet

- **WHEN** the user unchecks "Protect this snippet" on a protected snippet and saves
- **THEN** the body is stored as plaintext and the card renders normally

#### Scenario: First-ever protect creates the vault

- **WHEN** the user saves a protected snippet and no vault password exists yet
- **THEN** the system prompts to create the vault password (with confirmation) before saving

### Requirement: Encrypted on-disk representation and hint rule

A protected snippet's JSON SHALL contain a `protection` object with `hint`, `nonce`, and `ciphertext` (base64), and an empty `body`. The hint SHALL be the body's first 5 characters when the body is at least 12 characters long, otherwise the empty string. Files written by older versions (no `protection` field, no `vault` config) SHALL load unchanged, and protected files SHALL keep loading correctly across restarts.

#### Scenario: Hint rule applied

- **WHEN** a body of 20 characters is protected
- **THEN** the stored hint is its first 5 characters; when a body of 8 characters is protected, the stored hint is empty

#### Scenario: Old library loads without migration

- **WHEN** the app opens a `snippets.json` written before this feature (no `protection` fields)
- **THEN** every snippet loads as unprotected and behaves exactly as before

#### Scenario: Protected library round-trips

- **WHEN** the app saves protected snippets, exits, and relaunches
- **THEN** the protected cards load censored and can be unlocked and copied with the same vault password

### Requirement: Vault status and manual lock in the top bar

When at least one protected snippet exists, the top bar SHALL show the vault state (locked/unlocked) and, while unlocked, a Lock control. The vault state indicator SHALL NOT appear when no protected snippets exist.

#### Scenario: Lock control re-locks the session

- **WHEN** the vault is unlocked and the user clicks the Lock control in the top bar
- **THEN** the vault returns to locked and protected actions prompt again
