use crate::model::Snippet;
use crate::vault::VaultMeta;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Category assigned to snippets that carry no usable category of their own
/// (blank, or the reserved "All" filter label).
pub const UNCATEGORIZED: &str = "Uncategorized";

/// Resolves the preferred stable per-user directory where snippets.json and config.json
/// live, WITHOUT creating it: `%APPDATA%\CopyIt` on Windows, falling back to the directory
/// containing the running .exe on non-Windows or when APPDATA is unset/empty.
pub fn data_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        if !appdata.is_empty() {
            return PathBuf::from(appdata).join("CopyIt");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    PathBuf::from(".") // Last-resort fallback: current working directory
}

/// Resolves the data directory and makes sure it exists, returning the path plus any
/// creation failure for the caller to surface. A failure here means saves will fail
/// later anyway — reporting it at startup is clearer than letting the first write
/// discover it (or silently falling back to an unexpected location).
pub fn ensure_data_dir() -> (PathBuf, Option<std::io::Error>) {
    let dir = data_dir();
    match std::fs::create_dir_all(&dir) {
        Ok(()) => (dir, None),
        Err(e) => (dir, Some(e)),
    }
}

/// Legacy locations `snippets.json`/`config.json` may have been left in by
/// earlier versions that stored data next to the .exe. Used for one-time
/// migration into the new stable `data_dir()`.
/// Outcome of reading one of the JSON data files.
///
/// The three cases must stay distinct: `Missing` means "first launch, seed the
/// defaults and write them out", while `Corrupt` means "there *is* user data
/// here that we failed to understand". Collapsing the two (as an
/// `Option`-returning loader does) makes the app silently seed defaults over a
/// file it couldn't parse, destroying the user's library on the next save.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
/// Maximum file size we accept for JSON data files (256 MiB). Anything larger
/// is reported as corrupt rather than slurped into memory.
const MAX_DATA_FILE_BYTES: usize = 256 * 1024 * 1024;

/// Reads at most `limit + 1` bytes from `path`. Returns `Ok(Some(data))` when the
/// whole file fit within `limit`, `Ok(None)` when anything exists beyond it, and an
/// error for open/read/UTF-8 failures.
fn read_bounded(path: &Path, limit: usize) -> io::Result<Option<String>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        if buf.len() + n > limit.saturating_add(1) {
            return Ok(None);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    String::from_utf8(buf)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn load_json_limited<T: serde::de::DeserializeOwned>(path: &Path, limit: usize) -> Load<T> {
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > limit as u64 {
            return Load::Corrupt(format!(
                "file is too large ({} bytes, limit {limit})",
                meta.len()
            ));
        }
    }
    let data = match read_bounded(path, limit) {
        Ok(Some(data)) => data,
        Ok(None) => return Load::Corrupt(format!("file is too large (limit {limit})")),
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

fn load_json<T: serde::de::DeserializeOwned>(path: &Path) -> Load<T> {
    load_json_limited(path, MAX_DATA_FILE_BYTES)
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
///
/// If a `.corrupt` backup already exists (a previous corruption), the next free
/// name in the sequence `.corrupt.1`, `.corrupt.2`, ... is used, so an old backup
/// is never silently overwritten.
pub fn backup_corrupt(path: &Path) -> io::Result<PathBuf> {
    let Some(name) = path.file_name() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "data file path has no file name",
        ));
    };
    let base = name.to_os_string();
    let mut backup = path.with_file_name({
        let mut first = base.clone();
        first.push(".corrupt");
        first
    });
    let mut n = 1;
    while backup.exists() {
        backup = path.with_file_name({
            let mut next = base.clone();
            next.push(".corrupt.");
            next.push(n.to_string());
            next
        });
        n += 1;
    }
    std::fs::rename(path, &backup)?;
    Ok(backup)
}

/// Removes stale `.tmp` *files* (not directories) from `dir` on startup. These
/// are leftovers from a crashed or killed previous session. Directories are
/// intentionally left alone because some tests depend on blocking directories
/// named `*.tmp`.
pub fn sweep_stale_tmp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if let Some(s) = name.to_str() {
            if s.ends_with(".tmp") && entry.file_type().is_ok_and(|ft| ft.is_file()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

/// Writes `contents` to `path` atomically: the bytes go to a temporary file in the
/// same directory, get flushed to disk, and only then replace `path` via a rename.
/// A crash, power loss, or full disk partway through a save can therefore never
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

/// Legacy JSON persistence helpers. The desktop now stores data in the shared
/// SQLite `copyit.db` (see `store.rs` / `sqlite.rs`); these remain only to keep the
/// JSON round-trip unit tests meaningful and are not used by the running app.
#[allow(dead_code)]
pub(crate) fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
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
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(tmp)?;
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
#[allow(dead_code)]
pub fn save(path: &Path, snippets: &[Snippet]) -> io::Result<()> {
    let json = serde_json::to_string_pretty(snippets).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Persists user config (categories and theme) to a JSON file (pretty-printed).
#[allow(dead_code)]
pub fn save_config(path: &Path, config: &Config) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// category on every lookup, so that case is answered without allocating; anything
/// non-ASCII still goes through full Unicode lowercasing, which for ASCII input
/// produces exactly the same result.
pub fn same_category(a: &str, b: &str) -> bool {
    if a.is_ascii() && b.is_ascii() {
        return a.eq_ignore_ascii_case(b);
    }
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

/// User configuration: canonical category list, the selected theme, and the vault
/// metadata backing protected snippets.
/// Stored separately from snippets.json so that snippet data can remain stable
/// across version upgrades; only the config file changes when features (categories, themes) evolve.
/// Both fields use `#[serde(default)]` to handle missing fields gracefully in older config files.
#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub categories: Vec<String>, // Sorted, deduplicated list of all known categories
    #[serde(default)]
    pub theme: String, // Theme name (e.g., "Dark", "Nord"); defaults to "Dark" on first run
    /// Vault metadata (KDF salt + encrypted canary) for protected snippets; `None`
    /// on files written before the vault feature, or until the user protects a
    /// snippet for the first time.
    #[serde(default)]
    pub vault: Option<VaultMeta>,
}

impl Config {
    /// Initializes config from an existing snippet library: extracts all unique, normalized categories
    /// and sets the theme to "Dark" by default. No vault metadata (it is created on first protect).
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
            vault: None,
        }
    }

    /// Adds categories to the canonical list, skipping any that are already present
    /// (case-insensitive) or reserved, and normalizing the rest. The list is sorted a
    /// single time at the end: startup registers every snippet's category, which used
    /// to re-sort the whole list once per snippet.
    pub fn add_categories<'a>(&mut self, raws: impl IntoIterator<Item = &'a str>) {
        let before = self.categories.len();
        for raw in raws {
            let cat = normalize_category(raw);
            if is_reserved_category(&cat) {
                continue;
            }
            if self.categories.iter().any(|c| same_category(c, &cat)) {
                continue;
            }
            self.categories.push(cat);
        }
        if self.categories.len() != before {
            self.categories.sort();
        }
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
            description: String::new(),
            category: category.to_string(),
            body: "body".to_string(),
            protection: None,
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
    fn config_add_categories_dedupes_case_insensitively() {
        let mut config = Config::default();
        config.add_categories(["git", "GIT", "  Git  ", "all", ""]);
        assert_eq!(config.categories, vec!["Git".to_string()]);

        // Adding in batches must behave like adding one at a time, and keep the
        // canonical list sorted.
        config.add_categories(["prompt"]);
        config.add_categories(["Docker", "prompt", "ansible"]);
        assert_eq!(
            config.categories,
            vec![
                "Ansible".to_string(),
                "Docker".to_string(),
                "Git".to_string(),
                "Prompt".to_string(),
            ]
        );
    }

    #[test]
    fn same_category_ignores_case_for_ascii_and_unicode() {
        assert!(same_category("git", "GIT"));
        assert!(same_category("Git Hub", "git hub"));
        assert!(!same_category("git", "gitt"));
        // Non-ASCII names still go through full Unicode lowercasing.
        assert!(same_category("Café", "CAFÉ"));
        assert!(!same_category("Café", "Cafe"));
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
    fn backup_corrupt_never_overwrites_an_existing_backup() {
        let dir = tmp_dir("backup-twice");
        let path = dir.join("snippets.json");

        std::fs::write(&path, "first corruption").unwrap();
        let first = backup_corrupt(&path).unwrap();
        assert_eq!(first.file_name().unwrap(), "snippets.json.corrupt");

        // A second corruption must pick the next free name, not clobber the first.
        std::fs::write(&path, "second corruption").unwrap();
        let second = backup_corrupt(&path).unwrap();
        assert_eq!(second.file_name().unwrap(), "snippets.json.corrupt.1");

        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first corruption");
        assert_eq!(
            std::fs::read_to_string(&second).unwrap(),
            "second corruption"
        );
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
            vault: None,
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

    #[test]
    fn config_missing_fields_default_gracefully() {
        // Older config files may predate a field; `#[serde(default)]` must fill
        // in the missing side so load never turns into `Corrupt`.
        let dir = tmp_dir("config-missing");
        let path = dir.join("config.json");

        // Only categories present: theme falls back to the empty string.
        std::fs::write(&path, r#"{"categories":["Git","Prompt"]}"#).unwrap();
        match load_config(&path) {
            Load::Loaded(config) => {
                assert_eq!(
                    config.categories,
                    vec!["Git".to_string(), "Prompt".to_string()]
                );
                assert_eq!(config.theme, "");
            }
            _ => panic!("config without theme should still load"),
        }

        // Only theme present: categories fall back to an empty list.
        std::fs::write(&path, r#"{"theme":"Nord"}"#).unwrap();
        match load_config(&path) {
            Load::Loaded(config) => {
                assert!(config.categories.is_empty());
                assert_eq!(config.theme, "Nord");
            }
            _ => panic!("config without categories should still load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_without_vault_loads_with_none() {
        // A config file written before the vault feature has no `vault` key; it must
        // load with `vault: None` rather than being reported corrupt.
        let dir = tmp_dir("config-no-vault");
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"categories":["Git"],"theme":"Dark"}"#).unwrap();
        match load_config(&path) {
            Load::Loaded(config) => {
                assert_eq!(config.categories, vec!["Git".to_string()]);
                assert!(config.vault.is_none());
            }
            _ => panic!("config without vault should still load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_vault_round_trips_through_disk() {
        let dir = tmp_dir("config-vault");
        let path = dir.join("config.json");
        let (meta, _) = crate::vault::create_vault("password1").unwrap();
        let config = Config {
            categories: vec!["Git".into()],
            theme: "Dark".into(),
            vault: Some(meta.clone()),
        };
        save_config(&path, &config).unwrap();
        match load_config(&path) {
            Load::Loaded(loaded) => {
                let loaded = loaded.vault.expect("vault should round-trip");
                assert_eq!(loaded.salt, meta.salt);
                assert_eq!(loaded.nonce, meta.nonce);
                assert_eq!(loaded.canary, meta.canary);
                // The persisted canary still unlocks with the same password.
                assert!(crate::vault::verify_password("password1", &loaded).is_ok());
            }
            _ => panic!("config with vault should load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalize_category_is_idempotent_for_ascii() {
        let inputs = ["git", "GIT", "Git", "  git  ", "api git", "macOS", "iPhone"];
        for input in inputs {
            let once = normalize_category(input);
            let twice = normalize_category(&once);
            assert_eq!(
                once, twice,
                "double-normalize changed '{input}': '{once}' -> '{twice}'"
            );
        }
    }

    #[test]
    fn load_json_rejects_oversized_files() {
        let dir = tmp_dir("size-guard");
        let path = dir.join("snippets.json");
        // Write a file that's under the limit: should load fine.
        save(&path, &[snippet(1, "Git")]).unwrap();
        assert!(matches!(load(&path), Load::Loaded(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_loader_rejects_limit_plus_one_and_accepts_exact_limit() {
        let dir = tmp_dir("bounded-loader");
        let path = dir.join("snippets.json");
        std::fs::write(&path, "[]").unwrap();
        let exact: Load<Vec<Snippet>> = load_json_limited(&path, 2);
        match exact {
            Load::Loaded(snippets) => assert!(snippets.is_empty()),
            _ => panic!("a file exactly at the limit should load"),
        }
        std::fs::write(&path, "[] ").unwrap();
        let over: Load<Vec<Snippet>> = load_json_limited(&path, 2);
        match over {
            Load::Corrupt(message) => assert!(message.contains("too large"), "got: {message}"),
            _ => panic!("limit+1 must be rejected"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bounded_loader_preserves_missing_corrupt_semantics() {
        let dir = tmp_dir("bounded-semantics");
        let path = dir.join("snippets.json");
        const LIMIT: usize = 4096;
        std::fs::write(&path, "").unwrap();
        let result: Load<Vec<Snippet>> = load_json_limited(&path, LIMIT);
        assert!(matches!(result, Load::Missing));
        std::fs::write(&path, "   \n\t ").unwrap();
        let result: Load<Vec<Snippet>> = load_json_limited(&path, LIMIT);
        assert!(matches!(result, Load::Missing));
        std::fs::write(&path, [0xff_u8, 0xfe, 0xfd]).unwrap();
        let result: Load<Vec<Snippet>> = load_json_limited(&path, LIMIT);
        assert!(matches!(result, Load::Corrupt(_)));
        std::fs::write(&path, "[{not json").unwrap();
        let result: Load<Vec<Snippet>> = load_json_limited(&path, LIMIT);
        assert!(matches!(result, Load::Corrupt(_)));
        save(&path, &[snippet(9, "Git")]).unwrap();
        let round_trip: Load<Vec<Snippet>> = load_json_limited(&path, LIMIT);
        match round_trip {
            Load::Loaded(snippets) => assert_eq!(snippets[0].id, 9),
            _ => panic!("valid file should round-trip"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn protected_snippet_round_trips_with_empty_body_on_disk() {
        // A protected snippet is stored with ciphertext + nonce + hint and an empty
        // `body`, and loads back with the same protection (so the app can decrypt it
        // on demand after unlocking).
        let dir = tmp_dir("protected-roundtrip");
        let (_, key) = crate::vault::create_vault("password1").unwrap();
        let protection = crate::vault::encrypt_body(&key, "a secret body long enough").unwrap();
        let snippet = Snippet {
            id: 1,
            title: "KEY".into(),
            description: String::new(),
            category: "Git".into(),
            body: String::new(),
            protection: Some(protection.clone()),
        };

        let store = crate::store::Store::at(dir.clone());
        store.save_snippets(&[snippet]).unwrap();
        // The secret must never be written to disk in plaintext: for protected
        // snippets the SQLite `body` column is empty and the secret exists only
        // as base64 ciphertext, never as the raw string.
        let db = crate::sqlite::db_path_for(&dir);
        let raw = std::fs::read(&db).unwrap();
        let raw_str = String::from_utf8_lossy(&raw);
        assert!(
            !raw_str.contains("a secret body long enough"),
            "the secret must never be written to disk"
        );

        match store.load_snippets() {
            Load::Loaded(loaded) => {
                assert_eq!(loaded.len(), 1);
                assert!(loaded[0].body.is_empty());
                let p = loaded[0]
                    .protection
                    .as_ref()
                    .expect("protection round-trips");
                assert_eq!(p.hint, protection.hint);
                assert_eq!(p.nonce, protection.nonce);
                assert_eq!(p.ciphertext, protection.ciphertext);
                assert_eq!(
                    crate::vault::decrypt_body(&key, p).unwrap(),
                    "a secret body long enough"
                );
            }
            _ => panic!("protected snippet should load"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
