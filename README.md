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
- Data stored as a plain, hand-editable `snippets.json` in a stable per-user folder (`%APPDATA%\CopyIt`), so it survives rebuilding, moving, or replacing the `.exe`.

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

   Run that `.exe` from wherever you like. On first launch it creates
   `snippets.json` in `%APPDATA%\CopyIt\`, pre-seeded with a few Git scripts
   and AI prompts. Edit, add, or delete freely — changes save automatically.

Debug builds show a console window; the `--release` build is a clean windowless
GUI app.

## Where's my data?

`snippets.json` lives in `%APPDATA%\CopyIt\` — a stable folder tied to your
Windows user account, not to wherever `copyit.exe` happens to be. That means
recompiling, moving the `.exe`, or running a fresh build all see the same
data. Back it up, sync it, or edit it by hand — it's just JSON. Your
canonical category list and chosen theme live in `config.json` beside it.
If you're upgrading from an older version that stored data next to the
`.exe`, CopyIt automatically picks that data up the first time you run the
new version.

```json
[
  { "id": 1, "title": "…", "category": "Git", "body": "…" }
]
```

## Notes

- Pinned to `eframe`/`egui` **0.27** for a stable, well-tested API surface.
- Release profile is tuned for a small binary and fast startup
  (`opt-level = "z"`, LTO, stripped).
