# Design: add-user-simulation

## Context

CopyIt is a single-user, offline, native Windows desktop app (Rust, egui/eframe 0.27.2, single `.exe`). All state lives in two JSON files under `%APPDATA%\CopyIt` via the `Store` seam. There is no network, server, account system, or deployed environment — so "system-wide real-user simulation" here means driving the real UI of the real app binary through complete user journeys, deterministically and in CI.

Relevant existing seams (verified against the code):

- `CopyIt::update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame)` (`src/app.rs:785`) never uses `_frame` — the entire UI can run against a bare `egui::Context`.
- egui 0.27.2 provides `egui::Context::run(RawInput, impl FnOnce(&Context)) -> FullOutput` (public, stable API) — exactly the frame pump a headless robot needs, and `Event::Screenshot` as the reply to `ViewportCommand::Screenshot` for headed captures.
- `FullOutput.shapes` carries the tessellated frame, including every text galley and its on-screen position — a faithful "what the user sees" source for locators and snapshots.
- `test_app()` (`src/app.rs:1380`) already proves the isolation pattern: an app whose `Store::at(temp_dir)` keeps tests away from real data.
- The app reads time from `ctx.input(|i| i.time)` (`src/app.rs:786`), so advancing `RawInput.time` simulates realistic timing without sleeping.
- Drag-and-drop is a pointer state machine (`grid::DragMachine`, 4px threshold) whose geometry comes from the same `grid::grid_card_rect` / `gap_point` constants the renderer uses — so synthesized drags can target real gaps.
- CI already runs on `windows-latest` with `cargo test --all-targets` and `clippy -D warnings`.

## Goals / Non-Goals

**Goals**

- Execute complete user journeys (launch → explore → mutate → verify persistence) automatically, locally and in CI, with zero manual clicking.
- Drive the UI the way a user does: locate controls by their visible text, click/type/drag via synthesized egui input events, react to modals, banners, and empty states.
- Realistic behavior: seeded-random think times, per-character typing cadence with occasional typo-and-correct, persona-specific decision patterns. Fully deterministic per seed.
- Reusable platform: personas, fixtures, and journeys are composable building blocks; adding a journey for a new feature (e.g. protected cards) is a few lines, not a new tool.
- Actionable failures: event log, visible-text snapshots, data-file dumps, and the seed, so any failure reproduces locally in one command.
- Hard isolation: a journey can never read, write, migrate, or delete the real `%APPDATA%\CopyIt` data.

**Non-Goals**

- OS-level UI automation (Windows UI Automation, AutoHotkey, image recognition). Flaky, needs an interactive desktop CI does not have, and duplicates what the in-process harness covers more reliably. Headed mode covers the "does the real window work" gap.
- AI-driven exploratory agents. Non-deterministic and un-debuggable as a correctness gate; may be added later as an optional exploration layer, not part of this change.
- Record/replay of real user sessions. Possible future extension; the intent-level DSL already provides the value.
- Browser automation, API orchestration, auth testing, staging/production rollout — none of these exist in this app.
- Performance/load simulation — the app is single-user; the existing virtualization tests cover scale.
- Pixel-perfect visual regression. Snapshots are structured visible-text dumps; pixel goldens are explicitly out.

## Decisions

### 1. In-process egui robot as the backbone; headed mode as the complement

The harness pumps frames with `egui::Context::run`, feeding synthetic `RawInput` events (pointer move/button, key, text) to the real `CopyIt` UI code. This is the egui equivalent of browser automation: same code, same layout, same interaction state machines, but headless, fast (thousands of frames per second), and runnable in CI.

A `sim` cargo feature adds a headed mode (`copyit --simulate <journey>`) that runs the same journey against the real eframe window, capturing PNG screenshots via `ViewportCommand::Screenshot`. Headed mode is for local verification and human review only; it never runs in CI.

Rejected: OS-level input injection (see Non-Goals); forking egui_kittest (it targets newer egui; the project pins 0.27, and `Context::run` gives us everything needed directly).

### 2. One mechanical UI seam: `CopyIt::ui(ctx)` + `CopyIt::from_store(Store)`

`update`'s body moves unchanged into `fn ui(&mut self, ctx: &egui::Context)`; `update` becomes a one-line delegation. Construction logic in `CopyIt::new` that does not need `CreationContext` moves to `CopyIt::from_store(store, ctx)`; `new` calls it after theme setup, and the harness/tests call it with a `Store::at(temp_dir)`. This is the only change to shipped code paths, it is behavior-preserving, and `cargo test` + `clippy -D warnings` prove it.

### 3. Locate by visible text, act by intent

Steps are intents: `click_text("New snippet")`, `type_into("Title", "…")`, `drag_card(title, before_title)`, `expect_visible("Copied")`, `expect_store(|snippets| …)`. Locators search the previous frame's `FullOutput.shapes` for text galleys matching the label and click the center of the enclosing widget area. This mirrors how a user finds controls, survives layout refactors that keep labels, and fails with "label not on screen" — the exact failure a user would hit. Drag steps use `grid::grid_card_rect`/`gap_point` coordinates (the renderer's own constants) so drops land on real gaps.

### 4. Deterministic realism via a virtual clock and seeded RNG

All randomness (think time 150–1200 ms, typing 40–140 ms/char, ~5% typo-then-backspace, persona choices) comes from one seeded PRNG per run. Time only advances through `RawInput.time` on the harness's virtual clock — no wall-clock sleeps, no threads. Consequence: same seed ⇒ identical event stream ⇒ reproducible failures and a cheap determinism test (run twice, diff the normalized logs). Journeys never "retry on flake"; a failure is a failure and gets investigated.

### 5. Isolation is enforced by construction, not by convention

The harness can only build an app through `CopyIt::from_store(Store::at(<tempdir>))`. The tempdir root is `std::env::temp_dir()/copyit-sim/<pid>/<run>`; the harness holds the path and asserts (runtime check, not just `debug_assert`) that every store path it touches is under it before the first frame. `Store::open()` and `migrate_legacy()` are never called in simulation. Report artifacts are written under `sim-report/` (gitignored), never into the repo's data locations.

### 6. Failures ship a debugging bundle; reporting is structured, not pixel-based

Each step appends to a JSONL event log (`step, intent, resolved rect, input events, virtual time, app-state digest`). Per-step snapshots are the frame's visible text (labels + card previews + banner/modal text), not pixels — stable across themes and font rasterization. On failure the harness writes a bundle: the full log, the last snapshot, the expected-vs-actual step, copies of `snippets.json`/`config.json`, the seed, and the exact command to reproduce. CI uploads `sim-report/` on failure. Headed mode adds PNGs to the same bundle.

### 7. Initial persona and journey set

Four personas, each a timing profile + decision weights over the shared intent vocabulary:

- **FirstRunExplorer** — seeded default library; searches, filters by category, copies a card, opens and cancels the editor. Verifies the "Copied" feedback and that nothing was mutated.
- **PowerOrganizer** — adds ~20 snippets across new and existing categories, edits bodies, drag-reorders (including in a filtered view), deletes one. Verifies `snippets.json` content and ordering after every mutation.
- **ErrorHandler** — journeys over the failure surfaces: corrupt `snippets.json` recovery (`.corrupt` rename + banner), save failure with a read-only data dir (banner, in-memory state intact), rejected reserved/blank category (form stays open).
- **ThemeHopper** — switches several themes and verifies `config.json` persists the selection across a simulated restart (drop app, rebuild from the same store dir).

### 8. Hybrid evaluation, honestly scoped

Browser automation / API orchestration / auth testing are N/A (no such layers). Workflow engines are overkill for one process; the journey DSL is a plain Rust builder, not YAML, so journeys get compile-time checking, refactoring support, and debugger access. AI agents are deferred (Non-Goals). The result is a hybrid: deterministic scripted journeys as the correctness gate, persona randomness for behavioral coverage within those journeys, headed mode for real-window confidence.

## Acceptance Criteria

1. `cargo test` runs the full journey suite headlessly; the suite (all personas, 13 journeys) completes in under 60 s on the CI runner.
2. CI (`windows-latest`) executes the suite on every push/PR and uploads `sim-report/` on failure; `cargo clippy --all-targets -- -D warnings` stays green.
3. Determinism test passes: two runs of the same journey with the same seed produce identical normalized event logs.
4. Isolation test passes: after the full suite, no file exists at the real `%APPDATA%\CopyIt` paths that did not exist before (verified by the harness guard never tripping plus an explicit test that journeys cannot construct `Store::open`).
5. A deliberately-broken build (meta-test: journey asserting a false expectation) produces a failure bundle containing the log, snapshot, seed, and repro command.
6. `cargo build --release` without `--features sim` is unchanged: no `sim` code in the binary (verified by the feature gate; size profile untouched).
7. Headed mode runs one journey locally against the real window and writes PNG screenshots into the report bundle.

## Risks / Trade-offs

- **Input fidelity.** Synthesized events may miss edge cases of real OS input (IME, double-click timing quirks, focus stealing). Mitigated by headed mode spot-checks and by testing through the same `RawInput` pipeline egui itself consumes — what differs from real input differs for every egui app equally.
- **Brittleness of text locators.** Renaming a button label breaks journeys. Accepted: the failure message names the missing label, and the fix is a one-line journey update — cheaper than pixel goldens. Theme/layout changes do not affect text locators.
- **Harness/app drift.** The seam guarantees the harness runs the shipped UI code, but app internals the harness pokes (store digests) can drift. Mitigated by keeping harness assertions on persisted files and visible text, not internal fields, wherever possible.
- **Runtime cost in CI.** Virtual-clock pumping is cheap; the 60 s budget is generous. If the journey library grows, journeys shard by persona.
- **Merge surface with `add-protected-snippets`.** Both touch `src/app.rs` but disjointly (this change relocates `update`'s body; that change edits logic inside it). Whichever lands second rebases mechanically. Vault journeys are follow-up work once protected cards exist; the scenario layer's custom-step hook is designed for that.
- **False confidence.** A green suite does not prove the app works on a real machine. Mitigated by keeping the manual smoke pass for releases (now much shorter) and headed mode for release candidates.

## Migration Plan

Purely additive: no data-model, persistence, or UI-behavior changes. Rollout order: (1) UI seam + harness skeleton with one smoke journey; (2) intents/locators + FirstRunExplorer; (3) remaining personas and the failure-surface journeys; (4) reporting/CI; (5) headed mode. Each step lands green. `AGENTS.md` is updated with the new module and rules when the harness lands; no user-facing migration.

## Open Questions

- Should headed mode also record an actual video (frame captures → gif/mp4), or are per-step PNGs enough for human review? (Lean: PNGs; video only if a real need appears.)
- When `add-protected-snippets` lands, which vault journeys join the required CI set (happy-path unlock + copy; wrong-password; censored-render) versus staying in the optional extended set?
- Do we want a nightly job running journeys with randomized seeds for broader behavioral coverage, while the PR gate stays on fixed seeds?
