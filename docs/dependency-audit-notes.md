# Dependency audit notes: Windows-target verdict

`cargo audit` reads `Cargo.lock`, which lists the **all-platform** dependency
graph. CopyIt is Windows-only (`#![windows_subsystem = "windows"]`,
`%APPDATA%` paths), so an advisory that only reaches the graph through a
Linux/Wayland- or Unix-only dependency chain is not actually present in the
shipped binary. This note records, crate by crate, whether each known
advisory's dependency actually survives when the graph is filtered to the
real target (`cargo tree --target x86_64-pc-windows-msvc -i <crate>`).

An earlier pass assumed all six known advisories were Linux-only and thus
irrelevant to the Windows build. That assumption was wrong for two of them
(`ttf-parser`, `paste`) — see the verdict below.

## Per-crate results

| Crate | Advisory | Kind | Windows target? |
|---|---|---|---|
| `quick-xml` 0.39.4 | RUSTSEC-2026-0194 | vulnerability (7.5 high) | drops out |
| `quick-xml` 0.39.4 | RUSTSEC-2026-0195 | vulnerability (7.5 high) | drops out |
| `derivative` 2.2.0 | RUSTSEC-2024-0388 | unmaintained (warning) | drops out |
| `instant` 0.1.13 | RUSTSEC-2024-0384 | unmaintained (warning) | drops out |
| `ttf-parser` 0.25.1 | RUSTSEC-2026-0192 | unmaintained (warning) | **persists** |
| `paste` 1.0.15 | RUSTSEC-2024-0436 | unmaintained (warning) | **persists** |

Raw output, `cargo tree --target x86_64-pc-windows-msvc -i <crate>`:

```
$ cargo tree --target x86_64-pc-windows-msvc -i quick-xml
warning: nothing to print.

$ cargo tree --target x86_64-pc-windows-msvc -i derivative
warning: nothing to print.

$ cargo tree --target x86_64-pc-windows-msvc -i instant
warning: nothing to print.

$ cargo tree --target x86_64-pc-windows-msvc -i ttf-parser
ttf-parser v0.25.1
└── owned_ttf_parser v0.25.1
    └── ab_glyph v0.2.32
        └── epaint v0.27.2
            └── egui v0.27.2
                ├── copyit v0.1.0 (D:\Documents\tryPython\CopyIt)
                ├── eframe v0.27.2
                │   └── copyit v0.1.0 (D:\Documents\tryPython\CopyIt)
                ├── egui-winit v0.27.2
                │   └── eframe v0.27.2 (*)
                └── egui_glow v0.27.2
                    └── eframe v0.27.2 (*)

$ cargo tree --target x86_64-pc-windows-msvc -i paste
paste v1.0.15 (proc-macro)
└── accesskit_windows v0.15.1
    └── accesskit_winit v0.16.1
        └── egui-winit v0.27.2
            └── eframe v0.27.2
                └── copyit v0.1.0 (D:\Documents\tryPython\CopyIt)
```

## Verdict

`quick-xml`, `derivative`, and `instant` are only reachable through
Linux/Wayland-only (`wayland-scanner`, `smithay-client-toolkit`) or Unix-only
(`zbus` → `accesskit_unix`) dependency chains, and correctly drop out when
the graph is filtered to `x86_64-pc-windows-msvc`. CopyIt's own `src/` also
parses no XML, so the two `quick-xml` vulnerabilities carry no exposure here
regardless of platform.

**`ttf-parser` (RUSTSEC-2026-0192) and `paste` (RUSTSEC-2024-0436) do reach
the real Windows dependency graph**, correcting the original audit's
Linux-only assumption: `ttf-parser` comes in through the core egui text
rendering path (`epaint → ab_glyph → owned_ttf_parser → ttf-parser`), and
`paste` comes in through Windows accessibility support
(`eframe → egui-winit → accesskit_winit → accesskit_windows → paste`).

Both are *unmaintained-crate* warnings, not active vulnerabilities — a
maintenance-risk signal, not an exploitable issue today. They aren't ignored
in `.cargo/audit.toml` (see policy there); they stay visible in `cargo audit`
output, and no action is needed beyond that visibility. Both chains run
through `egui`/`eframe`/`accesskit`, so they're expected to clear naturally
whenever those upstream crates bump their own dependencies.

## A second finding: the ignore-list file was in the wrong place

Separately from the persistence question above: a `.cargo/audit.toml` written
at the **repo root** (as plain `audit.toml`) is silently never read by
`cargo-audit` — it only auto-discovers `.cargo/audit.toml`. This was caught
by actually running `cargo audit` against the file (rather than trusting it
unverified): with the ignore list at the repo root, `cargo audit` still
reported `error: 2 vulnerabilities found!` for the two quick-xml IDs despite
them being listed; moved to `.cargo/audit.toml`, the same list correctly
suppressed them and `cargo audit` exited 0. See `.cargo/audit.toml` for the
corrected file and policy.
