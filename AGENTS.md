# CopyIt — Agent Guide

This file is a quick reference for AI agents working on the CopyIt project. It covers the project layout, build process, conventions, and anything else you need to know before modifying code.

## Project overview

CopyIt is a small Windows desktop app for storing scripts and AI prompts as copyable tiles. It is a native GUI application written in Rust using `egui`/`eframe`. It compiles to a single `.exe` with no installer, no WebView, and no runtime dependencies. User data lives in `snippets.json` and `config.json` files in a stable per-user directory (`%APPDATA%\CopyIt`), independent of wherever the `.exe` itself is run from. Snippets are stored as plain JSON; password-protected cards keep their body only as ciphertext (see `src/vault.rs`).

## Technology stack

- **Language:** Rust, edition 2021, recent stable toolchain.
- **GUI framework:** `egui` / `eframe` 0.27.
- **Serialization:** `serde` + `serde_json`.
- **Build tool:** Cargo.
- **Target platform:** Windows (the release build hides the console window; debug builds keep it).

## Project structure

```text
.
├── Cargo.toml          # Package manifest and release profile
├── Cargo.lock          # Dependency lockfile
├── README.md           # End-user documentation
├── AGENTS.md           # This file
└── src/
    ├── main.rs         # Application entry point; wires eframe to CopyIt
    ├── app.rs          # Main application state and egui UI implementation
    ├── editor.rs       # Add/edit modal: state, constructors, and the transition decision
    ├── grid.rs         # Card grid geometry, virtualization, insertion lines, drag machine
    ├── model.rs        # Core data type: `Snippet`
    ├── storage.rs      # Low-level JSON IO (`snippets.json` / `config.json`), category helpers
    ├── store.rs        # Persistence seam: paths, legacy migration, load/save
    ├── seed.rs         # Default snippet library shown on first launch
    ├── vault.rs        # Protected snippets: vault state, Argon2id KDF, XChaCha20-Poly1305
    ├── sim/            # In-process user simulation: harness, personas, journeys (test/sim)
    └── theme.rs        # Selectable color themes and their `egui::Visuals`
```

### Module responsibilities

- `src/main.rs` — Sets up the native window (`1000x700` default, `560x400` minimum) and runs the egui event loop. Under `feature = "sim"` it intercepts `--simulate <journey> [--seed N]` and runs the headed simulation driver instead.
- `src/app.rs` — Contains the `CopyIt` app state and the entire UI. All of it lives in `CopyIt::ui(ctx)`, driven both by `eframe::App::update` (a one-line delegation) and by the simulation harness — see User simulation below.
  - Top bar with search, category filter, theme selector, a New-snippet button, and — once any snippet is protected — the vault lock indicator with a **Lock** button while the vault is unlocked.
  - Responsive card grid of snippets, rendered by `CopyIt::card_grid` (virtualized — see below); the geometry, virtualization math, and drag machine live in `grid.rs`.
  - Copy-to-clipboard action and transient "Copied" feedback; protected snippets are decrypted on copy, and only while the vault is unlocked.
  - Modal editor for adding, editing, and deleting snippets, including the "Protect this snippet" checkbox; the editor's state and its button-click → outcome transition live in `editor.rs`.
  - Vault prompt modal (unlock, or create-vault on the first-ever protect) gates copy/edit of protected cards; the suspended action resumes once the vault is available (`run_pending_vault_action`).
  - Drag-and-drop reordering of snippet cards (pointer drag on a card body, not on its buttons), driven by `grid::DragMachine`.
  - Two caches keep per-frame work off the hot path: `Derived` holds each snippet's lowercase title/body/category plus its collapsed card preview, and `FilterCache` memoizes the list of visible card indices for the current query, category filter, and library `generation`.
- `src/editor.rs` — The snippet add/edit modal's state (`Editor`, including the `protect` checkbox), the button clicks it can produce (`EditorResult`), the pure `decide()` that turns a click plus the window's open/close flag into an `EditorOutcome`, and `card_action_requires_vault()` — the gate that decides whether a protected card's copy/edit must wait behind the vault prompt. All editor transitions are testable without a UI context.
- `src/grid.rs` — Everything a maintainer must touch to change the grid: the `CARD_*` / `CARD_SPACING` / `ROW_PITCH` / `GRID_TOP_SPACE` / `GRID_MARGIN_X` constants, `cols_for()`, `grid_card_rect()`, `visible_rows()`, the gap/insertion-line math (`gap_point`, `nearest_gap`, `draw_insertion_line`), and the `DragMachine` state machine (armed on press, dragging past the 4px threshold, consumed by `release()` against a `DragContext`).
- `src/model.rs` — Defines `Snippet { id, title, category, body, protection }`; `Protection { hint, nonce, ciphertext }` is the on-disk form of a protected body (see `crate::vault`).
- `src/storage.rs` — Low-level JSON IO: `data_dir()` resolves the stable `%APPDATA%\CopyIt` directory (falling back to next-to-the-exe if `APPDATA` isn't set, e.g. non-Windows dev/test), plus the category helpers `normalize_category()` (title-cases), `same_category()` (case-insensitive comparison), `is_reserved_category()` (rejects blank and the reserved `All`), `canonical_category()` (maps unusable names to `UNCATEGORIZED`), and `Config` (canonical categories, selected theme, and the optional `vault` metadata — the KDF salt and canary). Path construction is `store.rs`'s job, not this file's.
  - Both loaders return `Load<T>` — `Loaded` / `Missing` / `Corrupt` — rather than an `Option`. Keep those three cases distinct: collapsing `Corrupt` into `Missing` makes the app seed defaults over a file it merely failed to parse and destroy the user's library on the next save.
  - Both savers write through `write_atomic()` (temp file in the same directory → `sync_all` → rename). Never write a data file with a plain `fs::write`; a crash mid-write would truncate it.
- `src/store.rs` — The persistence seam: owns `snippets_path` / `config_path` (`Store::at` / `Store::open`), the one-time legacy migration (`migrate_legacy()`), and the load/save calls (`load_snippets`, `load_config`, `save_snippets`, `save_config`). `app.rs` never touches paths or migration rules.
- `src/seed.rs` — Initial default snippets (Git helpers and reusable AI prompts).
- `src/vault.rs` — Pure crypto + vault state behind protected snippets, testable without a UI: Argon2id key derivation (m = 19 MiB, t = 2, p = 1), XChaCha20-Poly1305 AEAD encrypt/decrypt with fresh nonces, the canary that verifies a candidate password, `VaultState` session lock/unlock (the derived key is memory-only and starts `Locked` on every launch), `encrypt_body` / `decrypt_body`, and the hint / masked-preview helpers. Everything returns `Result` — no panics, no I/O.
- `src/sim/` — The in-process user simulation, compiled only under `cfg(any(test, feature = "sim"))` (a default or release build contains none of it):
  - `harness.rs` — `SimApp` wraps the real `CopyIt` UI behind a bare `egui::Context` and pumps frames through `egui::Context::run` with synthesized `RawInput` events (pointer moves/buttons, keys, text) on a virtual clock and a seeded PRNG; interaction targets are resolved by scanning the tessellated frame for visible text (`visible_texts`, `locate`, `field_rect`, `click_text`, `type_into`, `drag_card`, `expect_*`).
  - `persona.rs` — Deterministic behavioral timing profiles (`Persona`): think time, per-character typing cadence, typo probability — all randomness draws from the run's seeded PRNG only.
  - `journey.rs` — The journey DSL (persona + fixtures + `Step`s) and the journey library (`first-run-explorer`, `power-organizer-*`, `error-*`, `theme-hopper`); the `Custom` step closure is the extension hook for future features. The `sim_journeys_*` test suite (one test per journey, plus determinism, isolation, and meta tests) lives here too.
  - `report.rs` — Run reporting under `sim-report/` (gitignored): the JSONL event log, per-step visible-text snapshots, and the failure bundle (failure detail, copies of the run's data files, seed, exact repro command); headed mode's PNG writer.
  - `cli.rs` — Headed `--simulate <journey> [--seed N]` mode (feature `sim` only): runs a journey against the real eframe window and saves a PNG screenshot into the report bundle. Local verification only, never in CI.
- `src/theme.rs` — `Theme` enum and custom `egui::Visuals` for 37 selectable themes.

## Build and run commands

Build a release binary:

```powershell
cargo build --release
```

The executable is produced at:

```text
target\release\copyit.exe
```

Run a debug build (shows a console window, useful for `println!` debugging):

```powershell
cargo run
```

Check and lint without building:

```powershell
cargo check
cargo clippy
```

## Code style guidelines

- Follow idiomatic Rust 2021.
- Keep UI code in `app.rs`; keep data definitions in `model.rs`; keep low-level persistence in `storage.rs`; keep the persistence seam in `store.rs`; keep grid geometry in `grid.rs`; keep editor transitions in `editor.rs`; keep theme definitions in `theme.rs`; keep vault crypto in `vault.rs`; keep simulation code in `src/sim/`.
- Prefer `String` over `&str` for persisted fields (`Snippet` owns its data).
- Use descriptive names. UI helpers such as `truncate_chars`, `preview_text`, and `category_color` live at the bottom of `app.rs`.
- The egui API surface is pinned to 0.27; do not upgrade the dependency without checking for breaking API changes.

## Testing instructions

```powershell
cargo test
```

Unit tests live in the relevant `src/*.rs` file under `#[cfg(test)] mod tests` (`mod layout_tests` in `app.rs`). Coverage today: grid/gap geometry and the scroll-area coordinate space (in `grid.rs` and `layout_tests`), grid virtualization (visible-row range, computed-vs-rendered card rects, and that a ten-times-larger library emits roughly the same paint work), the drag state machine (`grid.rs`), the memoized filter (matching a fresh scan, and invalidating on query/category/library changes), editor transition decisions (`editor.rs`), store round-trips and corrupt-file reporting (`store.rs`), preview collapsing and truncation, drag-and-drop reordering (including filtered views and a snippet that vanishes mid-drag), save-error reporting, atomic writes, corrupt-file recovery, category normalization, theme name round-trips, and the simulation journeys (`sim_journeys_*` in `src/sim/journey.rs`; see the User simulation section).

Tests that touch the save paths must point the app's data files at a throwaway temp directory — use the `test_app()` helper in `app.rs`, which builds a `Store::at(temp_dir)` instead of the real `%APPDATA%` location. A test that leaves them as bare relative filenames writes `snippets.json` into the repository root.

`CI` runs `cargo clippy --all-targets -- -D warnings`, so any new clippy warning fails the build.

## User simulation

The app drives its own UI headlessly: `src/sim/` wraps the real `CopyIt` UI behind a bare `egui::Context` and pumps frames through `egui::Context::run` with synthesized input events, so journeys exercise the shipped UI code — not a mock. Everything is compiled only under `cfg(any(test, feature = "sim"))`; a default build or the release binary contains none of it.

- **The UI seam.** All of the UI lives in `CopyIt::ui(ctx)`, and `eframe::App::update` is a one-line delegation to it. `CopyIt::from_store(store, ctx)` is the single constructor shared by `new` (production), the tests, and the harness; unlike `new`, it never runs legacy migration.
- **Isolation rule.** Journeys never touch the real store. The harness builds apps only via `CopyIt::from_store(Store::at(<temp>/copyit-sim/<pid>/<run>))` and never calls `Store::open` / `migrate_legacy`. `SimApp::build` refuses — before the first frame, as a runtime check — any store whose data files don't live under that per-run temp directory.
- **Determinism.** Time advances only through `RawInput.time` and all randomness comes from one seedable `StdRng`, so the same journey + persona + seed produces a byte-identical event stream; a failure reproduces by rerunning with the recorded `--seed`.
- **`sim` feature / headed mode.** `cargo run --features sim -- --simulate <journey> [--seed N]` runs a journey against the real eframe window (local verification only — the isolation guard still applies) and writes a PNG screenshot into the report bundle.
- **CI.** The headless suite runs serially as `cargo test --bin copyit sim_journeys -- --test-threads 1` (deterministic timing, must stay under 60 s), and `sim-report/` is uploaded as an artifact when the job fails.

## Data and storage behavior

- Data files live in a stable per-user directory, `%APPDATA%\CopyIt\`, not next to the executable:
  - `snippets.json` stores the snippet library.
  - `config.json` stores the canonical category list, the selected theme, and the vault metadata (base64 KDF salt + canary) once any snippet has been protected.
- This is deliberate: resolving storage relative to the running `.exe` meant `cargo run` (debug) and `cargo build --release` read/write different files, and `cargo clean` / git checkouts of the build folder could reset or destroy real data. `%APPDATA%\CopyIt` is immune to all of that.
- If `APPDATA` isn't set (non-Windows dev/test environments), the app falls back to the previous next-to-the-exe behavior.
- On first launch (or first launch after upgrading from an older version), the app checks legacy locations (next to the exe, `target/debug/`, `target/release/`, cwd) and migrates the first non-empty `snippets.json`/`config.json` it finds into the new location before falling back to the seeded defaults from `src/seed.rs`. The migration write is atomic (same temp-file-then-rename path as every other data write), so a crash mid-migration can't leave a truncated stable file behind.
- Both files are plain, hand-editable JSON:

  ```json
  [
    { "id": 1, "title": "...", "category": "Git", "body": "..." }
  ]
  ```

- A protected snippet stores an empty `body` plus a `protection` block (`hint`, `nonce`, `ciphertext`, base64) instead; the vault's salt and canary live in `config.json` under `vault`. Both new fields are optional, so files written by older versions load unchanged — but an older CopyIt reading a file with protected cards sees empty bodies. Don't downgrade after protecting.
- Saves are automatic after every add, edit, delete, or drag-and-drop reorder, and are atomic: the JSON is written to a temporary file in the same directory, flushed, and only then renamed over the real one. A crash, power loss, or full disk part-way through a save leaves the previous file intact instead of a truncated one.
- A data file that exists but doesn't parse is **not** treated as a first launch. It is renamed aside as `<name>.corrupt` (or `.corrupt.1`, `.corrupt.2`, … if a previous backup already exists, so an old backup is never overwritten) preserving the bytes for hand-recovery, the defaults are loaded, and the warning banner tells the user where the original went. If the backup rename fails (e.g. the file is locked), the corrupt file is left **in place** and the defaults are **not** written over it — the app refuses to destroy the only remaining copy of the user's data. An empty (zero-byte) file counts as absent, since it holds nothing to lose.
- Categories are normalized to title-case (e.g., `git` and `GIT` both become `Git`) and stored as a sorted, deduplicated list in `config.json`. On load the stored list is sanitized (normalized, deduplicated case-insensitively, reserved names dropped), so a hand-edited `config.json` can't smuggle an entry that collides with the reserved `All` filter sentinel. Blank categories and the reserved `All` are mapped to `Uncategorized` on load, so a hand-edited `"category": ""` can't produce a badge that no filter entry selects. Snippet ids are deduplicated on load (later duplicates get fresh ids) and the "next id" counter is overflow-safe, hardening the app against hand-edited JSON.

## Security considerations

- No network access and no secrets sent anywhere. Protected snippets are encrypted at rest: XChaCha20-Poly1305 AEAD under a key derived from the vault password with Argon2id. The password is never stored — a candidate password either decrypts the canary or fails, so a wrong guess (or a corrupt canary) can't unlock anything.
- Unprotected snippets remain plaintext JSON in `%APPDATA%\CopyIt\`. The old warning still applies to them: do not store sensitive credentials in *unprotected* cards — tick "Protect this snippet" instead.
- **No password recovery.** A forgotten vault password means the protected bodies are unrecoverable, by design.
- Documented leaks: the hint (a card's first 5 body characters, only when the body is ≥ 12 chars long) and the cleartext metadata (title/category) are visible without unlocking — keep secrets out of titles.
- Downgrade caveat: an older CopyIt reading a file with protected cards sees empty bodies (see Data and storage behavior).
- `storage::save` and `storage::save_config` return `io::Result<()>`; callers in `app.rs` surface failures via the `save_error: Vec<String>` field, shown as a warning banner in the top bar, instead of silently discarding them.
- Vault passwords must be at least 8 characters (`MIN_VAULT_PASSWORD_LEN`). `VaultError::WeakPassword` is returned for shorter passwords.
- `CopyIt::new()` acquires an instance lock (`file.try_lock()`) in the data directory. A second CopyIt instance exits immediately. `CopyIt::from_store()` does not acquire the lock (used by tests and the sim harness).
- `storage::sweep_stale_tmp()` deletes stale `*.tmp` files in the data directory at startup to clean up after crashed writes.
- Clipboard content is set through egui's `output_mut(|o| o.copied_text = text)`. It stays in the system clipboard until overwritten by something else.

## Deployment / distribution

- The recommended distribution artifact is the single `target\release\copyit.exe`.
- The release profile is tuned for size and fast startup:
  - `opt-level = "z"`
  - `lto = true`
  - `codegen-units = 1`
  - `panic = "abort"`
  - `strip = true`
- No installer or packaging step is currently provided. Distribute the `.exe` along with a note that `snippets.json` and `config.json` will be created on first run.

## Release console behavior

`src/main.rs` sets `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`, so `--release` builds launch without a console window. Debug builds retain the console for troubleshooting.

## Performance model

The UI is repainted on every mouse move, so anything done inside `update()` runs dozens of times a second. The costly work is therefore cached or skipped:

- **Derived snippet text is cached.** `Derived` stores the lowercase title/body/category the search matches against, plus the collapsed one-line card preview. Rebuilding it walks the whole library, so it happens only when the library changes. A protected snippet's body never enters the cache — its `body_lower` is empty (search can't match secrets) and its preview is the masked hint, even while the vault is unlocked.
- **All snippet mutations go through `CopyIt::snippets_changed()`.** It calls `rebuild_derived()` (which also bumps `generation`, invalidating the filter cache) and then `save_snippets()`. Never mutate `self.snippets` and call `save_snippets()` directly: `derived` is a parallel `Vec` indexed the same way as `snippets`, and letting it drift shows the wrong preview on a card and makes search match text that is no longer there. `card()` falls back to computing a preview if the two ever disagree, and a `debug_assert` catches it in debug builds.
- **The visible-card list is memoized.** `take_filtered()` recomputes the filtered index list only when the search text, the category filter, or `generation` changed; otherwise it hands back the same buffer. It *takes* the buffer (leaving the cache marked invalid) so the render loop can still borrow `self` mutably, and `restore_filtered()` puts it back at the end of the frame, reusing the allocation. If you add a second `take_filtered()` in one frame, restore it — the cache is deliberately treated as invalid while checked out, so the second call recomputes rather than reporting "no snippets match".
- **The card grid is virtualized.** `card_grid()` lays out only the rows intersecting `ui.clip_rect()`, plus one row of overscan, and reserves the height of the skipped rows above and below with `ui.add_space`, so the scrollbar and every card position are exactly what they would be if all rows were built. `grid::visible_rows()` computes that range (and falls back to "all rows" on non-finite geometry).
- **Card rects are computed, not harvested.** Because rows can be skipped, `grid::grid_card_rect()` derives each card's rect from the grid origin and the `CARD_*` / `CARD_SPACING` constants, and `card_grid` returns rects for *every* filtered card. Drag-and-drop can therefore still drop onto a gap that was never rendered. `layout_tests::computed_grid_rects_match_rendered_cards` pins the computed rects to what a real card gets, and a `debug_assert` in `card_grid` re-checks it per card. Changing card size, spacing, or grid margins means changing the constants in `grid.rs` — the renderer and the hit-testing read the same ones.
- **Each grid row gets an explicit widget id** via `ui.push_id(row_start, …)`. egui otherwise derives ids from a per-parent counter, which would make the ids inside a card depend on how many rows above the viewport were skipped, so a card's buttons would change identity as the user scrolls.
- **Visuals are applied only when the theme changes**, at startup and from the theme selector (which also requests one extra repaint so the already-painted top bar is redrawn with the new colors). `Theme::visuals()` builds a whole `egui::Visuals`; it is not something to do per frame. Use `Theme::name()` (`&'static str`) instead of `to_string()` in UI code, and avoid cloning app state (categories, the save-error message) just to satisfy the borrow checker inside a closure — borrow it, or record the decision in a local and apply it after the closure.

## Common gotchas

- `CopyIt` implements `eframe::App`, which already defines a `save(&mut self, _storage: &mut dyn Storage)` method. Do not add an inherent method named `save` on `CopyIt`; it will shadow the trait method and break compilation. Use descriptive names such as `save_snippets` and `save_config` for application-level persistence, as the current code does.
- Card drag-and-drop is handled by an `ui.interact` drag sensor on the whole card. Be careful not to make the copy or edit buttons consume that drag area, or drag initiation will conflict with button clicks.
- **Widget rects collected inside a `ScrollArea` are already absolute screen coordinates**, with the scroll offset baked in — `ScrollArea` places its content `Ui` at `inner_rect.min - state.offset`, so everything below inherits that origin. Do **not** translate them by `inner_rect.min - state.offset` to "convert them to screen space": that double-counts the origin and shifts all drop geometry by the height of the top bar, drifting further with every pixel scrolled. Compare them against `ctx.input(|i| i.pointer.interact_pos())` directly. `layout_tests::scroll_area_card_rects_are_already_in_screen_space` pins this down.
- The drag state (`grid::DragMachine`) stores only the dragged snippet's stable `id`, never the index its card had at drag start. `reorder()` resolves the index by id at drop time, so a library that changed mid-drag reorders the right card or nothing at all instead of moving the wrong one (or panicking in `Vec::remove`).
- **Running CopyIt locks the release executable on Windows.** A release build hardlinks `target\release\deps\copyit.exe` to `target\release\copyit.exe`. While CopyIt is running, Windows keeps that executable image locked, so `cargo clean` or `cargo build --release` may fail with `Access is denied. (os error 5)` or `LINK : fatal error LNK1104: cannot open file '...\target\release\deps\copyit.exe'`. Close any running CopyIt window before rebuilding. If the window is hidden or minimized, find the process in Task Manager or with `Get-Process copyit | Stop-Process` in PowerShell, then retry the build.
