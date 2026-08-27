# Dependency audit notes

**Reviewed:** 2026-08-28 on Windows, Rust 1.93.1, `cargo-audit 0.22.1`

The desktop application is a Windows-only release. The lockfile still contains
all-platform dependency metadata, so the audit is recorded both unfiltered and
against `x86_64-pc-windows-msvc` where the tool permits target filtering.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo audit` in `quantdale/CopyIt-brwsr-ext/native-host` | PASS; no findings |
| `cargo audit` in this repository before justified ignore | one vulnerability plus five unmaintained/unsound warnings; details below |
| `cargo audit --target-os windows` before justified ignore | same webbrowser finding; cargo-audit does not remove that crate from the target graph |
| `cargo tree --target x86_64-pc-windows-msvc -i event-listener@5.4.1` | empty; the unsound event-listener chain is not in the Windows graph |
| `npm audit` and `npm audit --omit=dev` in the extension repository | PASS; zero vulnerabilities |

`cargo audit 0.22.1` does not accept Cargo's `--locked` flag. All build/test
gates use the committed `Cargo.lock`; the audit command is read-only and scans
that lockfile.

## Desktop findings and decisions

### RUSTSEC-2026-0257 — webbrowser 0.8.15

The advisory concerns Unix `BROWSER` environment-variable argument handling.
The crate reaches the desktop lockfile through `eframe 0.27.2` →
`egui-winit`, but CopyIt ships the Windows target only and does not use the
Unix browser-launch implementation. This is therefore not an exploitable path
in the shipped Windows binary.

The advisory is explicitly listed in `.cargo/audit.toml` with this rationale,
so the CI audit remains visible and reproducible without pretending the
unfiltered lockfile is clean. Revisit the exception when `eframe` upgrades its
`webbrowser` dependency; do not expand the ignore list without a target-tree
and source-path review.

### RUSTSEC-2026-0221 — event-listener 5.4.1

This is an unsoundness warning reached through the Unix accessibility chain
(`zbus`/`accesskit_unix`). The Windows-target tree is empty for
`event-listener@5.4.1`, and the default audit command reports it as an allowed
warning rather than an active vulnerability. It is not ignored silently.

### Unmaintained dependency warnings

The audit also reports `derivative 2.2.0`, `instant 0.1.13`, `paste 1.0.15`,
and `ttf-parser 0.25.1` as unmaintained. `derivative`, `instant`, and the
event-listener chain are Unix-only for the desktop target. `paste` and
`ttf-parser` remain in the Windows graph through the pinned egui/eframe stack,
but have no active vulnerability in this audit. They remain visible as
warnings and are candidates for removal when the desktop UI dependencies are
upgraded; no unrelated UI-major upgrade was folded into this campaign.

## Extension findings and decisions

The original npm audit reported six development-tool vulnerabilities through
Vite/esbuild, Vitest/vite-node, and `vite-plugin-static-copy`. The unused
static-copy plugin was removed; Vite, Vitest, and the coverage provider were
upgraded to patched exact versions. A fresh `npm ci`, `npm audit`, and
`npm audit --omit=dev` now report zero vulnerabilities.

The extension workflows use `.node-version` (`24.3.0`), the desktop and host
repositories use `rust-toolchain.toml` (`1.93.1`), and GitHub Actions references
are pinned to immutable commit SHAs. The coordinated desktop compatibility
commit is pinned in the extension CI workflow after the final desktop commit
is created.
