# CopyIt

A fast, lightweight Windows desktop app to store your scripts and AI prompts as
copyable tiles. Built in Rust with `egui`/`eframe` — it compiles to a single
native `.exe` with no runtime, no WebView, and no installer.

## Features

- Tile/card grid: **title**, **category badge**, **preview**, and a **Copy button** (top-right of each card).
- One-click copy to the clipboard, with a brief "Copied" confirmation.
- Instant search across title, body, and category.
- Category filter dropdown.
- Seven selectable themes (Dark, Light, Nord, Dracula, Solarized Dark, Gruvbox Dark, Catppuccin Mocha).
- Add / edit / delete snippets from inside the app.
- Data stored as a plain, hand-editable `snippets.json` **next to the .exe** (portable — copy the folder anywhere).

## Build (on Windows)

1. Install Rust (recent stable, 1.85+): https://rustup.rs
2. In this folder, run:

   ```powershell
   cargo build --release
   ```

3. Your executable is at:

   ```
   target\release\copyit.exe
   ```

   Copy that `.exe` anywhere you like. On first launch it creates `snippets.json`
   beside itself, pre-seeded with a few Git scripts and AI prompts. Edit, add, or
   delete freely — changes save automatically.

Debug builds show a console window; the `--release` build is a clean windowless
GUI app.

## Where's my data?

`snippets.json` sits in the same folder as `copyit.exe`. Back it up, sync it,
or edit it by hand — it's just JSON. Your canonical category list and chosen
theme live in `config.json` beside it.

```json
[
  { "id": 1, "title": "…", "category": "Git", "body": "…" }
]
```

## Notes

- Pinned to `eframe`/`egui` **0.27** for a stable, well-tested API surface.
- Release profile is tuned for a small binary and fast startup
  (`opt-level = "z"`, LTO, stripped).
