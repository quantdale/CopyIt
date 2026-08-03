# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

CopyIt is a native Windows GUI app (Rust + `egui`/`eframe` 0.27) that stores scripts and AI prompts as copyable tiles. It compiles to a single `.exe` with no installer, WebView, or runtime dependencies.

`AGENTS.md` is the detailed agent guide — consult it for conventions, storage internals, and gotchas. This file is the quick orientation.

## Commands

```powershell
cargo build --release   # release exe -> target\release\copyit.exe (windowless GUI)
cargo run               # debug build; shows a console for println! debugging
cargo check
cargo clippy
cargo test              # unit tests in src/*.rs (layout, drag/reorder, storage, themes)
```

## Architecture

Nine modules in `src/`, with a strict separation of concerns:

- `main.rs` — eframe setup (1000x700 window, 560x400 min). Sets `windows_subsystem = "windows"` for release only, so release builds have no console.
- `app.rs` — the `CopyIt` struct (all app state) and the egui UI: top bar (search, category filter, theme selector, New button), responsive card grid, clipboard copy with transient "Copied" feedback, modal add/edit/delete editor, and drag-and-drop card reordering. UI helpers (`truncate_chars`, `preview_text`, `category_color`) live at the bottom.
  - Frame-rate hot paths are cached, not recomputed per repaint: `Derived` (per-snippet lowercase text + card preview) and `FilterCache` (the visible-card index list). The card grid also only lays out the rows in view.
- `editor.rs` — the add/edit modal's state (`Editor`), the clicks it can produce (`EditorResult`), and the pure `decide()` mapping clicks to an `EditorOutcome` the app applies. No egui here, so transitions are unit-testable.
- `grid.rs` — card grid geometry: `CARD_*` constants, `cols_for`, `grid_card_rect`, `visible_rows` (virtualization), gap/insertion-line math, and the `DragMachine` drag state machine.
- `model.rs` — `Snippet { id, title, category, body }`.
- `storage.rs` — low-level JSON persistence. `data_dir()` resolves `%APPDATA%\CopyIt` (falls back to next-to-exe when `APPDATA` is unset, e.g. non-Windows dev). `load`/`save` for `snippets.json`, `load_config`/`save_config` for `config.json`, atomic writes, corrupt-file tri-state, and the category helpers (`normalize_category`, `same_category`, `is_reserved_category`, `canonical_category`).
- `store.rs` — the persistence seam above `storage.rs`: owns the data paths (`Store::at` / `Store::open`), the one-time legacy migration, and the load/save calls. `app.rs` never touches paths.
- `seed.rs` — default snippet library seeded on first launch.
- `theme.rs` — `Theme` enum and custom `egui::Visuals` for 37 color themes.

Persistence rule: keep UI in `app.rs`, data types in `model.rs`, low-level IO + category rules in `storage.rs`, the path/migration seam in `store.rs`, grid geometry in `grid.rs`, editor transitions in `editor.rs`, themes in `theme.rs`. Saves happen automatically after every add/edit/delete/reorder.

## Performance rules

- **Every mutation of `snippets` must go through `CopyIt::snippets_changed()`** (not a bare `save_snippets()`). It rebuilds the `derived` cache so it stays index-aligned with `snippets`, bumps `generation` to invalidate the filter cache, and then saves. Skipping it leaves cards showing another snippet's preview and search matching stale text.
- **The card grid is virtualized.** Only the rows intersecting `ui.clip_rect()` (plus one row of overscan) are laid out; the rest are replaced with `add_space` of the same height. Card rects come from `grid::grid_card_rect` (computed from the grid origin), so drag-and-drop still sees the whole grid. If you change card size or spacing, change the `CARD_*` / `ROW_PITCH` / `GRID_*` constants in `grid.rs` — the renderer and the hit-testing both read them.
- **Don't call `ctx.set_visuals()` every frame.** It rebuilds a whole `egui::Visuals`; apply it at startup and when the theme selection changes. Use `Theme::name()` (`&'static str`) rather than `to_string()` in UI code.

## Critical gotchas

- **Do not add an inherent `save` method to `CopyIt`.** `eframe::App` already defines `save(&mut self, &mut dyn Storage)`; an inherent method would shadow it and break the build. Use `save_snippets` / `save_config` (as the code does).
- **Running CopyIt locks `target\release\copyit.exe` on Windows.** `cargo clean` / `cargo build --release` then fail with `Access is denied (os error 5)` or `LNK1104`. Close the app first — `Get-Process copyit | Stop-Process` if the window is hidden.
- **egui is pinned to 0.27.** Do not upgrade without checking for breaking API changes.
- **Card drag-and-drop** uses an `ui.interact` drag sensor on the whole card. Don't let the copy/edit buttons consume that drag area or clicks will conflict with drag initiation.
- **Rects gathered inside a `ScrollArea` are already in absolute screen coordinates** (the scroll offset is baked into the content `Ui`'s origin). Never re-translate them by `inner_rect.min - state.offset` — that double-shifts all drag/drop hit-testing. See `AGENTS.md` for the full note.
- **Data files are written atomically** via `storage::write_atomic`. Don't replace it with `fs::write`, and keep `load`'s `Missing` and `Corrupt` cases distinct — merging them makes seeded defaults silently overwrite a library that merely failed to parse.

## Storage note

Data lives in `%APPDATA%\CopyIt\`, not next to the exe — deliberately, so debug (`cargo run`) and release read the same files and `cargo clean` / git checkouts can't destroy real data. Both `snippets.json` and `config.json` are plain, hand-editable JSON.
