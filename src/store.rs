//! The snippet library's persistence seam.
//!
//! Everything about *where* data lives and *how* it is loaded, migrated, and
//! saved lives behind [`Store`]'s interface, so `app.rs` never opens a database
//! or touches migration rules directly. Since Phase D the canonical store is the
//! shared SQLite database (`copyit.db`); this module is a thin adapter over
//! `sqlite.rs` that preserves the legacy JSON `Load`/`Missing`/`Corrupt`
//! semantics the rest of the app already understands.

use crate::model::Snippet;
use crate::sqlite;
use crate::storage::{self, Config, Load};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Owns the on-disk location of the snippet library.
///
/// `snippets_path` / `config_path` point at the *legacy* JSON filenames only so
/// that existing users' `snippets.json` / `config.json` can be discovered and
/// imported; the live store is the sibling `copyit.db` SQLite file.
pub struct Store {
    pub snippets_path: PathBuf,
    pub config_path: PathBuf,
}

impl Store {
    /// Opens the stable per-user data directory and reports whether it could be
    /// initialized. The store still points at the intended location even when
    /// creation failed, so the caller can surface the exact path in the UI.
    #[allow(dead_code)] // called by `CopyIt::new` (the production entry; not compiled into the test target)
    pub fn open_initialized() -> (Self, Option<io::Error>) {
        let (dir, init_error) = storage::ensure_data_dir();
        (Self::at(dir), init_error)
    }

    /// A store rooted at an explicit directory. Does not create the directory;
    /// saving into a missing directory returns an `io::Error`.
    pub fn at(dir: PathBuf) -> Self {
        Self {
            snippets_path: dir.join("snippets.json"),
            config_path: dir.join("config.json"),
        }
    }

    /// The canonical SQLite database path (beside the legacy JSON files).
    fn db_path(&self) -> PathBuf {
        let dir = self
            .snippets_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        sqlite::db_path_for(&dir)
    }

    /// One-time recovery for users upgrading from earlier versions that stored
    /// `snippets.json`/`config.json`: if the SQLite store doesn't exist yet,
    /// import the first non-empty copy found. Runs at most once per process;
    /// after the SQLite db exists it is authoritative and the JSON is ignored.
    /// Returns per-file outcomes so a blocked migration cannot masquerade as a
    /// clean first launch.
    pub fn migrate_legacy(&self) -> LegacyMigration {
        if self.db_path().exists() {
            return LegacyMigration {
                snippets: MigrationOutcome::NotNeeded,
                config: MigrationOutcome::NotNeeded,
            };
        }
        match self.import_legacy_json() {
            ImportResult::Imported => LegacyMigration {
                snippets: MigrationOutcome::Migrated {
                    source: self.snippets_path.clone(),
                },
                config: MigrationOutcome::Migrated {
                    source: self.config_path.clone(),
                },
            },
            ImportResult::NoSource => LegacyMigration {
                snippets: MigrationOutcome::NoSource,
                config: MigrationOutcome::NoSource,
            },
            ImportResult::Corrupt(reason) => LegacyMigration {
                snippets: MigrationOutcome::Blocked {
                    source: self.snippets_path.clone(),
                    reason: reason.clone(),
                },
                config: MigrationOutcome::Blocked {
                    source: self.config_path.clone(),
                    reason,
                },
            },
        }
    }

    /// Loads the snippet library, distinguishing `Missing` (first launch, seed
    /// the defaults) from `Corrupt` (there *is* user data that failed to parse).
    /// A missing SQLite db is auto-imported from legacy JSON on this call.
    pub fn load_snippets(&self) -> Load<Vec<Snippet>> {
        let db = self.db_path();
        if db.exists() {
            return match sqlite::open_read_only(&db).and_then(|c| sqlite::load_all_snippets(&c)) {
                Ok(v) => Load::Loaded(v),
                Err(e) => Load::Corrupt(e.to_string()),
            };
        }
        match self.import_legacy_json() {
            ImportResult::Imported => {
                match sqlite::open_db(&db).and_then(|c| sqlite::load_all_snippets(&c)) {
                    Ok(v) => Load::Loaded(v),
                    Err(e) => Load::Corrupt(e.to_string()),
                }
            }
            ImportResult::Corrupt(reason) => Load::Corrupt(reason),
            ImportResult::NoSource => Load::Missing,
        }
    }

    /// Loads the config (canonical categories, theme, vault metadata).
    pub fn load_config(&self) -> Load<Config> {
        let db = self.db_path();
        if db.exists() {
            return match sqlite::open_read_only(&db).and_then(|c| sqlite::load_config(&c)) {
                Ok(Some(c)) => Load::Loaded(c),
                Ok(None) => Load::Missing,
                Err(e) => Load::Corrupt(e.to_string()),
            };
        }
        match self.import_legacy_json() {
            ImportResult::Imported => {
                match sqlite::open_db(&db).and_then(|c| sqlite::load_config(&c)) {
                    Ok(Some(c)) => Load::Loaded(c),
                    Ok(None) => Load::Missing,
                    Err(e) => Load::Corrupt(e.to_string()),
                }
            }
            ImportResult::Corrupt(reason) => Load::Corrupt(reason),
            ImportResult::NoSource => Load::Missing,
        }
    }

    /// Persists the snippet library (full reconcile: upsert all, delete removed,
    /// keep sort order aligned with the in-memory list).
    pub fn save_snippets(&self, snippets: &[Snippet]) -> io::Result<()> {
        let db = self.db_path();
        let conn = sqlite::open_db(&db).map_err(to_io)?;
        sqlite::reconcile_snippets(&conn, snippets).map_err(to_io)
    }

    /// Persists the config (theme, canonical categories, vault triple).
    pub fn save_config(&self, config: &Config) -> io::Result<()> {
        let db = self.db_path();
        let conn = sqlite::open_db(&db).map_err(to_io)?;
        sqlite::set_theme(&conn, &config.theme).map_err(to_io)?;
        sqlite::set_categories(&conn, &config.categories).map_err(to_io)?;
        sqlite::set_vault_meta(&conn, config.vault.as_ref()).map_err(to_io)?;
        Ok(())
    }

    /// Imports legacy JSON into the SQLite store exactly once. Never deletes the
    /// originals; on success they are renamed aside as `<name>.legacy-backup-<ts>`.
    fn import_legacy_json(&self) -> ImportResult {
        if self.db_path().exists() {
            return ImportResult::Imported;
        }
        let snippets_src = storage::load(&self.snippets_path);
        let config_src = storage::load_config(&self.config_path);

        let any_loaded =
            matches!(snippets_src, Load::Loaded(_)) || matches!(config_src, Load::Loaded(_));
        let any_corrupt =
            matches!(snippets_src, Load::Corrupt(_)) || matches!(config_src, Load::Corrupt(_));

        // A corrupt legacy source must not be overwritten: refuse to create a db.
        if any_corrupt && !any_loaded {
            let reason = match (&snippets_src, &config_src) {
                (Load::Corrupt(e), _) => e.clone(),
                (_, Load::Corrupt(e)) => e.clone(),
                _ => "corrupt legacy data".to_string(),
            };
            return ImportResult::Corrupt(reason);
        }
        if !any_loaded {
            return ImportResult::NoSource;
        }

        let db = self.db_path();
        let conn = match sqlite::open_db(&db) {
            Ok(c) => c,
            Err(e) => return ImportResult::Corrupt(e.to_string()),
        };
        let snips: Vec<Snippet> = match snippets_src {
            Load::Loaded(v) => v,
            _ => Vec::new(),
        };
        if let Err(e) = sqlite::reconcile_snippets(&conn, &snips) {
            return ImportResult::Corrupt(e.to_string());
        }
        let cfg: Config = match config_src {
            Load::Loaded(c) => c,
            _ => Config::from_snippets(&snips),
        };
        let _ = sqlite::set_theme(&conn, &cfg.theme);
        let _ = sqlite::set_categories(&conn, &cfg.categories);
        let _ = sqlite::set_vault_meta(&conn, cfg.vault.as_ref());

        backup_legacy(&self.snippets_path);
        backup_legacy(&self.config_path);
        ImportResult::Imported
    }
}

fn to_io(e: rusqlite::Error) -> io::Error {
    io::Error::other(e)
}

/// Renames `path` aside as a timestamped backup, leaving its contents fully
/// recoverable. Failures are ignored: the import has already succeeded and a
/// missing original simply means there is nothing to preserve.
fn backup_legacy(path: &Path) {
    if !path.exists() {
        return;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let backup = path.with_file_name(format!(
        "{}.legacy-backup-{}",
        path.file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default(),
        nanos
    ));
    let _ = std::fs::rename(path, backup);
}

/// Outcome of migrating one legacy data file into the SQLite store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrationOutcome {
    NotNeeded,
    NoSource,
    Migrated { source: PathBuf },
    Blocked { source: PathBuf, reason: String },
}

#[derive(Debug, Clone)]
pub struct LegacyMigration {
    pub snippets: MigrationOutcome,
    pub config: MigrationOutcome,
}

impl LegacyMigration {
    #[allow(dead_code)] // diagnostic helper; surfaced through `note_startup_problems` in some builds
    pub fn all_clean(&self) -> bool {
        matches!(
            self.snippets,
            MigrationOutcome::NotNeeded | MigrationOutcome::NoSource
        ) && matches!(
            self.config,
            MigrationOutcome::NotNeeded | MigrationOutcome::NoSource
        )
    }
}

/// Internal result of a single legacy-JSON import attempt.
enum ImportResult {
    Imported,
    NoSource,
    Corrupt(String),
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
            description: String::new(),
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
        assert!(matches!(store.load_snippets(), Load::Missing));
        assert!(matches!(store.load_config(), Load::Missing));
    }

    #[test]
    fn save_and_load_round_trips_through_sqlite() {
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
            Load::Loaded(snippets) => {
                assert_eq!(
                    snippets.iter().map(|s| s.id).collect::<Vec<_>>(),
                    vec![1, 2]
                );
            }
            _ => panic!("saved snippets should load back"),
        }
        match store.load_config() {
            Load::Loaded(config) => {
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
    fn legacy_json_is_imported_once_and_backed_up() {
        let dir = temp_dir("import");
        let store = Store::at(dir.clone());
        let legacy_snips = vec![snippet(1, "Git")];
        std::fs::write(
            store.snippets_path.clone(),
            serde_json::to_string(&legacy_snips).unwrap(),
        )
        .unwrap();
        std::fs::write(
            store.config_path.clone(),
            serde_json::to_string(&Config {
                categories: vec!["Git".into()],
                theme: "Dark".into(),
                vault: None,
            })
            .unwrap(),
        )
        .unwrap();

        assert!(matches!(store.load_snippets(), Load::Loaded(_)));
        // The SQLite db now owns the data and the JSON was renamed aside.
        assert!(store.db_path().exists());
        assert!(!store.snippets_path.exists());
        assert!(!store.config_path.exists());

        // A second load reads from the db, not the (now gone) JSON.
        assert!(matches!(store.load_snippets(), Load::Loaded(_)));
    }

    #[test]
    fn corrupt_legacy_json_is_not_overwritten() {
        let dir = temp_dir("corrupt-import");
        let store = Store::at(dir);
        std::fs::write(store.snippets_path.clone(), "not json").unwrap();
        match store.load_snippets() {
            Load::Corrupt(_) => {}
            other => panic!("expected Corrupt, got {other:?}"),
        }
        // No db was created over the corrupt source.
        assert!(!store.db_path().exists());
    }
}
