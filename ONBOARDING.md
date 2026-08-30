# Fresh-machine onboarding

This is the canonical bootstrap entry point for a new workstation or a fresh coding-agent environment. Complete this document before implementation work. The objective is a reproducible machine that can build, test, inspect, and operate this repository without rediscovering tooling mid-campaign.

## 1. Preflight rule

1. Clone the repository and enter its root.
2. Confirm the intended repository/branch and fetch current `origin/main`.
3. Read the repository control-plane documents before changing code: `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md`, `README.md`, `.agent/`, active OpenSpec state.
4. Install/verify the machine prerequisites below.
5. Enable the committed agent integrations and repository-local skills.
6. Restore dependencies from lockfiles/pins; do not casually upgrade them during bootstrap.
7. Run the baseline validation commands.
8. Only then begin a development campaign. If a prerequisite cannot be satisfied, record it as an environment blocker rather than weakening a gate.

Credentials, API keys, signing material, account logins, licensed models/assets, and other secrets are machine/user responsibilities. Never commit them.

## 2. Supported host and prerequisites

**Primary host:** Windows 10/11; native Rust desktop application.

**Required machine tools**
- Git
- Rust stable toolchain (README baseline 1.85+) with Cargo
- Windows SDK / MSVC build tools as required by the Rust Windows target
- PowerShell

**Task-dependent / optional tools**
- `cargo-audit` when performing dependency/security review


## 3. Agent setup

- Load repository instructions before acting. Prefer committed repository state over chat history.
- Repository-local skills: `goal`.
- Agent adapter/config directories present in this repository should be discovered and used in-place; do not duplicate them globally unless the harness cannot load repository-local configuration.
- Relevant committed agent surfaces: `.agent/`, `.agents/`, `.cargo/`, `.claude/`, `.kimi-code/`, `.opencode/`.
- MCP policy: No root `.mcp.json` is committed. The application and tests are native; do not add browser-oriented MCPs without a task-specific reason.
- Keep MCP/plugin authority narrow. Documentation/diagnostic MCPs are not permission to change architecture, bypass tests, or publish.
- Authentication for GitHub and coding-agent CLIs is configured separately on the machine. Never write tokens into tracked files.

## 4. Bootstrap

Run the repository's pinned bootstrap, not an improvised dependency upgrade:

```powershell
rustup show
cargo fetch --locked
cargo build --locked
```

The SQLite dependency is vendored through `rusqlite`'s bundled feature; no system SQLite install should be necessary.


## 5. Editor/LSP baseline

Use rust-analyzer with the repository toolchain. Keep rustfmt and Clippy available and treat warnings in validation as defects where the repo gate does.

The editor is optional; the language servers are not. Agents should have diagnostics/type information available before editing non-trivial code.

## 6. Baseline verification

```powershell
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --bin copyit sim_journeys -- --test-threads 1
cargo build --locked --release
```

A fresh machine is considered **development-ready** only when the applicable non-external gates above pass. Hardware/device/signing/account gates may remain explicitly blocked if the repository already classifies them that way.

## 7. Fresh-agent instruction

Use this exact operating rule when handing the repository to a new agent:

> Read `ONBOARDING.md` first. Set up every applicable prerequisite, repository-local skill, MCP/plugin, dependency, browser/device/runtime tool, and validation gate described there. Then read the repository's durable agent state and only start implementation after preflight is green or a genuine environment blocker is recorded. Do not replace pinned tooling, skip gates, or invent work to compensate for a missing machine capability.
