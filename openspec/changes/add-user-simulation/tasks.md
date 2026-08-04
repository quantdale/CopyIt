# Tasks: add-user-simulation

## 1. UI seam and module skeleton

- [x] 1.1 Move the body of `CopyIt::update` (`src/app.rs:785`) unchanged into `fn ui(&mut self, ctx: &egui::Context)`; make `update` a one-line delegation — `_frame` is already unused, so no call-site changes
- [x] 1.2 Factor construction into `CopyIt::from_store(store: Store, ctx: &egui::Context)`; `CopyIt::new` calls it after applying the theme from `CreationContext`; refactor the `test_app()` helper (`src/app.rs:1380`) to use it
- [x] 1.3 Add `mod sim;` to `src/main.rs` gated on `cfg(any(test, feature = "sim"))` and create `src/sim/` with empty submodules; add the optional `sim` feature and `rand` (dev-dependency, seeded) to `Cargo.toml`
- [x] 1.4 Verify the seam is behavior-preserving: `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` all pass with no test edits (close any running CopyIt first — the exe is locked while running)

## 2. Harness core (`src/sim/harness.rs`)

- [x] 2.1 Implement the frame pump: `SimApp` wrapping `CopyIt` + `egui::Context`, pumping frames via `Context::run` with synthetic `RawInput` (screen rect 1000x700, virtual `time`, event queue); assert the app produces a `FullOutput` with the top bar visible
- [x] 2.2 Implement the virtual clock (`advance(ms)` advancing `RawInput.time`) and a seeded PRNG owned by the run; test that no wall-clock sleep or system clock read paces the simulation
- [x] 2.3 Implement the isolation guard: `SimApp::build` accepts only `Store::at` paths under `<temp>/copyit-sim/<pid>/<run>`, refuses otherwise with a runtime error before any frame is pumped, and never calls `Store::open`/`migrate_legacy`; test both the accept and refuse paths
- [x] 2.4 Implement visible-text locators: extract text galleys with positions from `FullOutput.shapes`, find a label's widget position, fail with "label not visible; on screen: …" when absent; test against the top bar and an open editor

## 3. Intents, snapshots, and reporting

- [x] 3.1 Implement intents: `click_text`, `type_into` (per-character text events with seeded cadence and typo-and-correct), `press_key`, `wait`, `expect_visible`/`expect_absent`, `expect_store` (reload `snippets.json`/`config.json` from the run's store and assert); unit-test each through `SimApp`
- [x] 3.2 Implement `drag_card` honoring the 4px `DragMachine` threshold, targeting gaps via `grid::grid_card_rect`/`gap_point`; test an unfiltered and a filtered reorder end-to-end through pointer events (not by calling `reorder` directly)
- [x] 3.3 Implement per-step visible-text snapshots (labels, card previews, banner/modal text) and the JSONL event log (step, intent, resolved target, input events, virtual time, state digest)
- [x] 3.4 Implement the failure bundle in `src/sim/report.rs`: full log, last snapshot, failing expectation, copies of the run's data files, seed, and repro command under `sim-report/`; add `sim-report/` to `.gitignore`

## 4. Personas and journey DSL (`src/sim/persona.rs`, `src/sim/journey.rs`)

- [x] 4.1 Define `Persona` (think-time range, typing cadence, typo probability, decision weights) with all randomness from the run's seeded PRNG; define the four personas from the design (FirstRunExplorer, PowerOrganizer, ErrorHandler, ThemeHopper)
- [x] 4.2 Define the journey builder: persona + fixtures (snippet/category/config sets, seeded default library, pre-corrupted or read-only data dir) + steps interleaving intents and assertions, plus a custom-step hook for future features
- [x] 4.3 Determinism test: same journey + persona + seed run twice produces byte-identical normalized event logs; different seeds produce valid but different timing

## 5. Journey library

- [x] 5.1 FirstRunExplorer journeys: seeded library visible; search narrows the grid; category filter works; copy shows the "Copied" feedback (via virtual clock); editor opens and cancels without mutating; store files unchanged at the end
- [x] 5.2 PowerOrganizer journeys: add ~20 snippets across new and existing categories; edit a body; delete one; drag-reorder unfiltered and within a filtered view; assert `snippets.json` content and order after every mutation
- [x] 5.3 ErrorHandler journeys: corrupt `snippets.json` fixture → `.corrupt` rename + banner + seeded defaults in the temp store; read-only data dir → save-error banner with in-memory state intact and no partial file; reserved/blank category rejected with the form kept open
- [x] 5.4 ThemeHopper journeys: switch several themes via the top bar; drop the app and rebuild from the same temp store; assert the restarted app loads the persisted theme from `config.json`
- [x] 5.5 Meta-test: a journey with a deliberately false expectation fails and produces a complete failure bundle (log, snapshot, seed, data files, repro command)
- [x] 5.6 Isolation test: run the full suite and assert no file under the real `%APPDATA%\CopyIt` paths was created, modified, migrated, or deleted

## 6. Headed mode

- [x] 6.1 Add a `--simulate <journey> [--seed N]` CLI path in `src/main.rs` under `feature = "sim"` only, running a journey against the real eframe window with a temporary store (isolation guard applies)
- [x] 6.2 Capture per-step PNG screenshots via `egui::ViewportCommand::Screenshot` (egui 0.27.2) into the run's report bundle
- [x] 6.3 Verify `cargo build` and `cargo build --release` without `--features sim` compile no simulation code and reject `--simulate`; run one headed journey locally as a smoke check

## 7. CI and docs

- [x] 7.1 Add a journey-suite step to the `build-test` job in `.github/workflows/ci.yml` (windows-latest, headless) and upload `sim-report/` as an artifact on failure; confirm the suite finishes under 60 s
- [x] 7.2 Update `AGENTS.md`: `src/sim/` module responsibilities, the "journeys never touch the real store" rule, the `ui(ctx)`/`from_store` seam, the sim feature, and the CI step; add a short contributor section to `README.md`
- [x] 7.3 Run `cargo test`, `cargo clippy --all-targets -- -D warnings`, and `cargo build --release` (close any running CopyIt first)
- [x] 7.4 Run `openspec validate add-user-simulation --strict`

## 8. Follow-ups (after `add-protected-snippets` lands — not part of this change)

- [x] 8.1 Add vault journeys via the custom-step hook: happy-path unlock + copy, wrong-password rejection, censored rendering while locked, lock/unlock cycle
  - Implemented in `src/sim/journey.rs`: `vault-happy-unlock-copy`, `vault-wrong-password`, `vault-censored-while-locked`, `vault-lock-unlock-cycle` (tests `sim_journeys_vault_*`), each asserting clipboard, visible text, and on-disk ciphertext. All four run in the required CI set (`cargo test --all-targets`).
- [x] 8.2 Decide (design Open Questions) whether per-step PNGs suffice or headed mode should record video, and whether a nightly randomized-seed job complements the fixed-seed PR gate
  - **Decision: per-step PNGs suffice.** Headed mode captures per-step PNGs; video adds maintenance for little gain on a single-user local app (the design's stated lean). **Nightly randomized-seed job: not pursued.** The fixed-seed PR gate is the correctness gate and determinism test already covers seed reproducibility; a nightly job is optional coverage that this project (local app, no server) does not need. The custom-step hook keeps the platform open if either changes later.
