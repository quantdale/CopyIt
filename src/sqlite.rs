//! Desktop engine for the shared CopyIt SQLite library.
//!
//! This module is the desktop's *only* writer of `%APPDATA%\CopyIt\copyit.db`
//! and shares the exact schema (and the same vault crypto contract) with the
//! browser-extension native host in `quantdale/CopyIt-brwsr-ext`. Keeping the
//! two schemas byte-for-byte identical is what lets the extension read snippets
//! the desktop wrote, and vice-versa.
//!
//! Design notes:
//! * `Store` (store.rs) owns the on-disk *location* and calls into this engine;
//!   `app.rs` never opens the database directly.
//! * Every write opens (creating + migrating if needed) the single canonical db
//!   file. WAL + a 3s busy_timeout let the desktop write while the host reads.
//! * Legacy JSON (`snippets.json` / `config.json`) is imported exactly once,
//!   only when no SQLite db exists yet, and the originals are renamed aside
//!   (never deleted) so a corrupt source can always be recovered.

use crate::model::Snippet;
use crate::storage::Config;
use crate::vault::VaultMeta;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DB_FILE_NAME: &str = "copyit.db";

/// Highest schema version this binary understands (must match the native host).
pub const MAX_SUPPORTED_SCHEMA_VERSION: i64 = 1;

/// Canonical V1 schema. Copied verbatim from the native host so the two writers
/// produce identical tables, indexes, and CHECK constraints.
const SCHEMA_V1: &str = r#"
CREATE TABLE schema_migrations (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    applied_at  TEXT NOT NULL
);

CREATE TABLE snippets (
    id                      INTEGER PRIMARY KEY,
    title                   TEXT NOT NULL,
    description             TEXT,
    category                TEXT NOT NULL,
    body                    TEXT NOT NULL DEFAULT '',
    protection_hint         TEXT,
    protection_nonce        TEXT,
    protection_ciphertext   TEXT,
    sort_order              INTEGER NOT NULL,
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL,

    CHECK (
      (protection_hint IS NULL AND protection_nonce IS NULL AND protection_ciphertext IS NULL)
      OR
      (protection_hint IS NOT NULL AND protection_nonce IS NOT NULL AND protection_ciphertext IS NOT NULL AND body = '')
    )
);

CREATE INDEX idx_snippets_sort_order
    ON snippets(sort_order, id);

CREATE INDEX idx_snippets_category
    ON snippets(category COLLATE NOCASE);

CREATE TABLE categories (
    name       TEXT PRIMARY KEY COLLATE NOCASE,
    sort_order INTEGER NOT NULL
);

CREATE TABLE app_config (
    singleton_id INTEGER PRIMARY KEY CHECK (singleton_id = 1),
    theme        TEXT NOT NULL,
    vault_salt   TEXT,
    vault_nonce  TEXT,
    vault_canary TEXT,
    CHECK (
      (vault_salt IS NULL AND vault_nonce IS NULL AND vault_canary IS NULL)
      OR
      (vault_salt IS NOT NULL AND vault_nonce IS NOT NULL AND vault_canary IS NOT NULL)
    )
);

CREATE TABLE migration_meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

fn now_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // RFC3339-ish; the exact format is not part of the cross-process contract.
    format!("{secs:010}Z")
}

/// Applies the shared connection pragmas (foreign keys, busy_timeout, WAL, NORMAL sync).
fn apply_pragmas(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "busy_timeout", 3000)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn schema_version(conn: &Connection) -> rusqlite::Result<i64> {
    let has: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='schema_migrations'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    if has.is_none() {
        return Ok(0);
    }
    let max: Option<i64> =
        conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })?;
    Ok(max.unwrap_or(0))
}

fn record_migration(conn: &Connection, version: i64, name: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, ?2, ?3)",
        params![version, name, now_iso()],
    )?;
    Ok(())
}

/// Creates the V1 schema on a brand-new database.
fn create_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> rusqlite::Result<()> {
        conn.execute_batch(SCHEMA_V1)?;
        record_migration(conn, 1, "initial_schema")?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Opens an existing canonical db in read-only mode. Used for loads so that a
/// read-only data directory (the real-world failure the app must survive) still
/// yields the library, while any write attempt fails cleanly elsewhere.
pub fn open_read_only(path: &Path) -> rusqlite::Result<Connection> {
    use rusqlite::OpenFlags;
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let _ = conn.pragma_update(None, "busy_timeout", 3000);
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    Ok(conn)
}

/// Opens the canonical db, creating and migrating it if it does not yet exist.
/// Returns an error (mapped by the caller) if the file exists but is unusable.
pub fn open_db(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    apply_pragmas(&conn)?;
    let current = schema_version(&conn)?;
    if current > MAX_SUPPORTED_SCHEMA_VERSION {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "unsupported schema version {current} (max {MAX_SUPPORTED_SCHEMA_VERSION})"
            )),
        ));
    }
    if current < 1 {
        create_v1(&conn)?;
    }
    Ok(conn)
}

/// Loads every snippet in display order (sort_order, then id).
pub fn load_all_snippets(conn: &Connection) -> rusqlite::Result<Vec<Snippet>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, COALESCE(description, ''), category, body,
                protection_hint, protection_nonce, protection_ciphertext
         FROM snippets ORDER BY sort_order, id",
    )?;
    let rows = stmt.query_map([], |row| {
        let hint: Option<String> = row.get(5)?;
        let nonce: Option<String> = row.get(6)?;
        let ciphertext: Option<String> = row.get(7)?;
        let protection = match (hint, nonce, ciphertext) {
            (Some(hint), Some(nonce), Some(ciphertext)) => Some(crate::model::Protection {
                hint,
                nonce,
                ciphertext,
            }),
            _ => None,
        };
        Ok(Snippet {
            id: row.get::<_, i64>(0)? as u64,
            title: row.get(1)?,
            description: row.get(2)?,
            category: row.get(3)?,
            body: row.get(4)?,
            protection,
        })
    })?;
    rows.collect()
}

fn upsert_snippet(conn: &Connection, s: &Snippet, sort_order: i64) -> rusqlite::Result<()> {
    let (hint, nonce, ciphertext, body) = match &s.protection {
        Some(p) => (Some(&p.hint), Some(&p.nonce), Some(&p.ciphertext), ""),
        None => (None, None, None, s.body.as_str()),
    };
    let ts = now_iso();
    conn.execute(
        "INSERT INTO snippets (id, title, description, category, body,
                               protection_hint, protection_nonce, protection_ciphertext,
                               sort_order, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
         ON CONFLICT(id) DO UPDATE SET
            title = excluded.title,
            description = excluded.description,
            category = excluded.category,
            body = excluded.body,
            protection_hint = excluded.protection_hint,
            protection_nonce = excluded.protection_nonce,
            protection_ciphertext = excluded.protection_ciphertext,
            sort_order = excluded.sort_order,
            updated_at = excluded.updated_at",
        params![
            s.id as i64,
            s.title,
            s.description,
            s.category,
            body,
            hint,
            nonce,
            ciphertext,
            sort_order,
            ts,
            ts
        ],
    )?;
    Ok(())
}

/// Reconciles the whole snippet table with `snips` (the app's in-memory list):
/// every snippet is upserted at its index as `sort_order`, any snippet whose id
/// is absent is deleted, and any category referenced by a snippet is ensured in
/// the `categories` table (without dropping others) so cross-references resolve.
pub fn reconcile_snippets(conn: &Connection, snips: &[Snippet]) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> rusqlite::Result<()> {
        for (i, s) in snips.iter().enumerate() {
            upsert_snippet(conn, s, i as i64)?;
            conn.execute(
                "INSERT OR IGNORE INTO categories (name, sort_order) VALUES (?1, ?2)",
                params![s.category, 0],
            )?;
        }
        if !snips.is_empty() {
            let ids: Vec<i64> = snips.iter().map(|s| s.id as i64).collect();
            let placeholders = vec!["?"; ids.len()].join(",");
            let sql = format!("DELETE FROM snippets WHERE id NOT IN ({placeholders})");
            conn.execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Replaces the entire canonical category list, preserving the given order.
pub fn set_categories(conn: &Connection, cats: &[String]) -> rusqlite::Result<()> {
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> rusqlite::Result<()> {
        conn.execute("DELETE FROM categories", [])?;
        for (i, c) in cats.iter().enumerate() {
            conn.execute(
                "INSERT OR IGNORE INTO categories (name, sort_order) VALUES (?1, ?2)",
                params![c, i as i64],
            )?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Sets the theme, preserving any existing vault triple.
pub fn set_theme(conn: &Connection, theme: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO app_config (singleton_id, theme) VALUES (1, ?1)
         ON CONFLICT(singleton_id) DO UPDATE SET theme = excluded.theme",
        params![theme],
    )?;
    Ok(())
}

/// Sets (or clears) the vault triple, preserving the theme.
pub fn set_vault_meta(conn: &Connection, meta: Option<&VaultMeta>) -> rusqlite::Result<()> {
    let (salt, nonce, canary) = match meta {
        Some(m) => (Some(&m.salt), Some(&m.nonce), Some(&m.canary)),
        None => (None, None, None),
    };
    conn.execute(
        "INSERT INTO app_config (singleton_id, theme, vault_salt, vault_nonce, vault_canary)
         VALUES (1, 'Dark', ?1, ?2, ?3)
         ON CONFLICT(singleton_id) DO UPDATE SET
            vault_salt = excluded.vault_salt,
            vault_nonce = excluded.vault_nonce,
            vault_canary = excluded.vault_canary",
        params![salt, nonce, canary],
    )?;
    Ok(())
}

/// Loads the persisted config (theme, categories, vault triple), or `None` when
/// the database has not been initialized with a config row yet.
#[allow(clippy::type_complexity)]
pub fn load_config(conn: &Connection) -> rusqlite::Result<Option<Config>> {
    let row: Option<(String, Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT theme, vault_salt, vault_nonce, vault_canary FROM app_config WHERE singleton_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((theme, salt, nonce, canary)) = row else {
        return Ok(None);
    };
    let mut stmt = conn.prepare("SELECT name FROM categories ORDER BY sort_order, name")?;
    let categories: Vec<String> = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let vault = match (salt, nonce, canary) {
        (Some(salt), Some(nonce), Some(canary)) => Some(VaultMeta {
            salt,
            nonce,
            canary,
        }),
        _ => None,
    };
    Ok(Some(Config {
        categories,
        theme,
        vault,
    }))
}

/// Resolves the canonical db path that sits beside the legacy JSON files.
pub fn db_path_for(store_dir: &Path) -> PathBuf {
    store_dir.join(DB_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Protection;

    fn memory() -> Connection {
        open_db(Path::new(":memory:")).unwrap()
    }

    #[test]
    fn round_trips_a_snippet_with_description_and_protection() {
        let conn = memory();
        let snips = vec![Snippet {
            id: 1,
            title: "T".into(),
            description: "D".into(),
            category: "Git".into(),
            body: "plain".into(),
            protection: None,
        }];
        reconcile_snippets(&conn, &snips).unwrap();
        let loaded = load_all_snippets(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].description, "D");
        assert_eq!(loaded[0].body, "plain");

        let protected = Snippet {
            id: 2,
            title: "Secret".into(),
            description: String::new(),
            category: "Git".into(),
            body: String::new(),
            protection: Some(Protection {
                hint: "h".into(),
                nonce: "n".into(),
                ciphertext: "c".into(),
            }),
        };
        reconcile_snippets(&conn, &[snips[0].clone(), protected]).unwrap();
        let loaded = load_all_snippets(&conn).unwrap();
        assert_eq!(loaded.len(), 2);
        assert!(loaded[1].protection.is_some());
        assert_eq!(loaded[1].body, "");
    }

    #[test]
    fn delete_is_reflected_by_reconcile() {
        let conn = memory();
        let snips = vec![
            Snippet {
                id: 1,
                title: "A".into(),
                description: String::new(),
                category: "Git".into(),
                body: "a".into(),
                protection: None,
            },
            Snippet {
                id: 2,
                title: "B".into(),
                description: String::new(),
                category: "Git".into(),
                body: "b".into(),
                protection: None,
            },
        ];
        reconcile_snippets(&conn, &snips).unwrap();
        reconcile_snippets(&conn, &snips[..1]).unwrap();
        let loaded = load_all_snippets(&conn).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, 1);
    }

    #[test]
    fn config_round_trips() {
        let conn = memory();
        set_theme(&conn, "Nord").unwrap();
        set_categories(&conn, &["Git".into(), "Prompt".into()]).unwrap();
        set_vault_meta(
            &conn,
            Some(&VaultMeta {
                salt: "s".into(),
                nonce: "n".into(),
                canary: "c".into(),
            }),
        )
        .unwrap();
        let cfg = load_config(&conn).unwrap().unwrap();
        assert_eq!(cfg.theme, "Nord");
        assert_eq!(
            cfg.categories,
            vec!["Git".to_string(), "Prompt".to_string()]
        );
        assert!(cfg.vault.is_some());
    }
}
