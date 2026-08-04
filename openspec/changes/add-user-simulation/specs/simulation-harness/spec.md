# Spec: simulation-harness

The in-process driver that executes the real CopyIt UI under synthesized user input: frame pumping, a deterministic virtual clock, visible-text locators, user-intent actions, snapshots, real-data isolation, and an opt-in headed mode with screenshots.

## ADDED Requirements

### Requirement: UI simulation seam

The system SHALL expose the entire CopyIt UI through `CopyIt::ui(&mut self, ctx: &egui::Context)`, containing the full body formerly in `eframe::App::update`, so the identical render and interaction code runs both under eframe and under a bare `egui::Context`. `update` SHALL delegate to `ui` with no other logic. The system SHALL provide `CopyIt::from_store(Store, &egui::Context)` constructing a fully initialized app from an explicit store, used by `CopyIt::new`, tests, and the harness.

#### Scenario: Harness and eframe run the same UI code

- **WHEN** the harness pumps a frame through `CopyIt::ui` with a bare `egui::Context`
- **THEN** the rendered output and state transitions are produced by the same code path that `eframe::App::update` delegates to

#### Scenario: Existing behavior is preserved

- **WHEN** the seam is introduced
- **THEN** the entire pre-existing `cargo test` suite passes unchanged and the release binary's behavior is identical

### Requirement: Deterministic frame pump and virtual clock

The harness SHALL drive the app by calling `egui::Context::run` with synthetic `RawInput` per frame, advancing time only through `RawInput.time` on a harness-owned virtual clock. The harness SHALL NOT sleep on the wall clock, spawn threads for timing, or read the system clock to pace the simulation. All randomness SHALL come from a single seeded PRNG per run, recorded in the run report.

#### Scenario: Same seed produces the same event stream

- **WHEN** the same journey is executed twice with the same seed
- **THEN** the normalized event logs (step intents, resolved targets, input events, virtual timestamps) are byte-identical

#### Scenario: Virtual time drives time-dependent UI

- **WHEN** a journey waits for transient feedback (e.g. the "Copied" indicator) to appear and disappear
- **THEN** the harness advances the virtual clock and observes the transition without wall-clock delay

### Requirement: Visible-text locators

The harness SHALL resolve interaction targets by searching the most recent frame's tessellated output for visible text matching the requested label, and SHALL synthesize pointer events at the target's on-screen position. If no match exists, the step SHALL fail with an error naming the missing label and listing the visible text of the current frame.

#### Scenario: Click a control by its label

- **WHEN** a journey step requests `click_text("New snippet")`
- **THEN** the harness clicks the on-screen position of that label's widget, and the editor opens exactly as if a user had clicked it

#### Scenario: Missing label fails informatively

- **WHEN** a journey step references a label that is not visible in the current frame
- **THEN** the step fails, naming the missing label and reporting what text is on screen instead

### Requirement: User-intent action vocabulary

The harness SHALL provide intents covering the full interaction surface of the app: `click_text`, `type_into` (per-character text events with cadence), `press_key`, `drag_card` (press, move past the drag threshold, move to a target gap, release), `wait` (virtual time), `expect_visible` / `expect_absent`, and `expect_store` (assertions on the persisted `snippets.json` / `config.json`). Drag geometry SHALL use the same `grid` module constants and functions as the renderer, so drops land on real insertion gaps.

#### Scenario: Synthesized drag reorders cards

- **WHEN** a journey drags a card onto the gap before another card
- **THEN** the drag machine arms, passes its threshold, shows the insertion line, and the drop reorders the library identically to a real pointer drag, persisted to `snippets.json`

#### Scenario: Typing produces realistic input

- **WHEN** a journey types into the Title field
- **THEN** the app receives one text event per character at the persona's cadence, including any seeded typo-and-correct sequences (backspace events), and the field content matches the intended final text

### Requirement: Visible-text snapshots

The harness SHALL capture per-step snapshots as the frame's visible text (control labels, card previews, banner and modal text), not pixels. Snapshots SHALL be stable across theme changes and font rasterization differences.

#### Scenario: Snapshot reflects what a user sees

- **WHEN** a modal or warning banner is open
- **THEN** the snapshot contains the modal's or banner's text, and a step can assert on it

### Requirement: Real-data isolation guard

The harness SHALL construct apps only via `CopyIt::from_store(Store::at(path))` where `path` lies under a per-run temporary directory (`<temp>/copyit-sim/<pid>/<run>`). Before pumping the first frame, the harness SHALL verify with a runtime check (not only `debug_assert`) that every store path it will use is under that directory, and SHALL refuse to run otherwise. The harness SHALL NOT call `Store::open` or `migrate_legacy`.

#### Scenario: Simulation never touches the real store

- **WHEN** the full journey suite runs on a machine with an existing `%APPDATA%\CopyIt` library
- **THEN** no file under `%APPDATA%\CopyIt` is created, modified, migrated, or deleted

#### Scenario: Harness refuses a real store path

- **WHEN** code attempts to build a simulated app whose store path is not under the per-run temp directory
- **THEN** the harness refuses before any frame is pumped or file is touched

### Requirement: Headed mode with screenshots

Under the `sim` cargo feature, the application SHALL support `copyit --simulate <journey> [--seed N]`, running the named journey against the real eframe window and capturing PNG screenshots per step via the viewport screenshot command. Headed mode SHALL use the same isolation guard (temporary store only) and SHALL NOT be part of CI. Without `--features sim`, the binary SHALL contain no simulation code and the default build SHALL be unaffected.

#### Scenario: Headed run captures screenshots

- **WHEN** a developer runs a journey in headed mode locally
- **THEN** the journey executes against the real window and the report bundle contains a PNG per step

#### Scenario: Default build is untouched

- **WHEN** the app is built without the `sim` feature (debug or release)
- **THEN** no harness, persona, or journey code is compiled in, and `--simulate` is not accepted

### Requirement: Run reporting and failure bundles

Every run SHALL produce a machine-readable report under a gitignored `sim-report/` directory: a JSONL event log (step, intent, resolved target, input events, virtual time, app-state digest), per-step snapshots, and a per-journey pass/fail summary. On failure the harness SHALL additionally write a bundle containing the full log, the last snapshot, the failing expectation, copies of the run's data files, the seed, and the exact command to reproduce the failure.

#### Scenario: Failure bundle reproduces the bug

- **WHEN** any journey step fails
- **THEN** the bundle contains everything needed to reproduce the failure locally with one command, including the seed and the data files at failure time
