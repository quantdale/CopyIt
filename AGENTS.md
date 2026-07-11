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
  - Responsive card grid of snippets.
  - Copy-to-clipboard action and transient "Copied" feedback.
  - Modal editor for adding, editing, and deleting snippets.
  - Drag-and-drop reordering of snippet cards (pointer drag on a card body, not on its buttons).
- `src/model.rs` — Defines `Snippet { id, title, category, body }`.
- `src/storage.rs` — `data_dir()` resolves the stable `%APPDATA%\CopyIt` directory (falling back to next-to-the-exe if `APPDATA` isn't set, e.g. non-Windows dev/test). `data_path()`, `load()`, and `save()` handle `snippets.json`; `config_path()`, `load_config()`, and `save_config()` handle `config.json` (canonical categories and theme). `legacy_candidate_dirs()` lists old next-to-exe locations used for one-time migration. Also contains `normalize_category()` for title-casing category strings.
- `src/seed.rs` — Initial default snippets (Git helpers and reusable AI prompts).
- `src/theme.rs` — `Theme` enum and custom `egui::Visuals` for seven selectable themes.

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

The project currently has no automated tests. The standard Cargo test command runs cleanly:

```powershell
cargo test
```

If you add tests, place unit tests in the relevant `src/*.rs` file under `#[cfg(test)] mod tests`. Consider extracting pure helper functions (search/filter, serialization round-trips, text truncation, category normalization) for testability.

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

- Saves are automatic after every add, edit, delete, or drag-and-drop reorder.
- Categories are normalized to title-case (e.g., `git` and `GIT` both become `Git`) and stored as a sorted, deduplicated list in `config.json`.

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

## Common gotchas

- `CopyIt` implements `eframe::App`, which already defines a `save(&mut self, _storage: &mut dyn Storage)` method. Do not add an inherent method named `save` on `CopyIt`; it will shadow the trait method and break compilation. Use descriptive names such as `save_snippets` and `save_config` for application-level persistence, as the current code does.
- Card drag-and-drop is handled by an `ui.interact` drag sensor on the whole card. Be careful not to make the copy or edit buttons consume that drag area, or drag initiation will conflict with button clicks.
