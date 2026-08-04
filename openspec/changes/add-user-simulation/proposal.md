# Proposal: add-user-simulation

## Why

CopyIt's quality today rests on `cargo test` unit tests plus a manual smoke pass before each release (see `tasks.md` 6.2 in `add-protected-snippets`). Every new feature — protected cards being the current example — adds more workflows that someone must click through by hand: add/edit/delete, drag-reorder, search/filter, category management, theme switching, corrupt-file recovery. Manual passes are repetitive, skipped under time pressure, and cannot catch regressions between releases. The app needs a reusable platform that drives CopyIt the way a real user does — clicking what is on screen, typing with realistic timing, completing whole journeys, reacting to modals and error banners — and runs it in CI on every push.

Several items from the original capability request describe a client/server web app and must be mapped onto what CopyIt actually is: there is **no browser, backend, API, authentication, staging, or production environment**. The equivalents for a local single-user egui desktop app are:

- *Browser automation* → in-process synthesis of real egui input events (pointer, keys, text) against the real `CopyIt` UI, plus an opt-in headed mode that runs the actual window.
- *API / persistence validation* → assertions against the on-disk `snippets.json` / `config.json` and the `Store` seam after every journey.
- *Isolation from production data* → a hard guarantee that simulation never touches `%APPDATA%\CopyIt`; every run uses a throwaway `Store::at(temp_dir)`.
- *Staging / CI environments* → local dev (`cargo test`) and the existing `windows-latest` CI job; headed mode is local-only.

## What Changes

- **Simulation harness (`src/sim/`, new).** An in-process robot that pumps frames through the real `CopyIt` UI via `egui::Context::run` (egui 0.27.2, already locked), feeding synthetic `RawInput` pointer/key/text events on a deterministic virtual clock. Actions are expressed as user intents ("click the card titled X", "type into the Title field") and resolved by locating visible text in the tessellated frame output — the robot clicks what a user sees, not internal coordinates.
- **UI seam in `app.rs`.** The body of `CopyIt::update` (which already ignores its `eframe::Frame` argument, `src/app.rs:785`) moves into `CopyIt::ui(&mut self, ctx: &egui::Context)` so the harness can drive the identical render/interaction code without eframe. App construction factors out a `CopyIt::from_store(Store)` path next to `CopyIt::new`, mirroring the existing `test_app()` helper (`src/app.rs:1380`).
- **Personas and journeys (`simulation-scenarios`).** A declarative scenario layer on top of the harness: seeded-random personas with realistic think-time and typing cadence (including occasional typo-and-correct), reusable data fixtures, and a journey library covering first-run exploration, CRUD, drag-reorder, category management, theme persistence, corrupt-file recovery, and save-error handling. Same seed ⇒ byte-identical event log, so failures reproduce exactly.
- **Reporting.** Every run writes a JSONL event log, per-step visible-text snapshots, and — on failure — a debugging bundle (last N events, final snapshot, copies of the data files, the seed). Headed mode (`cargo run --features sim -- --simulate <journey>`) can capture real PNG screenshots via `ViewportCommand::Screenshot` (available in egui 0.27.2). CI uploads the report directory on failure.
- **CI integration.** A simulation step in the existing `build-test` job (windows-latest, headless). Simulation code is compiled under `cfg(any(test, feature = "sim"))`, so the release binary and its size profile are untouched.
- **Docs.** `AGENTS.md` gains the simulation module's responsibilities and the "never let a journey touch the real store" rule; `README.md` gains a short contributor section on running journeys.

## Capabilities

### New Capabilities

- `simulation-harness`: The in-process egui driver — frame pumping, synthetic input and virtual clock, visible-text locators, user-intent actions (click/type/drag/wait), snapshots, the real-store isolation guard, and the opt-in headed mode with screenshots.
- `simulation-scenarios`: The reusable simulation platform above the harness — personas with seeded behavioral timing, the scenario/journey definition style, fixtures, the journey library, run reporting and failure bundles, and CI integration.

### Modified Capabilities

<!-- No specs exist under openspec/specs/ yet; there is nothing to modify. -->

## Impact

- **`src/app.rs`**: extract `CopyIt::ui(ctx)` from `update` (mechanical move; `_frame` is already unused); add `CopyIt::from_store(Store)` constructor shared by `new`, tests, and the harness. No behavior change; existing tests keep passing.
- **`src/sim/` (new module)**: `harness.rs` (frame pump, input synthesis, locators), `persona.rs` (timing model, seeded RNG), `journey.rs` (journey DSL and library), `report.rs` (event log, snapshots, failure bundle). Compiled only under `cfg(any(test, feature = "sim"))`.
- **`Cargo.toml`**: optional `sim` feature (headed mode only); one new dev-dependency for seeded RNG (`rand` with a pinned seed — pure Rust, consistent with the dependency policy).
- **`src/main.rs`**: under `feature = "sim"` only, a `--simulate <journey>` CLI entry that runs a journey in the real window with screenshot capture.
- **CI** (`.github/workflows/ci.yml`): one added step running the journey suite; artifact upload of `sim-report/` on failure.
- **Docs**: `AGENTS.md` (module list, performance-model and testing sections), `README.md` (contributor note).
- **Compatibility with `add-protected-snippets`**: both changes touch `src/app.rs`, but disjointly — this change moves `update`'s body and adds a constructor; that change adds dispatch logic inside it. The scenario layer exposes custom-step hooks so vault journeys (unlock prompt, censored cards, wrong-password handling) can be added when that change lands, without reworking the harness. No changes to that change's specs.
