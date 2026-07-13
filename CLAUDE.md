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
cargo test              # runs clean; no automated tests exist yet
```

## Architecture

Six modules in `src/`, with a strict separation of concerns:

- `main.rs` — eframe setup (1000x700 window, 560x400 min). Sets `windows_subsystem = "windows"` for release only, so release builds have no console.
- `app.rs` — the `CopyIt` struct (all app state) and the entire egui UI: top bar (search, category filter, theme selector, New button), responsive card grid, clipboard copy with transient "Copied" feedback, modal add/edit/delete editor, and drag-and-drop card reordering. UI helpers (`truncate_chars`, `preview_text`, `category_color`) live at the bottom.
- `model.rs` — `Snippet { id, title, category, body }`.
- `storage.rs` — JSON persistence. `data_dir()` resolves `%APPDATA%\CopyIt` (falls back to next-to-exe when `APPDATA` is unset, e.g. non-Windows dev). Handles `snippets.json` (library) and `config.json` (canonical categories + selected theme), one-time migration from legacy next-to-exe locations, and `normalize_category()` (title-cases and dedupes).
- `seed.rs` — default snippet library seeded on first launch.
- `theme.rs` — `Theme` enum and custom `egui::Visuals` for 37 color themes.

Persistence rule: keep UI in `app.rs`, data types in `model.rs`, persistence in `storage.rs`, themes in `theme.rs`. Saves happen automatically after every add/edit/delete/reorder.

## Critical gotchas

- **Do not add an inherent `save` method to `CopyIt`.** `eframe::App` already defines `save(&mut self, &mut dyn Storage)`; an inherent method would shadow it and break the build. Use `save_snippets` / `save_config` (as the code does).
- **Running CopyIt locks `target\release\copyit.exe` on Windows.** `cargo clean` / `cargo build --release` then fail with `Access is denied (os error 5)` or `LNK1104`. Close the app first — `Get-Process copyit | Stop-Process` if the window is hidden.
- **egui is pinned to 0.27.** Do not upgrade without checking for breaking API changes.
- **Card drag-and-drop** uses an `ui.interact` drag sensor on the whole card. Don't let the copy/edit buttons consume that drag area or clicks will conflict with drag initiation.

## Storage note

Data lives in `%APPDATA%\CopyIt\`, not next to the exe — deliberately, so debug (`cargo run`) and release read the same files and `cargo clean` / git checkouts can't destroy real data. Both `snippets.json` and `config.json` are plain, hand-editable JSON.
