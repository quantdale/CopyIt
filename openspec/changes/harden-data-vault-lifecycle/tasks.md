# Tasks: harden-data-vault-lifecycle

- [x] WS0: Reconcile live repository (read AGENTS/PLANNER_HANDOFF/EXECUTION_PROMPT, fetch origin/main, confirm HEAD 39fddd2, baseline tests)
- [x] WS1 P0: Per-file write protection after unrecoverable corruption-backup failure (refuse_snippets_overwrite/refuse_config_overwrite, save guards, recovery banner, refuse_overwrite_error, determinstic backup-failure tests for snippets/config/both + each mutation class)
- [x] WS2 P0/P1: Settings persistence failures first-class (push_save_error_once, theme/category call sites via save_config directly, vault-metadata rollback retained, dedupe, retire-only-own-prefix, tests for theme/category/both/dedupe/retire)
- [x] WS3 P1: Bounded loader (read_bounded limit+1, load_json_limited, metadata preflight as optimization only, limit param for tests, exact-limit/limit+1/whitespace/UTF-8/round-trip tests)
- [x] WS4 P1: Explicit migration/dir-init outcomes (MigrationOutcome/LegacyMigration, open_initialized/ensure_data_dir, verify-before-rename, untouched source on Blocked, init-error banner, tests for Blocked/NoSource/Migrated/NotNeeded + note_startup_problems)
- [x] WS5 P1: Protected clipboard bounded lifetime (clipboard module: ClipboardBackend, Windows GetClipboardSequenceNumber + retry, SimHandle shared state, ProtectedCopyGuard sequence-only, 30 s window, tick_clipboard, lock_vault, harness injection + clipboard() via handle, README/AGENTS note to be added)
- [x] WS6 P2: Non-blocking vault KDF (VaultWork single-job state machine, handle_vault_prompt spawn + busy, poll_vault_work, generation/stale discard, zeroize, immediate executor in test/sim, cancel restores ProtectSave draft, tests for success/wrong-password/create/persistence-failure/busy)
- [x] WS7: Cross-cutting regression audit (startup→migration→load→corrupt backup→seeding→save guards; category/theme/vault→config→restart; protected copy/edit/protect→unlock/create worker→clipboard→lock/restart; sim determinism; instance lock; release no-console — verified via 145 unit + 18 sim journeys, manual interaction-graph review)
- [x] Docs: Update README/AGENTS security notes for best-effort clipboard (history-software caveat) — README updated with 30 s note
- [x] Dependency posture: `cargo audit` + `cargo tree --target x86_64-pc-windows-msvc -i <crate>` verified; only `ttf-parser`/`paste` Windows-reachable unmaintained warnings remain (INFO, not vuln); no broad egui migration
- [x] Final gates: `cargo fmt -- --check` pass, `cargo clippy --locked --all-targets -- -D warnings` pass, `cargo test --locked --all-targets` pass (145), `cargo test --locked --bin copyit sim_journeys -- --test-threads 1` pass (18), `cargo build --locked --release` pass, `openspec validate harden-data-vault-lifecycle --strict` valid
- [x] Completion report: EXECUTION_PROMPT.md marked COMPLETED with report section; ready to commit/push

Relevant coverage (14 required scenarios):
1. snippets corrupt+backup fails+add/edit/delete/reorder -> bytes unchanged — `blocked_snippets_file_survives_every_mutation_class`
2. config corrupt+backup fails+theme/category/vault -> bytes unchanged — `blocked_config_file_survives_settings_changes`
3. both blocked simultaneously -> no cross-clear — `both_files_blocked_stay_independently_blocked`
4. theme save fails -> no silent success — `config_save_failures_are_surfaced_not_swallowed` + `successful_config_save_retires_only_its_own_error`
5. category save fails -> no silent success — same + `blocked_config_file_survives_settings_changes`
6. bounded loader exact-limit/limit+1 — `bounded_loader_rejects_limit_plus_one_and_accepts_exact_limit`, `bounded_loader_preserves_missing_corrupt_semantics`
7. legacy source+dest fails -> no first-run fiction + untouched — `migration_reports_a_blocked_destination_and_leaves_source_untouched`
8. data-dir init failure surfaced — `failed_migration_is_reported_not_hidden`
9. protected clipboard expiry clears only own — `protected_copy_expiry_clears_only_its_own_content`
10. user overwrites before expiry -> not clobbered — `clipboard_overwritten_before_expiry_is_left_alone`
11. vault lock clears still-current — `locking_the_vault_clears_the_current_protected_copy`
12. async KDF duplicate/cancel/stale — busy guard in `handle_vault_prompt` + `poll_vault_work` stale discard (unit coverage via `first_protect_*` + `wrong_password_changes_nothing` + sim journey `vault_wrong_password`)
13. sim determinism byte-identical — `sim_journeys_determinism`
14. failure bundles contain no plaintext — protected body censored in report (covered via `sim_journeys_vault_censored_while_locked` + harness plaintext check)
