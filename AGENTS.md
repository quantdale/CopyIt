# CopyIt — Agent Guide

This file is a quick reference for AI agents working on the CopyIt project. It covers the project layout, build process, conventions, and anything else you need to know before modifying code.

## Project overview

CopyIt is a small Windows desktop app for storing scripts and AI prompts as copyable tiles. It is a native GUI application written in Rust using `egui`/`eframe`. It compiles to a single `.exe` with no installer, no WebView, and no runtime dependencies. User data lives in plain `snippets.json` and `config.json` files in a stable per-user directory (`%APPDATA%\CopyIt`), independent of wherever the `.exe` itself is run from.

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
    ├── model.rs        # Core data type: `Snippet`
    ├── storage.rs      # JSON persistence (`snippets.json` / `config.json` next to the .exe)
    ├── seed.rs         # Default snippet library shown on first launch
    └── theme.rs        # Selectable color themes and their `egui::Visuals`
```

### Module responsibilities

- `src/main.rs` — Sets up the native window (`1000x700` default, `560x400` minimum) and runs the egui event loop.
- `src/app.rs` — Contains the `CopyIt` app state and the entire UI:
  - Top bar with search, category filter, theme selector, and a New-snippet button.
  - Responsive card grid of snippets, rendered by `CopyIt::card_grid` (virtualized — see below).
  - Copy-to-clipboard action and transient "Copied" feedback.
  - Modal editor for adding, editing, and deleting snippets.
  - Drag-and-drop reordering of snippet cards (pointer drag on a card body, not on its buttons).
  - Two caches keep per-frame work off the hot path: `Derived` holds each snippet's lowercase title/body/category plus its collapsed card preview, and `FilterCache` memoizes the list of visible card indices for the current query, category filter, and library `generation`.
- `src/model.rs` — Defines `Snippet { id, title, category, body }`.
- `src/storage.rs` — `data_dir()` resolves the stable `%APPDATA%\CopyIt` directory (falling back to next-to-the-exe if `APPDATA` isn't set, e.g. non-Windows dev/test). `data_path()`, `load()`, and `save()` handle `snippets.json`; `config_path()`, `load_config()`, and `save_config()` handle `config.json` (canonical categories and theme). `legacy_candidate_dirs()` lists old next-to-exe locations used for one-time migration. Also contains the category helpers: `normalize_category()` (title-cases), `same_category()` (case-insensitive comparison), `is_reserved_category()` (rejects blank and the reserved `All`), and `canonical_category()` (maps unusable names to `UNCATEGORIZED`).
  - Both loaders return `Load<T>` — `Loaded` / `Missing` / `Corrupt` — rather than an `Option`. Keep those three cases distinct: collapsing `Corrupt` into `Missing` makes the app seed defaults over a file it merely failed to parse and destroy the user's library on the next save.
  - Both savers write through `write_atomic()` (temp file in the same directory → `sync_all` → rename). Never write a data file with a plain `fs::write`; a crash mid-write would truncate it.
- `src/seed.rs` — Initial default snippets (Git helpers and reusable AI prompts).
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
- Keep UI code in `app.rs`; keep data definitions in `model.rs`; keep persistence in `storage.rs`; keep theme definitions in `theme.rs`.
- Prefer `String` over `&str` for persisted fields (`Snippet` owns its data).
- Use descriptive names. UI helpers such as `truncate_chars`, `preview_text`, and `category_color` live at the bottom of `app.rs`.
- The egui API surface is pinned to 0.27; do not upgrade the dependency without checking for breaking API changes.

## Testing instructions

```powershell
cargo test
```

Unit tests live in the relevant `src/*.rs` file under `#[cfg(test)] mod tests` (`mod layout_tests` in `app.rs`). Coverage today: grid/gap geometry and the scroll-area coordinate space, grid virtualization (visible-row range, computed-vs-rendered card rects, and that a ten-times-larger library emits roughly the same paint work), the memoized filter (matching a fresh scan, and invalidating on query/category/library changes), preview collapsing and truncation, drag-and-drop reordering (including filtered views and a snippet that vanishes mid-drag), save-error reporting, atomic writes, corrupt-file recovery, category normalization, and theme name round-trips.

Tests that touch the save paths must point `path`/`config_path` at a throwaway temp directory — use the `test_app()` helper in `app.rs`, which does this. A test that leaves them as bare relative filenames writes `snippets.json` into the repository root.

`CI` runs `cargo clippy --all-targets -- -D warnings`, so any new clippy warning fails the build.

## Data and storage behavior

- Data files live in a stable per-user directory, `%APPDATA%\CopyIt\`, not next to the executable:
  - `snippets.json` stores the snippet library.
  - `config.json` stores the canonical category list and selected theme.
- This is deliberate: resolving storage relative to the running `.exe` meant `cargo run` (debug) and `cargo build --release` read/write different files, and `cargo clean` / git checkouts of the build folder could reset or destroy real data. `%APPDATA%\CopyIt` is immune to all of that.
- If `APPDATA` isn't set (non-Windows dev/test environments), the app falls back to the previous next-to-the-exe behavior.
- On first launch (or first launch after upgrading from an older version), the app checks legacy locations (next to the exe, `target/debug/`, `target/release/`, cwd) and migrates the first non-empty `snippets.json`/`config.json` it finds into the new location before falling back to the seeded defaults from `src/seed.rs`.
- Both files are plain, hand-editable JSON:

  ```json
  [
    { "id": 1, "title": "...", "category": "Git", "body": "..." }
  ]
  ```

- Saves are automatic after every add, edit, delete, or drag-and-drop reorder, and are atomic: the JSON is written to a temporary file in the same directory, flushed, and only then renamed over the real one. A crash, power loss, or full disk part-way through a save leaves the previous file intact instead of a truncated one.
- A data file that exists but doesn't parse is **not** treated as a first launch. It is renamed to `<name>.corrupt` (preserving the bytes for hand-recovery), the defaults are loaded, and the warning banner tells the user where the original went. An empty (zero-byte) file counts as absent, since it holds nothing to lose.
- Categories are normalized to title-case (e.g., `git` and `GIT` both become `Git`) and stored as a sorted, deduplicated list in `config.json`. Blank categories and the reserved `All` are mapped to `Uncategorized` on load, so a hand-edited `"category": ""` can't produce a badge that no filter entry selects.

## Security considerations

- No network access, no external secrets, and no encrypted storage.
- `snippets.json` and `config.json` are stored unencrypted in `%APPDATA%\CopyIt\`. Do not store sensitive credentials in snippets.
- `storage::save` and `storage::save_config` return `io::Result<()>`; callers in `app.rs` surface failures via the `save_error` field, shown as a warning banner in the top bar, instead of silently discarding them.
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

- **Derived snippet text is cached.** `Derived` stores the lowercase title/body/category the search matches against, plus the collapsed one-line card preview. Rebuilding it walks the whole library, so it happens only when the library changes.
- **All snippet mutations go through `CopyIt::snippets_changed()`.** It calls `rebuild_derived()` (which also bumps `generation`, invalidating the filter cache) and then `save_snippets()`. Never mutate `self.snippets` and call `save_snippets()` directly: `derived` is a parallel `Vec` indexed the same way as `snippets`, and letting it drift shows the wrong preview on a card and makes search match text that is no longer there. `card()` falls back to computing a preview if the two ever disagree, and a `debug_assert` catches it in debug builds.
- **The visible-card list is memoized.** `take_filtered()` recomputes the filtered index list only when the search text, the category filter, or `generation` changed; otherwise it hands back the same buffer. It *takes* the buffer (leaving the cache marked invalid) so the render loop can still borrow `self` mutably, and `restore_filtered()` puts it back at the end of the frame, reusing the allocation. If you add a second `take_filtered()` in one frame, restore it — the cache is deliberately treated as invalid while checked out, so the second call recomputes rather than reporting "no snippets match".
- **The card grid is virtualized.** `card_grid()` lays out only the rows intersecting `ui.clip_rect()`, plus one row of overscan, and reserves the height of the skipped rows above and below with `ui.add_space`, so the scrollbar and every card position are exactly what they would be if all rows were built. `visible_rows()` computes that range (and falls back to "all rows" on non-finite geometry).
- **Card rects are computed, not harvested.** Because rows can be skipped, `grid_card_rect()` derives each card's rect from the grid origin and the `CARD_*` / `CARD_SPACING` constants, and `card_grid` returns rects for *every* filtered card. Drag-and-drop can therefore still drop onto a gap that was never rendered. `layout_tests::computed_grid_rects_match_rendered_cards` pins the computed rects to what a real card gets, and a `debug_assert` in `card_grid` re-checks it per card. Changing card size, spacing, or grid margins means changing those constants — the renderer and the hit-testing read the same ones.
- **Each grid row gets an explicit widget id** via `ui.push_id(row_start, …)`. egui otherwise derives ids from a per-parent counter, which would make the ids inside a card depend on how many rows above the viewport were skipped, so a card's buttons would change identity as the user scrolls.
- **Visuals are applied only when the theme changes**, at startup and from the theme selector (which also requests one extra repaint so the already-painted top bar is redrawn with the new colors). `Theme::visuals()` builds a whole `egui::Visuals`; it is not something to do per frame. Use `Theme::name()` (`&'static str`) instead of `to_string()` in UI code, and avoid cloning app state (categories, the save-error message) just to satisfy the borrow checker inside a closure — borrow it, or record the decision in a local and apply it after the closure.

## Common gotchas

- `CopyIt` implements `eframe::App`, which already defines a `save(&mut self, _storage: &mut dyn Storage)` method. Do not add an inherent method named `save` on `CopyIt`; it will shadow the trait method and break compilation. Use descriptive names such as `save_snippets` and `save_config` for application-level persistence, as the current code does.
- Card drag-and-drop is handled by an `ui.interact` drag sensor on the whole card. Be careful not to make the copy or edit buttons consume that drag area, or drag initiation will conflict with button clicks.
- **Widget rects collected inside a `ScrollArea` are already absolute screen coordinates**, with the scroll offset baked in — `ScrollArea` places its content `Ui` at `inner_rect.min - state.offset`, so everything below inherits that origin. Do **not** translate them by `inner_rect.min - state.offset` to "convert them to screen space": that double-counts the origin and shifts all drop geometry by the height of the top bar, drifting further with every pixel scrolled. Compare them against `ctx.input(|i| i.pointer.interact_pos())` directly. `layout_tests::scroll_area_card_rects_are_already_in_screen_space` pins this down.
- The drag state stores only the dragged snippet's stable `id`, never the index its card had at drag start. `reorder()` resolves the index by id at drop time, so a library that changed mid-drag reorders the right card or nothing at all instead of moving the wrong one (or panicking in `Vec::remove`).
- **Running CopyIt locks the release executable on Windows.** A release build hardlinks `target\release\deps\copyit.exe` to `target\release\copyit.exe`. While CopyIt is running, Windows keeps that executable image locked, so `cargo clean` or `cargo build --release` may fail with `Access is denied. (os error 5)` or `LINK : fatal error LNK1104: cannot open file '...\target\release\deps\copyit.exe'`. Close any running CopyIt window before rebuilding. If the window is hidden or minimized, find the process in Task Manager or with `Get-Process copyit | Stop-Process` in PowerShell, then retry the build.
