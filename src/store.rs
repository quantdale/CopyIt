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
        Self::at(storage::data_dir())
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
    pub fn migrate_legacy(&self) {
        migrate_path(&self.snippets_path, "snippets.json");
        migrate_path(&self.config_path, "config.json");
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

/// Moves the first non-empty legacy `filename` into `new_path` if the stable
/// location doesn't have one yet, searching the standard legacy locations.
fn migrate_path(new_path: &Path, filename: &str) {
    migrate_path_from(new_path, filename, &storage::legacy_candidate_dirs());
}

/// Moves the first non-empty legacy `filename` found in an explicit `candidates`
/// list into `new_path` if the stable location doesn't have one yet. Extracted
/// from [`migrate_path`] as a pure function so migration can be tested with an
/// explicit candidate list instead of the hard-coded legacy scan locations.
fn migrate_path_from(new_path: &Path, filename: &str, candidates: &[PathBuf]) {
    if new_path.exists() {
        return; // Already migrated or was created fresh; don't search legacy locations
    }
    for dir in candidates {
        let candidate = dir.join(filename);
        if candidate == new_path {
            continue; // Skip the new location itself (shouldn't happen, but be safe)
        }
        if let Ok(data) = std::fs::read_to_string(&candidate) {
            let trimmed = data.trim();
            // Skip empty or dummy JSON (e.g. "[]" or "{}" from a failed write).
            if trimmed.is_empty() || trimmed == "[]" || trimmed == "{}" {
                continue;
            }
            // Write atomically so a crash mid-migration can't leave a truncated
            // stable file. On write failure, stop rather than swallow it: the
            // legacy source stays intact for the next launch to retry.
            if storage::write_atomic(new_path, &data).is_ok() {
                return; // Success: migrate and stop searching
            }
        }
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

        migrate_path_from(&new_path, "snippets.json", std::slice::from_ref(&legacy));
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), data);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_skips_empty_and_placeholder_legacy_files() {
        let dir = temp_dir("migrate_skip");
        // Three candidate dirs, each holding a "snippets.json"; all are empty or
        // dummy JSON, so nothing should be migrated.
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
        migrate_path_from(
            &new_path,
            "snippets.json",
            &[empty.clone(), array.clone(), object.clone()],
        );
        assert!(!new_path.exists());
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

        migrate_path_from(&new_path, "snippets.json", std::slice::from_ref(&legacy));
        assert_eq!(std::fs::read_to_string(&new_path).unwrap(), "stable data");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
