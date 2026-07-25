use crate::model::Snippet;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Category assigned to snippets that carry no usable category of their own
/// (blank, or the reserved "All" filter label).
pub const UNCATEGORIZED: &str = "Uncategorized";

/// Resolves the stable per-user directory where snippets.json and config.json live.
/// Uses `%APPDATA%\CopyIt` on Windows (preferred: survives git checkouts, cargo clean, etc.),
/// falling back to the directory containing the running .exe on non-Windows or when APPDATA
/// is unset (e.g., in dev/CI environments). This strategy decouples data persistence from
/// build artifacts: users can freely update/rebuild the application without losing their
/// saved snippets. The directory is created on first access if it doesn't exist.
pub fn data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        let dir = PathBuf::from(appdata).join("CopyIt");
        let _ = std::fs::create_dir_all(&dir);
        return dir;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    PathBuf::from(".") // Last-resort fallback: current working directory
}

/// Returns the full path to snippets.json in the stable data directory.
pub fn data_path() -> PathBuf {
    data_dir().join("snippets.json")
}

/// Small config file holding canonical categories and the selected theme.
/// Kept separate from `snippets.json` so snippet data stays
/// backward-compatible with earlier versions.
pub fn config_path() -> PathBuf {
    data_dir().join("config.json")
}

/// Legacy locations `snippets.json`/`config.json` may have been left in by
/// earlier versions that stored data next to the .exe. Used for one-time
/// migration into the new stable `data_dir()`.
pub fn legacy_candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
            if let Some(target) = dir.parent() {
                dirs.push(target.join("debug"));
                dirs.push(target.join("release"));
            }
        }
    }
    dirs.push(PathBuf::from("."));
    dirs
}

/// Outcome of reading one of the JSON data files.
///
/// The three cases must stay distinct: `Missing` means "first launch, seed the
/// defaults and write them out", while `Corrupt` means "there *is* user data
/// here that we failed to understand". Collapsing the two (as an
/// `Option`-returning loader does) makes the app silently seed defaults over a
/// file it couldn't parse, destroying the user's library on the next save.
pub enum Load<T> {
    /// The file was read and parsed successfully.
    Loaded(T),
    /// The file doesn't exist yet, or exists but is empty — nothing to lose.
    Missing,
    /// The file exists but couldn't be read or parsed; the message describes why.
    Corrupt(String),
}

/// Reads and deserializes a JSON data file, distinguishing "not there yet" from
/// "there but unreadable". A zero-byte file is reported as `Missing` because it
/// holds no data that could be lost by overwriting it.
fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Load<T> {
    let data = match std::fs::read_to_string(path) {
        Ok(data) => data,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Load::Missing,
        Err(e) => return Load::Corrupt(e.to_string()),
    };
    if data.trim().is_empty() {
        return Load::Missing;
    }
    match serde_json::from_str(&data) {
        Ok(value) => Load::Loaded(value),
        Err(e) => Load::Corrupt(e.to_string()),
    }
}

/// Loads the snippet library from a JSON file.
pub fn load(path: &Path) -> Load<Vec<Snippet>> {
    load_json(path)
}

/// Loads user config (categories and theme) from JSON.
pub fn load_config(path: &Path) -> Load<Config> {
    load_json(path)
}

/// Moves an unreadable data file aside (appending `.corrupt` to its name) so the
/// user can still recover it by hand, and returns the backup path. Called before
/// the seeded defaults are allowed to take over the original filename.
pub fn backup_corrupt(path: &Path) -> io::Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data file path has no file name",
        ));
    };
    let mut backup_name = name.to_os_string();
    backup_name.push(".corrupt");
    let backup = path.with_file_name(backup_name);
    std::fs::rename(path, &backup)?;
    Ok(backup)
}

/// Writes `contents` to `path` atomically: the bytes go to a temporary file in the
/// same directory, get flushed to disk, and only then replace `path` via a rename.
/// A crash, power loss, or full disk partway through a save can therefore never
/// leave a truncated `snippets.json` behind — the old file survives intact instead.
/// The temporary name includes the process id so two running copies can't collide.
fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data file path has no file name",
        ));
    };
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    let tmp = path.with_file_name(tmp_name);

    let write_tmp = |tmp: &Path| -> io::Result<()> {
        let mut file = std::fs::File::create(tmp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()
    };

    if let Err(e) = write_tmp(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    // std::fs::rename replaces an existing destination on both Windows and Unix.
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Persists the snippet library to a JSON file (pretty-printed for human readability).
pub fn save(path: &Path, snippets: &[Snippet]) -> io::Result<()> {
    let json = serde_json::to_string_pretty(snippets).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Persists user config (categories and theme) to a JSON file (pretty-printed).
pub fn save_config(path: &Path, config: &Config) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Normalizes a category name for canonical storage: trims whitespace, collapses multiple
/// spaces into single spaces, and converts to title-case word-by-word. This ensures
/// "  git ", "GIT", "Git", and "gIt" all round-trip to the same canonical "Git".
/// Used during config initialization and category creation to prevent duplicates
/// that differ only in whitespace or casing.
pub fn normalize_category(s: &str) -> String {
    let s = s.trim();
    if s.is_empty() {
        return String::new();
    }
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    // Title-case: capitalize first letter, lowercase the rest
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// True when two category names denote the same category, ignoring case.
/// Uses full Unicode lowercasing (not `eq_ignore_ascii_case`) so accented
/// categories don't sneak in as near-duplicates.
pub fn same_category(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// True for names that can't be stored as a category: blank, or "All", which is
/// reserved for the "show everything" entry in the category filter.
pub fn is_reserved_category(cat: &str) -> bool {
    cat.trim().is_empty() || same_category(cat.trim(), "All")
}

/// Normalizes a category read from disk into one the app can actually filter on:
/// blank categories and the reserved "All" become `UNCATEGORIZED`. Without this a
/// hand-edited `"category": ""` renders as an empty badge that no filter can select.
pub fn canonical_category(raw: &str) -> String {
    let cat = normalize_category(raw);
    if is_reserved_category(&cat) {
        UNCATEGORIZED.to_string()
    } else {
        cat
    }
}

/// User configuration: canonical category list and the selected theme.
/// Stored separately from snippets.json so that snippet data can remain stable
/// across version upgrades; only the config file changes when features (categories, themes) evolve.
/// Both fields use `#[serde(default)]` to handle missing fields gracefully in older config files.
#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub categories: Vec<String>, // Sorted, deduplicated list of all known categories
    #[serde(default)]
    pub theme: String, // Theme name (e.g., "Dark", "Nord"); defaults to "Dark" on first run
}

impl Config {
    /// Initializes config from an existing snippet library: extracts all unique, normalized categories
    /// and sets the theme to "Dark" by default.
    pub fn from_snippets(snippets: &[Snippet]) -> Self {
        let mut cats: Vec<String> = snippets
            .iter()
            .map(|s| normalize_category(&s.category))
            .filter(|c| !is_reserved_category(c))
            .collect();
        cats.sort();
        cats.dedup();
        Self {
            categories: cats,
            theme: "Dark".to_string(),
        }
    }

    /// Adds a category to the canonical list if it isn't already present (case-insensitive).
    /// Normalizes the input, rejects empty strings and "All" (reserved), and keeps the list sorted.
    pub fn add_category(&mut self, raw: &str) {
        let cat = normalize_category(raw);
        if is_reserved_category(&cat) {
            return;
        }
        if self.categories.iter().any(|c| same_category(c, &cat)) {
            return;
        }
        self.categories.push(cat);
        self.categories.sort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("copyit-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
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
    fn normalize_category_title_cases_and_collapses_whitespace() {
        assert_eq!(normalize_category("  git "), "Git");
        assert_eq!(normalize_category("GIT"), "Git");
        assert_eq!(normalize_category("gIt   hUb  helpers"), "Git Hub Helpers");
        assert_eq!(normalize_category("   "), "");
    }

    #[test]
    fn reserved_categories_are_rejected() {
        assert!(is_reserved_category(""));
        assert!(is_reserved_category("   "));
        assert!(is_reserved_category("all"));
        assert!(is_reserved_category("ALL"));
        assert!(is_reserved_category(" All "));
        assert!(!is_reserved_category("Git"));
        assert!(!is_reserved_category("Alliteration"));
    }

    #[test]
    fn canonical_category_maps_unusable_names_to_uncategorized() {
        // A hand-edited `"category": ""` used to render as an empty badge that no
        // filter entry could ever select.
        assert_eq!(canonical_category(""), UNCATEGORIZED);
        assert_eq!(canonical_category("   "), UNCATEGORIZED);
        assert_eq!(canonical_category("all"), UNCATEGORIZED);
        assert_eq!(canonical_category("  git "), "Git");
    }

    #[test]
    fn config_add_category_dedupes_case_insensitively() {
        let mut config = Config::default();
        config.add_category("git");
        config.add_category("GIT");
        config.add_category("  Git  ");
        config.add_category("all");
        config.add_category("");
        assert_eq!(config.categories, vec!["Git".to_string()]);
    }

    #[test]
    fn config_from_snippets_skips_reserved_categories() {
        let snippets = vec![snippet(1, "git"), snippet(2, "All"), snippet(3, "")];
        let config = Config::from_snippets(&snippets);
        assert_eq!(config.categories, vec!["Git".to_string()]);
        assert_eq!(config.theme, "Dark");
    }

    #[test]
    fn missing_file_is_distinct_from_corrupt_file() {
        let dir = tmp_dir("load-outcomes");
        let missing = dir.join("snippets.json");
        assert!(matches!(load(&missing), Load::Missing));

        // A zero-byte file holds nothing worth preserving.
        std::fs::write(&missing, "").unwrap();
        assert!(matches!(load(&missing), Load::Missing));

        // Truncated / hand-mangled JSON must NOT look like a first launch,
        // otherwise the seeded defaults silently overwrite real user data.
        std::fs::write(&missing, "[{\"id\": 1, \"title\": \"hal").unwrap();
        assert!(matches!(load(&missing), Load::Corrupt(_)));

        save(&missing, &[snippet(7, "Git")]).unwrap();
        match load(&missing) {
            Load::Loaded(snippets) => {
                assert_eq!(snippets.len(), 1);
                assert_eq!(snippets[0].id, 7);
            }
            _ => panic!("round-trip should load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backup_corrupt_preserves_the_original_bytes() {
        let dir = tmp_dir("backup");
        let path = dir.join("snippets.json");
        std::fs::write(&path, "not json").unwrap();

        let backup = backup_corrupt(&path).unwrap();
        assert!(!path.exists(), "corrupt file is moved out of the way");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "not json");
        assert_eq!(backup.file_name().unwrap(), "snippets.json.corrupt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_files() {
        let dir = tmp_dir("atomic");
        let path = dir.join("snippets.json");
        save(&path, &[snippet(1, "Git")]).unwrap();
        save(&path, &[snippet(1, "Git"), snippet(2, "Prompt")]).unwrap();

        let entries: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["snippets.json".to_string()]);

        match load(&path) {
            Load::Loaded(snippets) => assert_eq!(snippets.len(), 2),
            _ => panic!("saved file should load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_reports_an_error_instead_of_panicking_on_a_bad_path() {
        // The data directory is missing entirely: the write must fail cleanly so
        // the UI can surface it, rather than unwrapping.
        let dir = tmp_dir("bad-path");
        let path = dir.join("no-such-subdir").join("snippets.json");
        assert!(save(&path, &[snippet(1, "Git")]).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_round_trips_through_disk() {
        let dir = tmp_dir("config");
        let path = dir.join("config.json");
        assert!(matches!(load_config(&path), Load::Missing));

        let config = Config {
            categories: vec!["Git".into(), "Prompt".into()],
            theme: "Nord".into(),
        };
        save_config(&path, &config).unwrap();
        match load_config(&path) {
            Load::Loaded(loaded) => {
                assert_eq!(loaded.categories, config.categories);
                assert_eq!(loaded.theme, "Nord");
            }
            _ => panic!("config should load"),
        }

        std::fs::write(&path, "{ not json").unwrap();
        assert!(matches!(load_config(&path), Load::Corrupt(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
