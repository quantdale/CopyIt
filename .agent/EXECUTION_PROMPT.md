# CopyIt — Next Campaign: Data Integrity, Failure Transparency & Vault Lifecycle Hardening

Status: ACTIVE
Planned-From: 7b8807e0439257fe468a3d65ef91c4da4c973bdb
Planned-At: 2026-08-25T18:29:00+08:00
Target-Branch: main
Campaign-ID: hardening-2026-08-25-data-vault-lifecycle

## Mission

Harden CopyIt so it never gives the user false confidence about the safety, persistence, or lifetime of their data. Close the currently reachable corrupt-file overwrite regression first, then harden the adjacent storage, migration, settings-persistence, and protected-clipboard/vault execution paths. Preserve the existing native Windows single-binary architecture, JSON compatibility, simulation determinism, performance model, and all already-shipped user workflows.

This is an implementation campaign, not another broad rewrite. Work from the current code and existing seams (`storage.rs`, `store.rs`, `vault.rs`, `editor.rs`, `grid.rs`, `src/sim/`, and `CopyIt` coordination in `app.rs`). Do not re-implement completed OpenSpec work or replace working architecture merely to make it look cleaner.

## Why this campaign is next

The previous feature tracks are complete, there are no current open implementation issues/PRs to finish, and recent commits already addressed broad correctness/security/performance findings. A fresh audit of current `main` nevertheless exposes several concrete whole-system gaps, including one data-loss path that contradicts the repository's documented invariant that an unreadable file whose backup fails must never be overwritten.

### P0 — corrupt originals can still be overwritten after backup failure

`CopyIt::from_store` correctly avoids the *initial* seeded write when `backup_corrupt` fails, but subsequent mutation paths still call normal persistence unconditionally:

- `snippets_changed()` -> `save_snippets()` writes `snippets.json` after add/edit/delete/reorder.
- theme and category changes eventually call `save_config()`.
- `refuse_overwrite` exists but is effectively dead and does not guard either file.

Therefore a corrupt `snippets.json` or `config.json` whose backup rename failed can remain safe only until the user performs a later normal action; that action can replace the only remaining original bytes. This is the highest-priority defect and must be fixed before any lower-priority work.

### P0/P1 — config persistence failures are still silently swallowed

`save_config()` returns `Result`, but ordinary category registration and theme selection discard it with `let _ = self.save_config()`. The UI can therefore show a changed theme/category while the setting was never persisted and no durable warning explains that it will revert after restart. This violates the documented rule that persistence failures are surfaced rather than silently discarded.

### P1 — the 256 MiB load cap is checked only after the whole file is read

`storage::load_json` calls `std::fs::read_to_string(path)` before testing `data.len() > MAX_DATA_FILE_BYTES`. A huge file therefore still allocates/reads the full file before being rejected. The intended defensive bound is not actually a bound on memory or startup work.

### P1 — legacy migration failures are not represented to the app

`Store::migrate_legacy()` returns no outcome. `migrate_path_from` silently continues after read/write failures. If valid legacy data is present but cannot be copied to the stable `%APPDATA%\CopyIt` location, startup can proceed as though no migration source existed and seed defaults. The legacy source is not destroyed, but the app can present a misleading first-run state rather than a clear recovery state.

Directory-creation failures are similarly ignored by `storage::data_dir()` until a later operation happens to fail.

### P1 — protected clipboard plaintext has an unbounded lifetime

Protected snippets are encrypted at rest, but the primary action decrypts the body and places plaintext on the Windows clipboard. The repository explicitly documents that clipboard content remains there until something else overwrites it. Protected-copy semantics should reduce that exposure without ever deleting clipboard content that the user copied after CopyIt.

### P2 — vault KDF work blocks the UI and an unfinished worker field already exists

Vault create/unlock currently performs Argon2id key derivation synchronously from the UI event path. `CopyIt` already carries an unused `vault_work` receiver, indicating an incomplete attempt at moving this work off the frame loop. Complete this properly (or replace the dead field with a better bounded state machine) while preserving deterministic simulation.

### Dependency posture — monitor, do not derail this campaign

The repository intentionally pins egui/eframe 0.27 and documents Windows-reachable unmaintained dependency warnings (`ttf-parser`, `paste`) that are not active vulnerabilities. The framework is now many releases behind, so a blind upgrade is a migration project, not a quick hardening patch. Re-run the audit and target-graph checks during this campaign, but do not perform a broad egui/eframe major upgrade unless it is demonstrably bounded, necessary to fix a real vulnerability, and every compatibility/simulation gate passes. Otherwise record the verified current posture and leave the framework migration for a dedicated future campaign.

## Preserved behavior and invariants

Do not regress any of these while fixing the campaign:

- Native Windows Rust/egui app; no WebView, server, analytics, or network requirement.
- Single portable release `.exe`; no installer requirement introduced.
- Existing `%APPDATA%\CopyIt` stable storage and one-time legacy migration intent.
- `snippets.json` and `config.json` remain hand-editable JSON and older files continue loading.
- Protected snippet bodies remain ciphertext at rest under XChaCha20-Poly1305 with Argon2id-derived keys; no password storage/recovery is introduced.
- Protected body plaintext must never enter `Derived`, search indexing, logs, reports, failure bundles, titles, or unprotected persistence.
- Session key starts locked on every launch and is zeroized on lock/drop.
- Unprotected copy remains a normal persistent clipboard copy unless explicitly documented otherwise; protected-copy hardening must not unexpectedly alter ordinary snippets.
- Search, category filtering, themes, add/edit/delete, protected-card gating, drag/drop reorder, virtualization, filter/derived caching, and existing simulation journeys continue working.
- Every snippet mutation still invalidates/rebuilds derived/filter state through the established mutation seam; do not bypass it.
- `Missing` and `Corrupt` remain distinct storage outcomes.
- Atomic writes remain atomic; do not replace `write_atomic` with direct truncating writes.
- Existing corruption backups are never overwritten; `.corrupt`, `.corrupt.1`, etc. remain recoverable.
- The simulation harness must never touch real `%APPDATA%\CopyIt` data.
- Fixed-seed journey determinism remains a required property.

## Scope

Implement all ordered workstreams below unless a genuinely external blocker makes one impossible. If a workstream reveals a deeper same-domain correctness defect, fix it in the same campaign when doing so is necessary for the acceptance criteria. Do not use the campaign as justification for unrelated UI redesigns, theme work, feature expansion, or a wholesale `app.rs` rewrite.

Before changing behavior that affects persistence/recovery, vault lifecycle, or clipboard semantics, create an OpenSpec change (recommended name: `harden-data-vault-lifecycle`) with proposal/design/tasks/spec deltas. Keep it implementation-oriented and update tasks as work lands. Validate it strictly before completion.

## Ordered workstreams

### 0. Reconcile the live repository before editing

1. Read `AGENTS.md`, `CLAUDE.md`, `CONTEXT.md`, `.agent/PLANNER_HANDOFF.md`, this file, current OpenSpec changes, and current CI/audit policy.
2. Fetch/reconcile `origin/main`; confirm the actual HEAD and inspect all commits after `Planned-From` before assuming any finding is still present.
3. If work has landed since planning, preserve it and re-evaluate every finding against current code. Do not redo superseded work.
4. Establish a clean baseline with the relevant tests/builds. Record pre-existing failures separately; do not mislabel them as campaign regressions.

### 1. P0: enforce per-file write protection after unrecoverable corruption-backup failure

Replace the dead/ambiguous `refuse_overwrite` behavior with an explicit, testable persistence-safety state for **each** data file. A single bool is insufficient because `snippets.json` and `config.json` can fail independently.

Required behavior:

- If a corrupt original was successfully renamed aside, normal replacement/seeding may proceed as today.
- If backup/rename of a corrupt original fails, CopyIt must never write, truncate, rename over, or otherwise replace that original during the rest of the process.
- This protection must cover every path: initial seeding/canonicalization, add/edit/delete/reorder, category creation, theme selection, vault metadata creation/update, and any future save helper reached during the same session.
- The UI must clearly indicate recovery/read-only state. Do not merely suppress the disk write while presenting a successful-save experience.
- For blocked `snippets.json`, prevent misleading destructive library mutations against seeded/default in-memory data, or otherwise provide an equally safe explicit recovery-mode UX. The user must understand that the visible fallback library is not their persisted library.
- For blocked `config.json`, theme/category/vault operations must not claim durable success. In particular, never create/protect a snippet under vault metadata that cannot be persisted.
- Do not clear the recovery warning merely because an unrelated save succeeds.
- Recovery protection may be lifted only by a deliberately verified recovery/reload path or by restart after the underlying problem has been corrected; never infer safety from a failed write attempt.

Regression tests must force deterministic backup failure without relying on flaky OS permissions and prove the original bytes remain byte-for-byte unchanged after attempting **each class of later mutation**. Test snippets and config independently and together.

### 2. P0/P1: make settings persistence failures first-class and non-silent

Centralize config mutation/persistence handling so callers cannot casually discard failures.

Required behavior:

- Remove ordinary `let _ = self.save_config()` swallowing from theme/category flows.
- Every config write failure produces a visible, durable, deduplicated warning describing that the in-memory change is not safely persisted.
- Decide and consistently implement whether a failed setting change remains as an explicitly-unsaved in-memory state or rolls back to the last persisted value. Either is acceptable only if the UI is truthful and tests pin the behavior.
- Vault metadata is stricter: any operation whose safety depends on persisted vault metadata must roll back/refuse as it does today; never downgrade that invariant.
- A later successful config save may retire only the config-save error it actually repairs, not unrelated corruption/recovery notices or snippet-save errors.
- Repeated frame activity must not append duplicate copies of the same error indefinitely.

Add unit and simulation coverage for theme/category save failures plus restart semantics.

### 3. P1: make file-size limits real bounds before parsing/allocation

Refactor JSON loading so `MAX_DATA_FILE_BYTES` actually bounds bytes read into memory.

Required behavior:

- Preflight size where useful, but do not rely solely on metadata because the file can change between metadata and read.
- Use a bounded read that can detect `limit + 1` bytes and rejects anything larger without reading the rest of the file.
- Preserve `Missing`/zero-byte/corrupt semantics and UTF-8/serde errors.
- Do not allocate capacity based on an untrusted multi-gigabyte file length.
- Make the size-limit helper testable with a small injected limit so regression tests do not create 256 MiB fixtures.
- Cover exact-limit, limit+1, whitespace-only, invalid UTF-8/JSON, and normal round-trip cases.

### 4. P1: make stable-directory and legacy-migration failures explicit recovery states

Change the persistence seam so startup can distinguish at least:

- no legacy source exists,
- migration succeeded,
- a legacy source exists but migration was blocked/failed,
- stable data already exists and migration is unnecessary.

Required behavior:

- Do not silently treat “legacy data exists but could not be migrated” as a clean first launch.
- Leave every failed legacy source untouched and report the exact source path plus actionable failure reason in the UI/recovery state.
- Do not seed defaults over the intended stable destination when doing so would hide a known failed migration.
- Surface failure to create/access the stable data directory rather than silently relying on later write failures or an unexpected alternate location.
- Preserve the existing behavior that valid empty JSON (`[]` / appropriate config object) is real data while zero-byte/whitespace-only files contain nothing to migrate.
- After a successful migration, verify the stable copy can be read before best-effort renaming the legacy source to `.migrated`.
- Keep test/simulation stores isolated; the harness must not scan or mutate real legacy locations.

Add focused store tests and an integration-level startup/recovery test using explicit candidate directories rather than real user paths.

### 5. P1: bound protected clipboard lifetime without clobbering newer clipboard data

Add a small clipboard abstraction/module rather than spreading platform-specific handling through `app.rs`.

Required protected-copy semantics:

- Copying a protected snippet still gives the user plaintext immediately.
- Schedule best-effort removal after a short fixed security window (default target: 30 seconds; document the chosen value).
- Clear only if CopyIt can prove the clipboard still represents the protected copy it placed there. If the user or another application has changed the clipboard since then, do **nothing**.
- Never retain the protected plaintext merely to compare it later. Prefer an OS clipboard generation/sequence token or another design that avoids keeping a second plaintext copy in app state.
- Clearing must be best-effort and failure-safe; clipboard contention must not crash or freeze CopyIt.
- Never write the protected plaintext to logs, simulation reports, error strings, or persistent config.
- Ordinary unprotected snippet copies retain current behavior unless there is a compelling documented reason otherwise.
- Locking the vault should also attempt an immediate safe clear of a still-current protected CopyIt clipboard entry.

Because the app is Windows-only, a narrowly encapsulated Windows clipboard implementation is acceptable. Keep unsafe/platform API code minimal and isolated. If adding a crate, choose the smallest maintained dependency that satisfies the requirement and re-run the dependency audit.

For tests/simulation, inject or provide a deterministic fake clipboard backend. Cover: protected copy -> expiry clear; user overwrites clipboard before expiry -> CopyIt leaves it alone; vault lock -> safe early clear; unprotected copy -> no security timeout; clipboard backend failure -> visible/non-fatal behavior as appropriate.

Update README/AGENTS security notes to state that clearing is best-effort and cannot revoke copies already captured by third-party clipboard-history software.

### 6. P2: finish non-blocking vault KDF execution without breaking deterministic tests

Do not leave Argon2id key derivation on the egui frame thread.

Implement a bounded vault-work state machine for create/unlock operations:

- Production create/unlock KDF work runs off the UI thread.
- Only one vault KDF job can be active at a time; repeated submit clicks cannot spawn unbounded work.
- The modal enters a clear busy state while work is running and prevents duplicate submission while still allowing a safe cancel policy.
- A canceled/stale result must not execute an old pending copy/edit/protect action against newer UI state.
- Password buffers owned by the worker are zeroized as soon as practical after derivation/verification.
- Vault state remains locked until a verified success result is accepted by the current request.
- Creation still persists vault metadata atomically before a protected save can proceed; persistence failure rolls back the in-memory key/meta.
- Remove the current dead `vault_work` shape if a better design replaces it; do not retain unused concurrency scaffolding.
- Request repaints/poll results without a busy loop.

Preserve deterministic simulation by putting the execution mechanism behind a small seam: production can use a thread/worker, while unit/sim runs can use a deterministic immediate executor or explicitly controlled completion. The behavior/state machine must still be tested; do not weaken simulation determinism just to add concurrency.

Add tests for success, wrong password, corrupt metadata, duplicate submit, cancel/stale result, create-vault persistence failure, and pending action resumption.

### 7. Cross-cutting regression audit and dependency posture

After the implementation is functionally complete, re-audit the **entire affected interaction graph**, not only changed files:

- startup -> migration -> load -> corrupt backup -> seeding -> save guards;
- category/theme/vault config mutations -> config persistence -> restart;
- protected copy/edit/protect -> unlock/create worker -> clipboard -> lock/restart;
- simulation failure bundles -> verify no plaintext secrets are captured;
- cache invalidation/reorder behavior after any new recovery/read-only state;
- second-instance/instance-lock behavior if storage initialization changes;
- Windows release build and no-console behavior.

Re-run `cargo audit` and `cargo tree --target x86_64-pc-windows-msvc -i <crate>` for every advisory currently documented in `.cargo/audit.toml` / `docs/dependency-audit-notes.md`. Update the notes if the graph changed. Do not suppress a Windows-reachable vulnerability. Do not force a broad egui/eframe migration solely to silence INFO/unmaintained warnings.

## Tests and validation

At minimum, finish with all applicable commands green from a clean/reconciled tree:

```powershell
cargo fmt -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --bin copyit sim_journeys -- --test-threads 1
cargo build --locked --release
```

Also run:

```powershell
openspec validate harden-data-vault-lifecycle --strict
```

if that is the chosen OpenSpec change name.

Run `cargo audit` with the repository's `.cargo/audit.toml` policy where the tool is available, and verify the Windows target dependency graph for any reported advisory. If local audit tooling is unavailable, do not invent a pass: record the limitation and require the pushed CI audit job to provide the authoritative result.

### Required new regression coverage

The final suite must include explicit tests for all of these, not merely indirect coverage:

1. `snippets.json` corrupt + backup fails + later add/edit/delete/reorder attempt -> original corrupt bytes unchanged.
2. `config.json` corrupt + backup fails + later theme/category/vault operation -> original corrupt bytes unchanged.
3. both data files blocked simultaneously -> no cross-file success clears the other file's recovery state.
4. theme save fails -> no silent success; restart semantics match documented behavior.
5. category save fails -> no silent success; snippet/category consistency remains recoverable.
6. bounded loader exact-limit and limit+1 behavior using a small test limit.
7. legacy source exists + migration destination fails -> no seeded first-run fiction and source remains untouched.
8. stable data-directory initialization failure is surfaced.
9. protected clipboard expiry clears only CopyIt's still-current protected copy.
10. clipboard changed by user/another app before expiry -> never clobbered.
11. vault lock safely expires the still-current protected copy.
12. async KDF duplicate submit/cancel/stale-result safety.
13. simulation determinism still produces byte-identical normalized logs for same journey/persona/seed.
14. failure bundles/logs contain no protected plaintext.

Add or extend headless journeys where they provide real shipped-UI coverage; do not force platform clipboard API calls into deterministic headless journeys when a fake backend gives cleaner isolation.

## Integration/E2E expectations

Perform at least one headed Windows smoke pass against an isolated temporary store (never the real user library) covering:

1. normal first launch/add/edit/delete/reorder/theme flow;
2. protected snippet create-vault -> copy -> timeout/lock behavior -> unlock -> edit;
3. wrong-password path while KDF work is asynchronous;
4. restart with persisted vault/theme/categories;
5. intentionally blocked/corrupt store recovery state with proof the original bytes remain intact.

Use the existing headed simulation mode where it fits, and add a purpose-built isolated smoke helper only if necessary. Do not manually test by risking `%APPDATA%\CopyIt` production data.

## Acceptance criteria

The campaign is complete only when all of the following are true:

- No code path can overwrite a corrupt original after its backup failed, for either snippets or config.
- Recovery/write-block state is explicit per file, durable for the process lifetime, and truthfully represented in the UI.
- No ordinary theme/category config-save error is silently discarded.
- Oversized data files are rejected by a genuinely bounded read before unbounded allocation/parsing.
- A known failed legacy migration cannot masquerade as a clean first launch.
- Stable data-directory initialization failures are surfaced rather than hidden.
- Protected clipboard content has a bounded best-effort lifetime and CopyIt never clears newer third-party/user clipboard content.
- Vault create/unlock KDF work no longer blocks the egui UI thread, cannot race stale pending actions, and does not break deterministic simulation.
- Existing encrypted-at-rest, search-censorship, cache, drag/drop, theme, storage, and simulation invariants still pass.
- No protected plaintext is written to disk/logs/reports by the new code.
- Dependency audit posture is re-verified; no known Windows-reachable Critical/High vulnerability is ignored or introduced.
- Full lint/test/release/OpenSpec gates pass.
- No known Critical or High correctness/security regression remains in the touched interaction graph.

## Out of scope

Unless required to satisfy the acceptance criteria, do not:

- redesign the overall UI or theme system;
- add cloud sync, accounts, networking, telemetry, analytics, or an installer;
- introduce a new database or replace JSON storage;
- change the vault cipher/KDF parameters or invent password recovery;
- remove the documented protected-card hint/title/category metadata behavior;
- rewrite the grid/drag/cache architecture;
- perform a broad egui/eframe major-version migration;
- add unrelated snippet-management features.

If a major framework migration is the only viable way to resolve a newly discovered real Windows vulnerability, stop and document the evidence as a genuine blocker/next campaign boundary rather than smuggling that migration into this hardening run.

## Git and campaign-state requirements

Work autonomously through the full campaign; do not stop after the first fix.

- Start from/reconcile current `main`; never reset away newer landed work.
- Keep commits coherent and reviewable. A small number of substantial commits is preferred over noisy micro-commits.
- Never force-push.
- Update the OpenSpec task list as work completes.
- Keep this file `Status: ACTIVE` until all acceptance criteria are satisfied.
- If genuinely blocked by an external dependency or environment condition, set `Status: BLOCKED` and add a precise blocker section with evidence, completed work, remaining work, and exact unblock condition. Do not use BLOCKED for ordinary engineering difficulty.
- On successful completion, change `Status: COMPLETED` and append a `## Completion Report` containing:
  - starting SHA and final SHA;
  - major defects fixed and design decisions;
  - files/modules changed;
  - tests/journeys added;
  - exact validation commands and results;
  - dependency-audit result and any INFO warnings intentionally remaining;
  - any explicitly deferred non-Critical/non-High work;
  - CI status for the pushed final SHA.
- Commit the completed implementation and state/docs updates with a detailed session-report commit message.
- Reconcile with `origin/main` before the final push; resolve conflicts by preserving both valid newer work and campaign fixes.
- Push `main`.
- Verify local HEAD equals `origin/main`, the worktree is clean, and CI for the pushed SHA is green. If CI exposes a campaign regression, fix it, re-run validation, commit, and push again before marking COMPLETED.

## Completion gate

Do not declare this campaign done because unit tests pass around the files you touched. Completion requires the whole-system regression audit above, the explicit P0 data-preservation tests, integration/headed validation, clean release build, strict OpenSpec validation, pushed final state, and no known Critical/High regression.

When this file is consumed by `/goal continue`, begin with Workstream 0 reconciliation, then execute from the first genuinely incomplete requirement through the completion gate without asking for routine confirmations.