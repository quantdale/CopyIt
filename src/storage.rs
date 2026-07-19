use crate::model::Snippet;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;

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

pub fn load(path: &PathBuf) -> Option<Vec<Snippet>> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save(path: &PathBuf, snippets: &[Snippet]) -> io::Result<()> {
    let json = serde_json::to_string_pretty(snippets)
        .map_err(io::Error::other)?;
    std::fs::write(path, json)
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

/// User configuration: canonical category list and the selected theme.
/// Stored separately from snippets.json so that snippet data can remain stable
/// across version upgrades; only the config file changes when features (categories, themes) evolve.
/// Both fields use `#[serde(default)]` to handle missing fields gracefully in older config files.
#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub categories: Vec<String>,  // Sorted, deduplicated list of all known categories
    #[serde(default)]
    pub theme: String,            // Theme name (e.g., "Dark", "Nord"); defaults to "Dark" on first run
}

impl Config {
    pub fn from_snippets(snippets: &[Snippet]) -> Self {
        let mut cats: Vec<String> = snippets
            .iter()
            .map(|s| normalize_category(&s.category))
            .filter(|c| !c.is_empty())
            .collect();
        cats.sort();
        cats.dedup();
        Self {
            categories: cats,
            theme: "Dark".to_string(),
        }
    }

    pub fn add_category(&mut self, raw: &str) {
        let cat = normalize_category(raw);
        if cat.is_empty() || cat.eq_ignore_ascii_case("all") {
            return;
        }
        if self
            .categories
            .iter()
            .any(|c| c.to_lowercase() == cat.to_lowercase())
        {
            return;
        }
        self.categories.push(cat);
        self.categories.sort();
    }
}

pub fn load_config(path: &PathBuf) -> Option<Config> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save_config(path: &PathBuf, config: &Config) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config)
        .map_err(io::Error::other)?;
    std::fs::write(path, json)
}
