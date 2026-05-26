//! Persistent storage for watched status, playback positions, and settings.
//!
//! Backed by a bundled SQLite database (`yt-offline.db`). Access goes through
//! a small `r2d2`-managed pool of connections rather than a single shared
//! `Connection` — that way concurrent read queries from different axum
//! handlers don't serialize on a mutex, and write queries still take their
//! turn via SQLite's own per-connection locking.
//!
//! # Schema
//!
//! | Table | Columns | Purpose |
//! |---|---|---|
//! | `watched` | `video_id` (PK), `watched_at` | Records videos the user has marked watched |
//! | `positions` | `video_id` (PK), `position_secs`, `updated_at` | Stores resume positions |
//! | `settings` | `key` (PK), `value` | Persistent app settings (password hash, etc.) |

use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, Result};
use std::collections::{HashMap, HashSet};
use std::path::Path;

type Pool = r2d2::Pool<SqliteConnectionManager>;
type PooledConn = r2d2::PooledConnection<SqliteConnectionManager>;

/// In-memory representation of the `video_flags` table. Each set holds the
/// video IDs that have the named flag enabled — kept small (a few hundred
/// to a few thousand entries in practice).
#[derive(Default, Clone, Debug)]
pub struct VideoFlagsBundle {
    pub bookmark: HashSet<String>,
    pub favourite: HashSet<String>,
    pub waiting: HashSet<String>,
    pub archive: HashSet<String>,
}

/// Default pool size for a file-backed database. Small intentionally — the
/// app is single-user and our queries are short. A handful is plenty.
const FILE_POOL_SIZE: u32 = 4;

/// Thin wrapper around an `r2d2` SQLite pool with schema management.
///
/// Construction always returns a pool with at least one usable connection
/// and the schema initialised. Subsequent method calls borrow a connection,
/// run their query, and return it — no external `Mutex` is needed.
pub struct Database {
    pool: Pool,
}

impl Database {
    /// Open or create the database at `path`, running schema migrations.
    ///
    /// On Unix the file mode is tightened to `0600` so the Argon2 password
    /// hash and resume positions aren't readable by other local users. A
    /// best-effort: failure is logged but doesn't abort startup.
    pub fn open(path: &Path) -> Result<Self> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::builder()
            .max_size(FILE_POOL_SIZE)
            .build(manager)
            .map_err(pool_init_to_rusqlite)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(path) {
                let mut perms = meta.permissions();
                if perms.mode() & 0o777 != 0o600 {
                    perms.set_mode(0o600);
                    let _ = std::fs::set_permissions(path, perms);
                }
            }
        }

        let db = Database { pool };
        db.init_schema()?;
        Ok(db)
    }

    /// Open an in-memory database — used in tests and as a fallback when the
    /// real file can't be opened.
    ///
    /// In-memory SQLite databases are per-connection by default, so the pool
    /// is capped at 1 connection here. Otherwise each `get()` would hand back
    /// a fresh, empty database and our schema/data would vanish between calls.
    pub fn open_in_memory() -> Result<Self> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(pool_init_to_rusqlite)?;
        let db = Database { pool };
        db.init_schema()?;
        Ok(db)
    }

    /// Acquire a connection from the pool. Panics on pool failure — these
    /// are effectively unrecoverable (the SQLite file vanished, the disk is
    /// full / read-only, or the pool is exhausted under runaway load).
    fn conn(&self) -> PooledConn {
        self.pool.get().expect("db pool checkout failed")
    }

    fn init_schema(&self) -> Result<()> {
        self.conn().execute_batch(
            "CREATE TABLE IF NOT EXISTS watched (
                video_id TEXT PRIMARY KEY,
                watched_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS positions (
                video_id TEXT PRIMARY KEY,
                position_secs REAL NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS channel_options (
                platform TEXT NOT NULL,
                handle   TEXT NOT NULL,
                options_json TEXT NOT NULL,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (platform, handle)
            );
            CREATE TABLE IF NOT EXISTS video_flags (
                video_id  TEXT PRIMARY KEY,
                bookmark  INTEGER NOT NULL DEFAULT 0,
                favourite INTEGER NOT NULL DEFAULT 0,
                waiting   INTEGER NOT NULL DEFAULT 0,
                archive   INTEGER NOT NULL DEFAULT 0,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        Ok(())
    }

    /// Fetch the raw JSON-encoded download-options blob for a channel.
    /// `platform` is the [`crate::platform::Platform::dir_name`] string;
    /// `handle` is the on-disk folder name. Returns `None` when no options
    /// row exists.
    pub fn get_channel_options(&self, platform: &str, handle: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT options_json FROM channel_options WHERE platform = ?1 AND handle = ?2",
        )?;
        let mut rows = stmt.query([platform, handle])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Upsert the download-options JSON blob for a channel.
    pub fn set_channel_options(&self, platform: &str, handle: &str, options_json: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO channel_options (platform, handle, options_json, updated_at)
             VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP)",
            [platform, handle, options_json],
        )?;
        Ok(())
    }

    /// Delete a channel's options row, falling its behavior back to global defaults.
    pub fn delete_channel_options(&self, platform: &str, handle: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM channel_options WHERE platform = ?1 AND handle = ?2",
            [platform, handle],
        )?;
        Ok(())
    }

    /// Set or clear a single per-video flag. `flag` must be one of
    /// `"bookmark"`, `"favourite"`, `"waiting"`, `"archive"` — the only
    /// columns in `video_flags`. Returns an error if `flag` is unknown so a
    /// typo doesn't silently no-op.
    ///
    /// A row is upserted on first call; once any flag on a video is set,
    /// the row sticks around even after all flags are cleared. This keeps
    /// the schema simple at the cost of a few orphan rows.
    pub fn set_video_flag(&self, video_id: &str, flag: &str, value: bool) -> Result<()> {
        let col = match flag {
            "bookmark" | "favourite" | "waiting" | "archive" => flag,
            _ => return Err(rusqlite::Error::InvalidParameterName(flag.to_string())),
        };
        let conn = self.conn();
        // SQLite doesn't allow parameterised column names; we validated `col`
        // against an allow-list above so direct interpolation is safe.
        let sql = format!(
            "INSERT INTO video_flags (video_id, {col}) VALUES (?1, ?2)
             ON CONFLICT(video_id) DO UPDATE SET {col} = ?2, updated_at = CURRENT_TIMESTAMP"
        );
        conn.execute(&sql, rusqlite::params![video_id, value as i32])?;
        Ok(())
    }

    /// Bulk fetch every video's flag set, grouped by flag. Used at startup +
    /// after rescan to hydrate the in-memory caches. The four returned sets
    /// hold the IDs of videos with each flag set to true.
    pub fn get_video_flags(&self) -> Result<VideoFlagsBundle> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT video_id, bookmark, favourite, waiting, archive FROM video_flags",
        )?;
        let mut bundle = VideoFlagsBundle::default();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)? != 0,
                row.get::<_, i32>(2)? != 0,
                row.get::<_, i32>(3)? != 0,
                row.get::<_, i32>(4)? != 0,
            ))
        })?;
        for row in rows.flatten() {
            let (id, b, f, w, a) = row;
            if b { bundle.bookmark.insert(id.clone()); }
            if f { bundle.favourite.insert(id.clone()); }
            if w { bundle.waiting.insert(id.clone()); }
            if a { bundle.archive.insert(id.clone()); }
        }
        Ok(bundle)
    }

    /// Bulk fetch of every channel's options, returned as
    /// `((platform, handle) → options_json)`. Used by the library scanner to
    /// attach options to each scanned [`crate::library::Channel`] without
    /// per-channel SQL round trips.
    pub fn get_all_channel_options(&self) -> Result<HashMap<(String, String), String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT platform, handle, options_json FROM channel_options")?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(std::result::Result::ok)
            .map(|(k, v)| (k, v))
            .collect();
        Ok(map)
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT value FROM settings WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    pub fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let conn = self.conn();
        match value {
            Some(v) => {
                conn.execute(
                    "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                    [key, v],
                )?;
            }
            None => {
                conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
            }
        }
        Ok(())
    }

    pub fn set_watched(&self, video_id: &str, watched: bool) -> Result<()> {
        let conn = self.conn();
        if watched {
            conn.execute(
                "INSERT OR REPLACE INTO watched (video_id) VALUES (?1)",
                [video_id],
            )?;
        } else {
            conn.execute("DELETE FROM watched WHERE video_id = ?1", [video_id])?;
        }
        Ok(())
    }

    pub fn get_watched(&self) -> Result<HashSet<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT video_id FROM watched")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(ids)
    }

    pub fn set_position(&self, video_id: &str, position_secs: f64) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR REPLACE INTO positions (video_id, position_secs, updated_at)
             VALUES (?1, ?2, CURRENT_TIMESTAMP)",
            rusqlite::params![video_id, position_secs],
        )?;
        Ok(())
    }

    pub fn clear_position(&self, video_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("DELETE FROM positions WHERE video_id = ?1", [video_id])?;
        Ok(())
    }

    pub fn get_positions(&self) -> Result<HashMap<String, f64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT video_id, position_secs FROM positions")?;
        let map = stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)))?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(map)
    }
}

/// Translate an `r2d2::Error` from `Pool::build()` into a `rusqlite::Error` so
/// callers don't have to juggle two error types. Pool-init failures are rare
/// (bad file path, OS-level problem) and the surfaced error message is what
/// matters; the variant is incidental.
fn pool_init_to_rusqlite(e: r2d2::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        format!("sqlite pool init failed: {e}"),
    )))
}

// `Connection` is still imported for the type alias path; suppress the
// unused-import warning when no caller references it directly.
#[allow(dead_code)]
type _SilenceConnectionImport = Connection;
