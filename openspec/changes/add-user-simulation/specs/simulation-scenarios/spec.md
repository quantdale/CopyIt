# Spec: simulation-scenarios

The reusable simulation platform above the harness: personas with seeded behavioral timing, composable journeys over the app's complete workflows, data fixtures, the journey library, and CI integration.

## ADDED Requirements

### Requirement: Reusable configurable personas

The system SHALL define personas as named, reusable configurations of behavioral timing (think-time range between actions, per-character typing cadence, typo probability) and decision weights (which journeys or branches to pick when a choice exists). Persona randomness SHALL draw only from the run's seeded PRNG, so a persona is fully deterministic per seed. Personas SHALL be definable and parameterizable in code without modifying the harness.

#### Scenario: Persona timing shapes the event stream

- **WHEN** two journeys run under personas with different think-time ranges
- **THEN** the virtual-time gaps between actions differ per persona while the action sequence and app outcomes remain correct

#### Scenario: Persona determinism

- **WHEN** the same persona runs the same journey twice with the same seed
- **THEN** every think time, typing delay, typo, and decision is identical across both runs

### Requirement: Composable journey definitions

Journeys SHALL be defined in Rust as compositions of harness intents and persona behaviors, with compile-time checking. A journey SHALL declare its persona, its initial fixtures (library contents, categories, config), and its steps interleaving actions with assertions. The platform SHALL provide extension hooks (custom step functions) so future features — including protected cards from `add-protected-snippets` — can add journeys for their own modals and flows without harness changes.

#### Scenario: New feature adds a journey without harness changes

- **WHEN** a developer adds a journey for a new feature using the public intent and hook vocabulary
- **THEN** the journey compiles and runs without modifying harness or persona code

### Requirement: Isolated fixtures and test data

Each journey SHALL receive its data only through fixtures: explicit snippet/category/config sets, or the seeded default library, materialized into the run's temporary store before the first frame. Fixtures SHALL NOT reference, copy, or derive from any real user data file. A journey SHALL be able to request a pre-corrupted or pre-populated data file to exercise recovery paths.

#### Scenario: Journey starts from a fixture library

- **WHEN** a journey declares a fixture of 20 snippets across 3 categories
- **THEN** the simulated app launches with exactly that library in its temporary store, and the real data directory is untouched

#### Scenario: Corrupt-file fixture exercises recovery

- **WHEN** a journey declares a malformed `snippets.json` fixture
- **THEN** the app shows the corrupt-file recovery banner, renames the file aside in the temporary store, and the journey asserts both the banner and the preserved bytes

### Requirement: Complete-workflow journey library

The journey library SHALL cover, at minimum: first-run exploration (seeded library, search, category filter, copy with feedback, editor open/cancel); CRUD (add with new category, edit, delete, persistence after each); drag-and-drop reordering (unfiltered and within a filtered view); category management (add via header and editor, reserved/blank rejection); theme switching with persistence across a simulated restart; corrupt-data recovery; and save-error handling (read-only data directory, banner shown, in-memory state preserved). Every journey SHALL end with assertions on visible state and on the persisted files.

#### Scenario: End-to-end add-edit-copy journey

- **WHEN** the PowerOrganizer journey adds a snippet, edits its body, then copies it
- **THEN** the editor, grid, and clipboard feedback behave as for a real user, and `snippets.json` in the temporary store contains exactly the final content

#### Scenario: Filtered drag journey

- **WHEN** a journey reorders cards while a category filter is active
- **THEN** the reorder applies to the full library in the order the filtered view implied, matching the documented filtered-reorder semantics

#### Scenario: Theme persists across simulated restart

- **WHEN** a journey switches theme, drops the app instance, and rebuilds it from the same temporary store
- **THEN** the restarted app applies the selected theme, proving `config.json` round-trips

#### Scenario: Save-error journey

- **WHEN** a journey makes the data directory read-only and performs a mutation
- **THEN** the warning banner reports the failure, in-memory state stays correct, and no partial file is written

### Requirement: Regression and failure detection

The suite SHALL fail a journey on: an unmet expectation, a missing interaction target, a panic or unexpected error in the app, or a persistence mismatch. A meta-test SHALL prove detection works by running a journey with a deliberately false expectation and asserting the suite reports failure with a complete bundle.

#### Scenario: Broken workflow is detected

- **WHEN** an app change breaks a workflow a journey covers (e.g. the editor no longer opens)
- **THEN** the corresponding journey fails at the affected step with a locator or expectation error and a debugging bundle

### Requirement: Environment coverage

The headless suite SHALL run locally via `cargo test` and in CI on the existing `windows-latest` job, on every push and pull request, completing in under 60 seconds. CI SHALL upload the `sim-report/` directory as a build artifact on failure. Headed mode SHALL be available for local runs only and SHALL NOT be required for CI success.

#### Scenario: CI gate runs the suite

- **WHEN** a pull request is opened
- **THEN** the journey suite runs headlessly on `windows-latest`, blocks merge on failure, and publishes the failure bundles as artifacts

### Requirement: No impact on the shipped application

Simulation code SHALL compile only under `cfg(any(test, feature = "sim"))`. The suite SHALL satisfy `cargo clippy --all-targets -- -D warnings`. The release build without the `sim` feature SHALL contain no simulation code, with the release size profile (`opt-level = "z"`, LTO, strip) unaffected.

#### Scenario: Release binary excludes simulation

- **WHEN** `cargo build --release` runs without `--features sim`
- **THEN** the binary contains no harness, persona, or journey symbols and behaves exactly as before this change
