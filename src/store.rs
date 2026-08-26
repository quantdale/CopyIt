//! The snippet library's persistence seam.
//!
//! Everything about *where* data lives and *how* it is loaded, migrated,
//! and saved lives behind [`Store`]'s interface, so `app.rs` never touches
//! paths or migration rules. Low-level JSON IO stays in `storage.rs`.

use crate::model::Snippet;
use crate::storage::{self, Config};
use std::io;
use std::path::{Path, PathBuf};

/// Owns the on-disk location of the snippet library and its config.
pub struct Store {
    pub snippets_path: PathBuf,
    pub config_path: PathBuf,
}

impl Store {
    /// Opens the stable per-user data directory (created if needed).
    pub fn open() -> Self {
        let (dir, _) = storage::ensure_data_dir();
        Self::at(dir)
    }

    /// Opens the stable per-user data directory and reports whether it could be
    /// initialized. The store still points at the intended location even when
    /// creation failed, so the caller can surface the exact path in the UI instead
    /// of silently falling back to an unexpected location.
    pub fn open_initialized() -> (Self, Option<std::io::Error>) {
        let (dir, init_error) = storage::ensure_data_dir();
        (Self::at(dir), init_error)
    }

    /// A store rooted at an explicit directory. Does not create the
    /// directory; saving into a missing directory returns an `io::Error`.
    pub fn at(dir: PathBuf) -> Self {
        Self {
            snippets_path: dir.join("snippets.json"),
            config_path: dir.join("config.json"),
        }
    }

    /// One-time recovery for users upgrading from earlier versions that stored
    /// `snippets.json`/`config.json` next to the .exe: if the stable location
    /// doesn't have a file yet, pull in the first non-empty copy found in a
    /// legacy location (next to the exe, `target/debug`, `target/release`, cwd).
    /// Runs once per file per session; after that the stable location owns the
    /// data and legacy locations are ignored.
    ///
    /// Returns one outcome per file so startup can distinguish a clean first
    /// launch from a legacy source that exists but could not be migrated — the
    /// latter must never masquerade as first-run fiction.
    pub fn migrate_legacy(&self) -> LegacyMigration {
        LegacyMigration {
            snippets: migrate_path(&self.snippets_path, "snippets.json"),
            config: migrate_path(&self.config_path, "config.json"),
        }
    }

    /// Loads the snippet library, distinguishing `Missing` (first launch, seed
    /// the defaults) from `Corrupt` (there *is* user data that failed to parse).
    pub fn load_snippets(&self) -> storage::Load<Vec<Snippet>> {
        storage::load(&self.snippets_path)
    }

    /// Loads the config (canonical categories and theme).
    pub fn load_config(&self) -> storage::Load<Config> {
        storage::load_config(&self.config_path)
    }

    /// Persists the snippet library, atomically.
    pub fn save_snippets(&self, snippets: &[Snippet]) -> io::Result<()> {
        storage::save(&self.snippets_path, snippets)
    }

    /// Persists the config, atomically.
    pub fn save_config(&self, config: &Config) -> io::Result<()> {
        storage::save_config(&self.config_path, config)
    }
}

/// Outcome of migrating one legacy data file into the stable location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    /// Stable data already exists; migration was unnecessary.
    NotNeeded,
    /// No non-empty legacy source was found in any candidate location.
    NoSource,
    /// A legacy source was copied into the stable location (and verified).
    Migrated { source: PathBuf },
    /// A legacy source exists but could not be migrated. The source is left
    /// untouched and the exact path + reason are reported to the user.
    Blocked { source: PathBuf, reason: String },
}

/// Per-file outcomes of [`Store::migrate_legacy`].
#[derive(Debug, Clone)]
pub struct LegacyMigration {
    pub snippets: MigrationOutcome,
    pub config: MigrationOutcome,
}

impl LegacyMigration {
    /// True when every outcome is a clean no-op (nothing to do / nothing failed).
    pub fn all_clean(&self) -> bool {
        matches!(self.snippets, MigrationOutcome::NotNeeded | MigrationOutcome::NoSource)
            && matches!(self.config, MigrationOutcome::NotNeeded | MigrationOutcome::NoSource)
    }
}

/// Moves the first non-empty legacy `filename` into `new_path` if the stable
/// location doesn't have one yet, searching the standard legacy locations.
fn migrate_path(new_path: &Path, filename: &str) -> MigrationOutcome {
    migrate_path_from(new_path, filename, &storage::legacy_candidate_dirs())
}

/// Moves the first non-empty legacy `filename` found in an explicit `candidates`
/// list into `new_path` if the stable location doesn't have one yet. Extracted
/// from [`migrate_path`] as a pure function so migration can be tested with an
/// explicit candidate list instead of the hard-coded legacy scan locations.
fn migrate_path_from(new_path: &Path, filename: &str, candidates: &[PathBuf]) -> MigrationOutcome {
    if new_path.exists() {
        return MigrationOutcome::NotNeeded; // Already migrated or created fresh
    }
    let mut unreadable: Option<(PathBuf, String)> = None;
    for dir in candidates {
        let candidate = dir.join(filename);
        if candidate == new_path {
            continue; // Skip the new location itself (shouldn't happen, but be safe)
        }
        let data = match std::fs::read_to_string(&candidate) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => {
                // An existing source we cannot read is a potential recovery copy:
                // remember it but keep scanning for one that migrates cleanly.
                if unreadable.is_none() {
                    unreadable = Some((candidate.clone(), e.to_string()));
                }
                continue;
            }
            Ok(data) => data,
        };
        // Skip zero-byte / whitespace-only files (nothing to lose). A valid
        // empty array "[]" or object "{}" is real data and gets migrated.
        if data.trim().is_empty() {
            continue;
        }
        // Write atomically so a crash mid-migration can't leave a truncated
        // stable file. On failure, report the exact source instead of silently
        // continuing — startup must not present this as a clean first launch.
        if let Err(e) = storage::write_atomic(new_path, &data) {
            return MigrationOutcome::Blocked {
                source: candidate,
                reason: e.to_string(),
            };
        }
        // Verify the stable copy actually landed intact before renaming the
        // legacy source aside; a silent mismatch would risk losing the original.
        match std::fs::read_to_string(new_path) {
            Ok(written) if written == data => {
                // Best-effort: rename the legacy source so it isn't re-migrated
                // on the next launch. Failure is harmless — the data is already
                // safely in the stable location.
                if let Some(name) = candidate.file_name() {
                    let mut new_name = name.to_owned();
                    new_name.push(".migrated");
                    let _ = std::fs::rename(&candidate, candidate.with_file_name(new_name));
                }
                return MigrationOutcome::Migrated { source: candidate };
            }
            Ok(_) | Err(_) => {
                return MigrationOutcome::Blocked {
                    source: candidate,
                    reason: "the copied file could not be read back for verification".to_string(),
                };
            }
        }
    }
    match unreadable {
        Some((source, reason)) => MigrationOutcome::Blocked { source, reason },
        None => MigrationOutcome::NoSource,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("copyit-store-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn snippet(id: u64, category: &str) -> Snippet {
        Snippet {
            id,
            title: format!("Snippet {id}"),
            category: category.to_string(),
            body: "body".to_string(),
            protection: None,
        }
    }

    #[test]
    fn at_joins_the_fixed_filenames() {
        let store = Store::at(PathBuf::from("some/dir"));
        assert_eq!(store.snippets_path, PathBuf::from("some/dir/snippets.json"));
        assert_eq!(store.config_path, PathBuf::from("some/dir/config.json"));
    }

    #[test]
    fn first_load_in_an_empty_dir_is_missing() {
        let store = Store::at(temp_dir("empty"));
        assert!(matches!(store.load_snippets(), storage::Load::Missing));
        assert!(matches!(store.load_config(), storage::Load::Missing));
    }

    #[test]
    fn save_and_load_round_trips() {
        let dir = temp_dir("roundtrip");
        let store = Store::at(dir);
        store
            .save_snippets(&[snippet(1, "Git"), snippet(2, "Prompt")])
            .unwrap();
        store
            .save_config(&Config {
                categories: vec!["Git".into(), "Prompt".into()],
                theme: "Nord".into(),
                vault: None,
            })
            .unwrap();

        match store.load_snippets() {
            storage::Load::Loaded(snippets) => {
                assert_eq!(
                    snippets.iter().map(|s| s.id).collect::<Vec<_>>(),
                    vec![1, 2]
                );
            }
            _ => panic!("saved snippets should load back"),
        }
        match store.load_config() {
            storage::Load::Loaded(config) => {
                assert_eq!(
                    config.categories,
                    vec!["Git".to_string(), "Prompt".to_string()]
                );
                assert_eq!(config.theme, "Nord");
            }
            _ => panic!("saved config should load back"),
        }
    }

    #[test]
    fn a_corrupt_file_is_not_reported_as_missing() {
        let dir = temp_dir("corrupt");
        let store = Store::at(dir.clone());
        std::fs::write(store.snippets_path.clone(), "not json").unwrap();
        assert!(matches!(store.load_snippets(), storage::Load::Corrupt(_)));
        // The bytes are left in place; the caller decides how to preserve them.
        assert_eq!(
            std::fs::read_to_string(store.snippets_path).unwrap(),
            "not json"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_copies_first_non_empty_legacy_file_verbatim() {
        let dir = temp_dir("migrate_copy");
        let new_path = dir.join("new").join("snippets.json");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let legacy = dir.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        let data = r#"[{"id":1,"title":"X","category":"Git","body":"y"}]"#;
        std::fs::write(legacy.join("snippets.json"), data).unwrap();

        let outcome = migrate_path_from(&new_path, "snippets.json", std::slice::from_ref(&legacy));
        assert_eq!(
            outcome,
            MigrationOutcome::Migrated { source: legacy.join("snippets.json") }
        );
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_skips_empty_files_but_migrates_empty_json() {
        let dir = temp_dir("migrate_skip");
        // Empty / whitespace-only files are truly empty and skipped.
        // Valid-but-empty JSON ("[]", "{}") is real data and gets migrated.
        let empty = dir.join("empty");
        let array = dir.join("array");
        let object = dir.join("object");
        for d in [&empty, &array, &object] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(empty.join("snippets.json"), "").unwrap();
        std::fs::write(array.join("snippets.json"), "[]").unwrap();
        std::fs::write(object.join("snippets.json"), "{}").unwrap();

        let new_path = dir.join("new").join("snippets.json");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        migrate_path_from(
            &new_path,
            "snippets.json",
            &[empty.clone(), array.clone(), object.clone()],
        );
        // The first non-empty candidate ("[]") should have been migrated,
        // and its legacy source renamed to .migrated.
        assert!(new_path.exists());
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "[]");
        assert!(
            array.join("snippets.json.migrated").exists(),
            "legacy source renamed after migration"
        );
        // The empty candidate was skipped (not migrated, not renamed).
        assert!(empty.join("snippets.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_skips_a_candidate_that_resolves_to_new_path() {
        let dir = temp_dir("migrate_self");
        let new_path = dir.join("new").join("snippets.json");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        std::fs::write(&new_path, "data").unwrap();
        // The candidate dir is new_path's parent, so candidate.join(filename) ==
        // new_path. The file must not be treated as a legacy source or copied
        // over itself.
        migrate_path_from(
            &new_path,
            "snippets.json",
            &[new_path.parent().unwrap().to_path_buf()],
        );
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_does_nothing_when_new_path_already_exists() {
        let dir = temp_dir("migrate_exists");
        let new_path = dir.join("new").join("snippets.json");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        std::fs::write(&new_path, "stable data").unwrap();
        let legacy = dir.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("snippets.json"), "legacy data").unwrap();

        assert_eq!(
            migrate_path_from(&new_path, "snippets.json", std::slice::from_ref(&legacy)),
            MigrationOutcome::NotNeeded
        );
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "stable data");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A legacy source whose stable destination cannot be written must be reported
    /// as Blocked with its exact path — never silently skipped (which would present
    /// a misleading clean first launch) — and the source bytes stay untouched.
    #[test]
    fn migration_reports_a_blocked_destination_and_leaves_source_untouched() {
        let dir = temp_dir("migrate_blocked");
        let legacy = dir.join("legacy");
        std::fs::create_dir_all(&legacy).unwrap();
        let data = r#"[{"id":1,"title":"X","category":"Git","body":"y"}]"#;
        std::fs::write(legacy.join("snippets.json"), data).unwrap();

        // The destination directory does not exist and is not created by the
        // migration itself, so the atomic write fails.
        let new_path = dir.join("missing").join("snippets.json");
        let outcome = migrate_path_from(&new_path, "snippets.json", std::slice::from_ref(&legacy));

        match &outcome {
            MigrationOutcome::Blocked { source, reason } => {
                assert_eq!(source, &legacy.join("snippets.json"));
                assert!(!reason.is_empty());
            }
            other => panic!("expected Blocked, got {other:?}"),
        }
        assert!(!new_path.exists(), "no partial copy may appear at the destination");
        assert_eq!(
            std::fs::read_to_string(legacy.join("snippets.json")).unwrap(),
            data,
            "the legacy source must be left byte-for-byte intact"
        );
        assert!(!legacy.join("snippets.json.migrated").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// No candidate holds a source: the outcome must say so explicitly rather
    /// than being indistinguishable from a blocked migration.
    #[test]
    fn migration_reports_no_source_cleanly() {
        let dir = temp_dir("migrate_nosource");
        let new_path = dir.join("new").join("config.json");
        std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
        let elsewhere = dir.join("nowhere");
        std::fs::create_dir_all(&elsewhere).unwrap();

        assert_eq!(
            migrate_path_from(&new_path, "config.json", std::slice::from_ref(&elsewhere)),
            MigrationOutcome::NoSource
        );
        assert!(!new_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The aggregated per-file outcomes expose a clean startup only when neither
    /// file was blocked.
    #[test]
    fn legacy_migration_aggregates_per_file_outcomes() {
        let clean = LegacyMigration {
            snippets: MigrationOutcome::NoSource,
            config: MigrationOutcome::NotNeeded,
        };
        assert!(clean.all_clean());
        let dirty = LegacyMigration {
            snippets: MigrationOutcome::Blocked {
                source: PathBuf::from("x"),
                reason: "denied".into(),
            },
            config: MigrationOutcome::NoSource,
        };
        assert!(!dirty.all_clean());
    }
}
