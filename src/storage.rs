use crate::model::Snippet;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where the library lives: `snippets.json` next to the .exe (portable).
/// Falls back to the current working directory if the exe path can't be found.
pub fn data_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("snippets.json");
        }
    }
    PathBuf::from("snippets.json")
}

/// Small config file next to the .exe holding canonical categories and the
/// selected theme. Kept separate from `snippets.json` so snippet data stays
/// backward-compatible with earlier versions.
pub fn config_path() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("config.json");
        }
    }
    PathBuf::from("config.json")
}

pub fn load(path: &PathBuf) -> Option<Vec<Snippet>> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

pub fn save(path: &PathBuf, snippets: &[Snippet]) {
    if let Ok(json) = serde_json::to_string_pretty(snippets) {
        let _ = std::fs::write(path, json);
    }
}

/// Trim whitespace and normalize category casing to title-case.
/// "  git " and "GIT" both become "Git".
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
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub theme: String,
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
        if cat.is_empty() {
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

pub fn save_config(path: &PathBuf, config: &Config) {
    if let Ok(json) = serde_json::to_string_pretty(config) {
        let _ = std::fs::write(path, json);
    }
}
