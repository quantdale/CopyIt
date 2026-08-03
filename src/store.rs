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
/// location doesn't have one yet.
fn migrate_path(new_path: &Path, filename: &str) {
    if new_path.exists() {
        return; // Already migrated or was created fresh; don't search legacy locations
    }
    for dir in storage::legacy_candidate_dirs() {
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
            let _ = std::fs::write(new_path, data);
            return; // Success: migrate and stop searching
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
    fn migrate_skips_empty_placeholder_files() {
        let dir = temp_dir("migrate_empty");
        let store = Store::at(dir.join("new"));
        std::fs::create_dir_all(dir.join("legacy")).unwrap();
        std::fs::write(dir.join("legacy/snippets.json"), "[]").unwrap();
        std::fs::write(dir.join("legacy/config.json"), "{}").unwrap();
        store.migrate_legacy();
        assert!(!store.snippets_path.exists());
        assert!(!store.config_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
