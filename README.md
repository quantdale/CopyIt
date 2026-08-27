# CopyIt

A fast, lightweight Windows desktop app to store your scripts and AI prompts as
copyable tiles. Built in Rust with `egui`/`eframe` — it compiles to a single
native `.exe` with no runtime, no WebView, and no installer.

## Features

- Tile/card grid: **title**, **category badge**, **preview**, and a **Copy button** (top-right of each card).
- One-click copy to the clipboard, with a brief "Copied" confirmation.
- Instant search across title, body, and category.
- Category filter dropdown.
- 37 selectable color themes (Dark, Light, Nord, Dracula, Solarized, Gruvbox, Catppuccin, Tokyo Night, One Dark/Light, Monokai, GitHub, Ayu, Rose Pine, Everforest, Material, Kanagawa, Night Owl, Zenburn, Synthwave '84, Cobalt2, Horizon, and more).
- Add / edit / delete snippets from inside the app.
- Password-protected snippets: tick **Protect this snippet** in the editor and the body is encrypted at rest — the card shows a masked hint until you unlock, and a vault lock/unlock control appears in the top bar.
- Data stored as plain, hand-editable JSON (`snippets.json`) in a stable per-user folder (`%APPDATA%\CopyIt`), so it survives rebuilding, moving, or replacing the `.exe` (protected bodies are ciphertext inside the same file).

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

Want a card to stay secret? Tick **Protect this snippet** in the Add/Edit dialog. Its body
is stored only as ciphertext (XChaCha20-Poly1305, key derived from your vault password with
Argon2id), and the card shows a short hint plus bullets instead of the text. Copying or
editing a protected card asks for the vault password once per session; the top bar shows
**Vault locked/unlocked** with a **Lock** button to drop the key from memory.

## Notes

- Pinned to `eframe`/`egui` **0.27** for a stable, well-tested API surface.
- Release profile is tuned for a small binary and fast startup
  (`opt-level = "z"`, LTO, stripped).
- **No password recovery.** Forget the vault password and protected bodies are unrecoverable. Titles and categories stay visible, and a protected card shows its first 5 body characters as a hint (bodies under 12 characters show none) — keep secrets out of titles.
- **Vault passwords must be at least 8 characters.** Shorter passwords are rejected at creation time.
- **Single instance.** Only one CopyIt window can run at a time — a second launch exits immediately.
- **Protected clipboard is best-effort (30 s).** Copying a protected snippet places plaintext on the clipboard immediately and CopyIt attempts to clear it after ~30 seconds — but only if the clipboard still holds CopyIt's own copy (proved by a sequence token, so newer clipboard content from you or another app is never clobbered). Clearing is best-effort: it can be delayed by clipboard contention and it cannot revoke copies already captured by third-party clipboard-history software.
- Unprotected snippets remain plaintext JSON, so the "don't store credentials" rule applies to *unprotected* cards. An older version of CopyIt reading a file with protected cards sees empty bodies — don't downgrade after protecting.

## For contributors

The UI is exercised by an in-process simulation suite that drives the real CopyIt UI
headlessly — the same code a user runs, pumped frame by frame with synthesized input. Run
the journeys headlessly (this is what CI does):

```powershell
cargo test --bin copyit sim_journeys -- --test-threads 1
```

To watch one journey in the real window (local only — requires the `sim` cargo feature):

```powershell
cargo run --features sim -- --simulate first-run-explorer --seed 42
```

Journeys never touch your real `%APPDATA%\CopyIt` data: each run gets a throwaway store in
the temp dir, and failures leave an inspectable report bundle under `sim-report/`.
