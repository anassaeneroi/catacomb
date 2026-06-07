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

/// Persisted folder row from the `folders` table. Used by the sidebar +
/// folder-management UI; channels reference folders via the separate
/// `channel_assignments` table.
#[derive(Clone, Debug, serde::Serialize)]
pub struct FolderRecord {
    pub id: i64,
    pub name: String,
    pub position: i64,
    /// Parent folder id for N-level nesting. `None` = top-level folder.
    pub parent_id: Option<i64>,
}

/// One video's searchable fields, fed to [`Database::sync_search_index`].
/// `description_path` is read lazily — only when the video is new or its
/// `mtime_unix` changed since the last index — so a routine rescan doesn't
/// re-read every description sidecar.
#[derive(Clone, Debug)]
pub struct SearchEntry {
    pub video_id: String,
    pub mtime_unix: i64,
    pub platform: String,
    pub channel: String,
    pub title: String,
    pub description_path: Option<std::path::PathBuf>,
}

/// A full-text search result row from [`Database::search_videos`].
#[derive(Debug, Clone, serde::Serialize)]
pub struct SearchHit {
    pub video_id: String,
    pub platform: String,
    pub channel: String,
    pub title: String,
    /// Description excerpt with the matched terms wrapped in `[`…`]`.
    pub snippet: String,
}

/// Build a safe FTS5 MATCH expression from free-form user input: each
/// whitespace token becomes a quoted prefix term, AND-ed together. Quoting
/// neutralizes FTS5 operators in the input; the trailing `*` gives
/// type-ahead prefix matching. Returns "" when nothing is searchable.
fn fts_match_expr(query: &str) -> String {
    query
        .split_whitespace()
        .map(|t| t.replace('"', " "))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|t| format!("\"{t}\"*"))
        .collect::<Vec<_>>()
        .join(" ")
}

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
///
/// `Clone` is cheap: the inner `r2d2::Pool` is an `Arc` so we hand out
/// new references, not new pools. This lets the library scanner take
/// its own handle for per-thread cache lookups during parallel scans.
#[derive(Clone)]
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
            );
            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL UNIQUE,
                position INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE IF NOT EXISTS channel_assignments (
                platform TEXT NOT NULL,
                handle   TEXT NOT NULL,
                folder_id INTEGER NOT NULL,
                PRIMARY KEY (platform, handle),
                FOREIGN KEY (folder_id) REFERENCES folders(id) ON DELETE CASCADE
            );
            -- Cache of parsed info.json fields keyed by the file's absolute
            -- path + mtime. Library scans hit this first; on miss they
            -- parse the JSON and upsert here so the next scan is free.
            -- The keyed-by-mtime invalidation means yt-dlp re-writing an
            -- info.json (e.g. after a metadata refresh) auto-invalidates
            -- without explicit eviction.
            CREATE TABLE IF NOT EXISTS info_cache (
                path          TEXT PRIMARY KEY,
                mtime_unix    INTEGER NOT NULL,
                duration_secs REAL,
                has_chapters  INTEGER NOT NULL DEFAULT 0,
                upload_date   TEXT
            );
            -- Free-text user annotations on a channel or a video.
            -- `target_kind` is 'channel' or 'video'; `target_id` is
            -- 'platform/handle' for channels or the video ID for videos.
            -- An empty body is treated as 'no note' and deleted rather
            -- than stored, so the table only holds rows the user cares
            -- about.
            CREATE TABLE IF NOT EXISTS notes (
                target_kind TEXT NOT NULL,
                target_id   TEXT NOT NULL,
                body        TEXT NOT NULL,
                updated_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (target_kind, target_id)
            );
            -- Full-text search over the library. `video_search` is a standalone
            -- FTS5 index (available in rusqlite's bundled SQLite); `search_meta`
            -- tracks each indexed video's mtime so [`Database::sync_search_index`]
            -- only re-reads a description sidecar when the video actually
            -- changed. video_id/platform are UNINDEXED — stored for retrieval,
            -- not matched.
            CREATE VIRTUAL TABLE IF NOT EXISTS video_search USING fts5(
                video_id UNINDEXED,
                platform UNINDEXED,
                channel,
                title,
                description,
                tokenize = 'porter unicode61'
            );
            CREATE TABLE IF NOT EXISTS search_meta (
                video_id   TEXT PRIMARY KEY,
                mtime_unix INTEGER NOT NULL
            );",
        )?;

        // ── Migration: folders.parent_id (N-level nesting) ───────────────
        // `folders` predates nesting. Add the column idempotently — SQLite
        // has no `ADD COLUMN IF NOT EXISTS`, so we attempt the ALTER and
        // swallow the "duplicate column name" error that fires on an
        // already-migrated DB. NULL parent_id = top-level folder.
        let conn = self.conn();
        match conn.execute("ALTER TABLE folders ADD COLUMN parent_id INTEGER REFERENCES folders(id) ON DELETE CASCADE", []) {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(_, Some(msg)))
                if msg.contains("duplicate column") => {}
            Err(e) => return Err(e),
        }
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

    /// Create a new folder with the given name. Returns the new folder's id.
    /// Trying to insert a duplicate name surfaces the SQLite UNIQUE error.
    pub fn create_folder(&self, name: &str) -> Result<i64> {
        let conn = self.conn();
        conn.execute("INSERT INTO folders (name) VALUES (?1)", [name])?;
        Ok(conn.last_insert_rowid())
    }

    /// Rename an existing folder. No-op when the new name already matches.
    pub fn rename_folder(&self, id: i64, new_name: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute("UPDATE folders SET name = ?1 WHERE id = ?2", rusqlite::params![new_name, id])?;
        Ok(())
    }

    /// Delete a folder. Associated channel_assignments rows cascade-delete
    /// via the foreign-key constraint, so each member channel reverts to
    /// "Unfiled".
    pub fn delete_folder(&self, id: i64) -> Result<()> {
        let conn = self.conn();
        // Enable FK cascade for this connection — SQLite has it off by default.
        conn.execute("PRAGMA foreign_keys = ON", [])?;
        conn.execute("DELETE FROM folders WHERE id = ?1", [id])?;
        Ok(())
    }

    /// List every folder, ordered by `position` then `id` so the sidebar
    /// has a deterministic order even before drag-to-reorder ships.
    pub fn list_folders(&self) -> Result<Vec<FolderRecord>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT id, name, position, parent_id FROM folders ORDER BY position, id")?;
        let rows = stmt.query_map([], |row| {
            Ok(FolderRecord {
                id: row.get(0)?,
                name: row.get(1)?,
                position: row.get(2)?,
                parent_id: row.get(3)?,
            })
        })?;
        Ok(rows.filter_map(std::result::Result::ok).collect())
    }

    /// Reparent a folder. `new_parent = None` makes it top-level.
    ///
    /// Refuses to create a cycle: a folder can't become its own ancestor.
    /// We walk up from the proposed new parent; if we hit `id` along the
    /// way the move would form a loop and we return a friendly error
    /// instead of corrupting the tree.
    pub fn set_folder_parent(&self, id: i64, new_parent: Option<i64>) -> Result<()> {
        let conn = self.conn();
        if let Some(parent) = new_parent {
            if parent == id {
                return Err(rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                    Some("a folder can't be its own parent".into()),
                ));
            }
            // Walk ancestors of `parent`; if `id` appears, this would cycle.
            let mut cur = Some(parent);
            let mut guard = 0;
            while let Some(c) = cur {
                if c == id {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT),
                        Some("that move would nest a folder inside its own descendant".into()),
                    ));
                }
                // Defensive bound in case the table is already corrupt.
                guard += 1;
                if guard > 10_000 { break; }
                cur = match conn.query_row(
                    "SELECT parent_id FROM folders WHERE id = ?1",
                    [c],
                    |r| r.get::<_, Option<i64>>(0),
                ) {
                    Ok(p) => p,
                    // Row gone (shouldn't happen mid-walk) → stop walking.
                    Err(rusqlite::Error::QueryReturnedNoRows) => None,
                    Err(e) => return Err(e),
                };
            }
        }
        conn.execute(
            "UPDATE folders SET parent_id = ?1 WHERE id = ?2",
            rusqlite::params![new_parent, id],
        )?;
        Ok(())
    }

    /// Set or clear a channel's folder assignment. `folder_id = None`
    /// deletes the row so the channel reverts to "Unfiled".
    pub fn set_channel_folder(
        &self,
        platform: &str,
        handle: &str,
        folder_id: Option<i64>,
    ) -> Result<()> {
        let conn = self.conn();
        match folder_id {
            Some(fid) => {
                conn.execute(
                    "INSERT OR REPLACE INTO channel_assignments (platform, handle, folder_id)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![platform, handle, fid],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM channel_assignments WHERE platform = ?1 AND handle = ?2",
                    [platform, handle],
                )?;
            }
        }
        Ok(())
    }

    /// Bulk fetch of every channel's folder assignment as a
    /// `((platform, handle) → folder_id)` map. Used by the library scanner
    /// to populate `Channel.folder_id` after a rescan.
    pub fn get_all_channel_assignments(&self) -> Result<HashMap<(String, String), i64>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT platform, handle, folder_id FROM channel_assignments",
        )?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, i64>(2)?,
                ))
            })?
            .filter_map(std::result::Result::ok)
            .map(|(k, v)| (k, v))
            .collect();
        Ok(map)
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

    /// Fetch a single note body. `target_kind` is `"channel"` or
    /// `"video"`; `target_id` is `"platform/handle"` or the video ID.
    /// Returns `None` when no note exists.
    pub fn get_note(&self, target_kind: &str, target_id: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT body FROM notes WHERE target_kind = ?1 AND target_id = ?2",
        )?;
        let mut rows = stmt.query([target_kind, target_id])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Upsert (or delete) a note. An empty / whitespace-only body deletes
    /// the row so we never store blank notes — that keeps `get_all_notes`
    /// and the search index free of noise.
    pub fn set_note(&self, target_kind: &str, target_id: &str, body: &str) -> Result<()> {
        let conn = self.conn();
        if body.trim().is_empty() {
            conn.execute(
                "DELETE FROM notes WHERE target_kind = ?1 AND target_id = ?2",
                [target_kind, target_id],
            )?;
        } else {
            conn.execute(
                "INSERT INTO notes (target_kind, target_id, body, updated_at) \
                 VALUES (?1, ?2, ?3, CURRENT_TIMESTAMP) \
                 ON CONFLICT(target_kind, target_id) \
                 DO UPDATE SET body = excluded.body, updated_at = CURRENT_TIMESTAMP",
                rusqlite::params![target_kind, target_id, body],
            )?;
        }
        Ok(())
    }

    /// Bulk fetch of every note as `((target_kind, target_id) → body)`.
    /// Hydrated into memory at startup so the UI can render note
    /// indicators + search bodies without per-item SQL.
    pub fn get_all_notes(&self) -> Result<HashMap<(String, String), String>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT target_kind, target_id, body FROM notes")?;
        let map = stmt
            .query_map([], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    row.get::<_, String>(2)?,
                ))
            })?
            .filter_map(std::result::Result::ok)
            .collect();
        Ok(map)
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

    /// Look up the cached parse of an info.json sidecar. Returns
    /// `(duration_secs, has_chapters, upload_date)` if the cache row's
    /// `mtime_unix` matches the supplied value (cache hit), or `None`
    /// when the row is missing or stale.
    ///
    /// Used by the library scan hot path. The lookup itself is two SQL
    /// columns + an integer compare, dominated by the SQLite call
    /// overhead (~microseconds) rather than JSON parsing (~hundreds of
    /// microseconds), which is the savings we're harvesting.
    pub fn info_cache_get(
        &self,
        path: &str,
        mtime_unix: u64,
    ) -> Option<(Option<f64>, bool, Option<String>)> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT mtime_unix, duration_secs, has_chapters, upload_date \
             FROM info_cache WHERE path = ?1",
        ).ok()?;
        let mut rows = stmt.query([path]).ok()?;
        let row = rows.next().ok().flatten()?;
        let stored_mtime: i64 = row.get(0).ok()?;
        if stored_mtime != mtime_unix as i64 {
            return None;
        }
        let dur: Option<f64> = row.get(1).ok()?;
        let chap: i64 = row.get(2).ok()?;
        let date: Option<String> = row.get(3).ok()?;
        Some((dur, chap != 0, date))
    }

    /// Upsert a parsed info.json result into the cache. Called on miss
    /// by the library scanner. Errors are swallowed — a cache miss next
    /// time costs the same as no cache at all.
    pub fn info_cache_put(
        &self,
        path: &str,
        mtime_unix: u64,
        duration_secs: Option<f64>,
        has_chapters: bool,
        upload_date: Option<&str>,
    ) {
        let conn = self.conn();
        let _ = conn.execute(
            "INSERT OR REPLACE INTO info_cache \
                (path, mtime_unix, duration_secs, has_chapters, upload_date) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                path,
                mtime_unix as i64,
                duration_secs,
                has_chapters as i64,
                upload_date,
            ],
        );
    }

    /// Idempotently merge another yt-offline database into this one.
    ///
    /// Designed for "Import library backup…" — the user uploads a snapshot
    /// produced by `GET /api/backup/db`, and we merge its rows in without
    /// disturbing the channel files on disk. Safe to re-run with the same
    /// backup (or a chain of overlapping backups): conflicting rows resolve
    /// deterministically by recency, flag-OR, or first-write-wins depending
    /// on the table's semantics.
    ///
    /// # Schema validation
    ///
    /// Before merging, we verify each expected table exists in the backup.
    /// A backup from an older schema is rejected outright — partial merges
    /// could leave the DB in a state where some features see stale data.
    /// The caller can offer a "do it anyway" path later if needed.
    ///
    /// # Per-table merge rules
    ///
    /// - `watched`: keep the later `watched_at`. INSERT-OR-IGNORE then
    ///   UPDATE-when-newer covers both directions.
    /// - `positions`: keep the later `updated_at` (same pattern).
    /// - `settings`: skip — `password_hash` and other settings are
    ///   *machine-local* per AGPL-deployment context. Importing them
    ///   would re-authorize an old password / overwrite the current
    ///   source_url with a stale value. The user can re-set them.
    /// - `channel_options`: keep the later `updated_at`.
    /// - `video_flags`: bitwise OR each flag column. If you favourited a
    ///   video on either side, it stays favourited.
    /// - `folders`: insert when the same name doesn't already exist.
    ///   Folder *contents* may differ between backups; we keep the
    ///   current side as authoritative and ignore the backup's
    ///   `channel_assignments` for any folder we already have.
    /// - `channel_assignments`: insert when the (platform, handle) pair
    ///   isn't already assigned. Doesn't change existing assignments.
    pub fn restore_from_backup(&self, backup_path: &Path) -> Result<RestoreSummary> {
        // Open the backup file via ATTACH so we can write `INSERT … SELECT
        // … FROM bk.<table>` in a single transaction against the live DB.
        // ATTACH paths are escaped by binding rather than interpolating to
        // avoid an injection if a caller ever passes a user-influenced
        // string (current callers pass a tmpfile path, but defense in
        // depth is cheap).
        let path_str = backup_path.to_string_lossy().to_string();
        let conn = self.conn();
        conn.execute("ATTACH DATABASE ?1 AS bk", [&path_str])?;
        // No matter what happens below, DETACH so the next caller's pool
        // checkout doesn't see a lingering attachment.
        let result = (|| -> Result<RestoreSummary> {
            // ── Schema validation ────────────────────────────────────
            let required = [
                "watched",
                "positions",
                "channel_options",
                "video_flags",
                "folders",
                "channel_assignments",
            ];
            for table in &required {
                let count: i64 = conn.query_row(
                    "SELECT count(*) FROM bk.sqlite_master \
                     WHERE type = 'table' AND name = ?1",
                    [table],
                    |r| r.get(0),
                )?;
                if count == 0 {
                    return Err(rusqlite::Error::SqliteFailure(
                        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_MISMATCH),
                        Some(format!(
                            "backup is missing required table `{table}` — \
                             not a yt-offline snapshot, or from an incompatible version"
                        )),
                    ));
                }
            }

            // Wrap the whole merge in a transaction. If any step fails the
            // attached DB stays read-only on our side and the live DB
            // rolls back to pre-import state.
            conn.execute("BEGIN", [])?;
            let summary = (|| -> Result<RestoreSummary> {
                // watched: keep the later timestamp. Two-step approach
                // (insert missing, then update older) works in plain SQL
                // without an UPSERT WHERE clause.
                let watched_before: i64 = conn.query_row(
                    "SELECT count(*) FROM watched", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO watched (video_id, watched_at) \
                     SELECT video_id, watched_at FROM bk.watched", [])?;
                conn.execute(
                    "UPDATE watched SET watched_at = (\
                         SELECT bk.watched.watched_at FROM bk.watched \
                         WHERE bk.watched.video_id = main.watched.video_id) \
                     WHERE EXISTS (\
                         SELECT 1 FROM bk.watched \
                         WHERE bk.watched.video_id = main.watched.video_id \
                         AND bk.watched.watched_at > main.watched.watched_at)", [])?;
                let watched_after: i64 = conn.query_row(
                    "SELECT count(*) FROM watched", [], |r| r.get(0))?;
                let watched_added = (watched_after - watched_before).max(0);

                // positions: same pattern as watched.
                let positions_before: i64 = conn.query_row(
                    "SELECT count(*) FROM positions", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO positions (video_id, position_secs, updated_at) \
                     SELECT video_id, position_secs, updated_at FROM bk.positions", [])?;
                // Use main.positions to disambiguate the target — inside
                // the WHERE/SET subqueries SQLite would otherwise resolve
                // the bare `positions` to the innermost FROM (bk.positions
                // for the SELECT subquery), giving us a degenerate
                // `bk.positions.x = bk.positions.x` join.
                conn.execute(
                    "UPDATE positions SET \
                        position_secs = (SELECT bk.positions.position_secs FROM bk.positions \
                            WHERE bk.positions.video_id = main.positions.video_id), \
                        updated_at = (SELECT bk.positions.updated_at FROM bk.positions \
                            WHERE bk.positions.video_id = main.positions.video_id) \
                     WHERE EXISTS (\
                         SELECT 1 FROM bk.positions \
                         WHERE bk.positions.video_id = main.positions.video_id \
                         AND bk.positions.updated_at > main.positions.updated_at)", [])?;
                let positions_after: i64 = conn.query_row(
                    "SELECT count(*) FROM positions", [], |r| r.get(0))?;
                let positions_added = (positions_after - positions_before).max(0);

                // channel_options: keep the later updated_at.
                let options_before: i64 = conn.query_row(
                    "SELECT count(*) FROM channel_options", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO channel_options \
                        (platform, handle, options_json, updated_at) \
                     SELECT platform, handle, options_json, updated_at \
                     FROM bk.channel_options", [])?;
                conn.execute(
                    "UPDATE channel_options SET \
                        options_json = (SELECT bk.channel_options.options_json \
                            FROM bk.channel_options \
                            WHERE bk.channel_options.platform = main.channel_options.platform \
                            AND bk.channel_options.handle   = main.channel_options.handle), \
                        updated_at = (SELECT bk.channel_options.updated_at \
                            FROM bk.channel_options \
                            WHERE bk.channel_options.platform = main.channel_options.platform \
                            AND bk.channel_options.handle   = main.channel_options.handle) \
                     WHERE EXISTS (\
                         SELECT 1 FROM bk.channel_options \
                         WHERE bk.channel_options.platform = main.channel_options.platform \
                         AND bk.channel_options.handle   = main.channel_options.handle \
                         AND bk.channel_options.updated_at > main.channel_options.updated_at)", [])?;
                let options_after: i64 = conn.query_row(
                    "SELECT count(*) FROM channel_options", [], |r| r.get(0))?;
                let options_added = (options_after - options_before).max(0);

                // video_flags: bitwise OR each flag column. Insert missing
                // first, then OR for collisions. (`MAX` works since each
                // flag is 0/1.)
                let flags_before: i64 = conn.query_row(
                    "SELECT count(*) FROM video_flags", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO video_flags \
                        (video_id, bookmark, favourite, waiting, archive, updated_at) \
                     SELECT video_id, bookmark, favourite, waiting, archive, updated_at \
                     FROM bk.video_flags", [])?;
                conn.execute(
                    "UPDATE video_flags SET \
                        bookmark  = MAX(bookmark,  COALESCE((SELECT bookmark  FROM bk.video_flags \
                            WHERE bk.video_flags.video_id = main.video_flags.video_id), 0)), \
                        favourite = MAX(favourite, COALESCE((SELECT favourite FROM bk.video_flags \
                            WHERE bk.video_flags.video_id = main.video_flags.video_id), 0)), \
                        waiting   = MAX(waiting,   COALESCE((SELECT waiting   FROM bk.video_flags \
                            WHERE bk.video_flags.video_id = main.video_flags.video_id), 0)), \
                        archive   = MAX(archive,   COALESCE((SELECT archive   FROM bk.video_flags \
                            WHERE bk.video_flags.video_id = main.video_flags.video_id), 0))", [])?;
                let flags_after: i64 = conn.query_row(
                    "SELECT count(*) FROM video_flags", [], |r| r.get(0))?;
                let flags_added = (flags_after - flags_before).max(0);

                // folders: only insert names we don't already have.
                let folders_before: i64 = conn.query_row(
                    "SELECT count(*) FROM folders", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO folders (name, position, created_at) \
                     SELECT name, position, created_at FROM bk.folders", [])?;
                let folders_after: i64 = conn.query_row(
                    "SELECT count(*) FROM folders", [], |r| r.get(0))?;
                let folders_added = (folders_after - folders_before).max(0);

                // channel_assignments: insert when (platform, handle) is
                // unassigned. We re-resolve folder_id by name to handle the
                // case where the backup and live DB have the same folder
                // name but different IDs.
                let assignments_before: i64 = conn.query_row(
                    "SELECT count(*) FROM channel_assignments", [], |r| r.get(0))?;
                conn.execute(
                    "INSERT OR IGNORE INTO channel_assignments (platform, handle, folder_id) \
                     SELECT b.platform, b.handle, f.id \
                     FROM bk.channel_assignments b \
                     JOIN bk.folders bf ON bf.id = b.folder_id \
                     JOIN folders f ON f.name = bf.name", [])?;
                let assignments_after: i64 = conn.query_row(
                    "SELECT count(*) FROM channel_assignments", [], |r| r.get(0))?;
                let assignments_added = (assignments_after - assignments_before).max(0);

                // notes: keep the later updated_at, same pattern as watched.
                // The notes table is newer than the others, so a backup from
                // before it existed won't have it — guard with a table check
                // and skip the merge silently when it's absent.
                let mut notes_added: i64 = 0;
                let has_notes: i64 = conn.query_row(
                    "SELECT count(*) FROM bk.sqlite_master \
                     WHERE type = 'table' AND name = 'notes'",
                    [], |r| r.get(0))?;
                if has_notes > 0 {
                    let notes_before: i64 = conn.query_row(
                        "SELECT count(*) FROM notes", [], |r| r.get(0))?;
                    conn.execute(
                        "INSERT OR IGNORE INTO notes (target_kind, target_id, body, updated_at) \
                         SELECT target_kind, target_id, body, updated_at FROM bk.notes", [])?;
                    conn.execute(
                        "UPDATE notes SET \
                            body = (SELECT bk.notes.body FROM bk.notes \
                                WHERE bk.notes.target_kind = main.notes.target_kind \
                                AND bk.notes.target_id = main.notes.target_id), \
                            updated_at = (SELECT bk.notes.updated_at FROM bk.notes \
                                WHERE bk.notes.target_kind = main.notes.target_kind \
                                AND bk.notes.target_id = main.notes.target_id) \
                         WHERE EXISTS (\
                             SELECT 1 FROM bk.notes \
                             WHERE bk.notes.target_kind = main.notes.target_kind \
                             AND bk.notes.target_id = main.notes.target_id \
                             AND bk.notes.updated_at > main.notes.updated_at)", [])?;
                    let notes_after: i64 = conn.query_row(
                        "SELECT count(*) FROM notes", [], |r| r.get(0))?;
                    notes_added = (notes_after - notes_before).max(0);
                }

                Ok(RestoreSummary {
                    watched_added: watched_added as u64,
                    positions_added: positions_added as u64,
                    options_added: options_added as u64,
                    flags_added: flags_added as u64,
                    folders_added: folders_added as u64,
                    assignments_added: assignments_added as u64,
                    notes_added: notes_added as u64,
                })
            })();
            match summary {
                Ok(s) => {
                    conn.execute("COMMIT", [])?;
                    Ok(s)
                }
                Err(e) => {
                    let _ = conn.execute("ROLLBACK", []);
                    Err(e)
                }
            }
        })();
        // ATTACH state lives on the connection. Detach even on error so
        // the pooled connection is clean when it goes back.
        let _ = conn.execute("DETACH DATABASE bk", []);
        result
    }

    /// Refresh the full-text search index against the current library.
    ///
    /// `entries` is the full set of videos currently on disk. A video whose
    /// `mtime_unix` already matches the index is skipped; new/changed videos
    /// get their description sidecar re-read and reindexed; videos that
    /// vanished from disk are dropped. Returns how many rows were
    /// (re)indexed (0 means the index was already current). Runs in one
    /// transaction so a crash mid-sync can't leave the index half-written.
    pub fn sync_search_index(&self, entries: &[SearchEntry]) -> Result<usize> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;

        // What's already indexed: video_id -> mtime.
        let mut existing: HashMap<String, i64> = HashMap::new();
        {
            let mut stmt = tx.prepare("SELECT video_id, mtime_unix FROM search_meta")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
            for row in rows { let (id, m) = row?; existing.insert(id, m); }
        }

        let mut seen: HashSet<&str> = HashSet::with_capacity(entries.len());
        let mut changed = 0usize;
        for e in entries {
            seen.insert(e.video_id.as_str());
            if existing.get(&e.video_id) == Some(&e.mtime_unix) {
                continue; // unchanged — leave the indexed row in place
            }
            // Only new/changed videos pay the description-read cost.
            let description = e.description_path.as_ref()
                .and_then(|p| std::fs::read_to_string(p).ok())
                .unwrap_or_default();
            tx.execute("DELETE FROM video_search WHERE video_id = ?1", [&e.video_id])?;
            tx.execute(
                "INSERT INTO video_search (video_id, platform, channel, title, description)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![e.video_id, e.platform, e.channel, e.title, description],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO search_meta (video_id, mtime_unix) VALUES (?1, ?2)",
                rusqlite::params![e.video_id, e.mtime_unix],
            )?;
            changed += 1;
        }

        // Evict videos that no longer exist on disk.
        let stale: Vec<String> = existing.keys()
            .filter(|id| !seen.contains(id.as_str()))
            .cloned()
            .collect();
        for id in &stale {
            tx.execute("DELETE FROM video_search WHERE video_id = ?1", [id])?;
            tx.execute("DELETE FROM search_meta WHERE video_id = ?1", [id])?;
        }

        tx.commit()?;
        Ok(changed)
    }

    /// Full-text search the library, newest-relevance first. Returns up to
    /// `limit` hits, each with a highlighted description snippet. An empty or
    /// punctuation-only query yields no results rather than an error.
    pub fn search_videos(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>> {
        let match_expr = fts_match_expr(query);
        if match_expr.is_empty() { return Ok(Vec::new()); }
        let conn = self.conn();
        let mut stmt = conn.prepare(
            // STX/ETX (\u{2}/\u{3}) delimit matched terms — control chars
            // that won't collide with literal '[' / ']' in a description.
            // The UI turns them into highlight markup.
            "SELECT video_id, platform, channel, title,
                    snippet(video_search, 4, char(2), char(3), '…', 12)
             FROM video_search
             WHERE video_search MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![match_expr, limit as i64], |r| {
            Ok(SearchHit {
                video_id: r.get(0)?,
                platform: r.get(1)?,
                channel: r.get(2)?,
                title: r.get(3)?,
                snippet: r.get(4)?,
            })
        })?;
        rows.collect()
    }
}

/// Per-table row counts that landed in the live DB during a restore.
/// Useful for the UI to show "imported N watched + M positions" so the
/// user sees evidence the merge actually did something.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RestoreSummary {
    pub watched_added: u64,
    pub positions_added: u64,
    pub options_added: u64,
    pub flags_added: u64,
    pub folders_added: u64,
    pub assignments_added: u64,
    pub notes_added: u64,
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn entry(id: &str, mtime: i64, channel: &str, title: &str) -> SearchEntry {
        SearchEntry {
            video_id: id.into(), mtime_unix: mtime,
            platform: "channels".into(), channel: channel.into(),
            title: title.into(), description_path: None,
        }
    }

    #[test]
    fn indexes_searches_and_evicts() {
        let db = Database::open_in_memory().unwrap();
        let entries = vec![
            entry("a", 1, "Rustaceans", "Async Rust deep dive"),
            entry("b", 1, "Cooking", "Sourdough bread from scratch"),
        ];
        assert_eq!(db.sync_search_index(&entries).unwrap(), 2);

        // Title match.
        let hits = db.search_videos("rust", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].video_id, "a");

        // Prefix (type-ahead) match.
        assert_eq!(db.search_videos("sourd", 10).unwrap().len(), 1);

        // The channel field is searched too.
        let hits = db.search_videos("cooking", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].video_id, "b");

        // Multi-token is AND-ed.
        assert_eq!(db.search_videos("async dive", 10).unwrap().len(), 1);
        assert_eq!(db.search_videos("async sourdough", 10).unwrap().len(), 0);

        // Re-syncing unchanged entries is a no-op.
        assert_eq!(db.sync_search_index(&entries).unwrap(), 0);

        // A changed mtime forces a reindex of just that row.
        let changed = vec![
            entry("a", 2, "Rustaceans", "Async Rust deep dive — updated"),
            entry("b", 1, "Cooking", "Sourdough bread from scratch"),
        ];
        assert_eq!(db.sync_search_index(&changed).unwrap(), 1);
        assert_eq!(db.search_videos("updated", 10).unwrap().len(), 1);

        // Dropping "b" from disk evicts it from the index.
        assert_eq!(db.sync_search_index(&changed[..1]).unwrap(), 0);
        assert_eq!(db.search_videos("sourdough", 10).unwrap().len(), 0);

        // Garbage / empty queries return nothing, not an error.
        assert!(db.search_videos("", 10).unwrap().is_empty());
        assert!(db.search_videos("   \"  ", 10).unwrap().is_empty());
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    /// Per-test scratch dir that auto-removes itself. Avoids pulling in
    /// the `tempfile` crate just for this test module.
    struct ScratchDir(std::path::PathBuf);
    impl ScratchDir {
        fn new(name: &str) -> Self {
            let mut p = std::env::temp_dir();
            // Disambiguate parallel-test runs of the same name with the
            // pid + a counter; collisions would otherwise leave one test
            // operating on another's DB.
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let id = N.fetch_add(1, Ordering::Relaxed);
            p.push(format!("yt-offline-test-{}-{}-{}", std::process::id(), id, name));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            ScratchDir(p)
        }
        fn join(&self, name: &str) -> std::path::PathBuf { self.0.join(name) }
    }
    impl Drop for ScratchDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn restores_watched_and_positions() {
        let dir = ScratchDir::new("watched-positions");
        let live = Database::open(&dir.join("live.db")).unwrap();
        let backup = Database::open(&dir.join("backup.db")).unwrap();

        backup.set_watched("v-only-in-backup", true).unwrap();
        // CURRENT_TIMESTAMP has 1-second resolution. Write the live row
        // first, sleep just over a second, then write the backup row so
        // the merge's `updated_at >` comparison actually picks the
        // backup's value. Real-world backups are taken minutes/days
        // apart, so this resolution is fine in production.
        live.set_position("v-shared", 10.0).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        backup.set_position("v-shared", 42.5).unwrap();

        let summary = live.restore_from_backup(&dir.join("backup.db")).unwrap();
        assert_eq!(summary.watched_added, 1);
        // v-shared existed on the live side already, so positions_added
        // counts only the *new* row (which there isn't one of).
        assert_eq!(summary.positions_added, 0);

        // The watched row from backup made it through.
        let w = live.get_watched().unwrap();
        assert!(w.contains("v-only-in-backup"));

        // v-shared got the *later* position (backup's, since the live one
        // was inserted earlier in this test).
        let p = live.get_positions().unwrap();
        assert!((p.get("v-shared").copied().unwrap_or(0.0) - 42.5).abs() < 0.001);
    }

    #[test]
    fn ors_video_flags() {
        let dir = ScratchDir::new("flags-or");
        let live = Database::open(&dir.join("live.db")).unwrap();
        let backup = Database::open(&dir.join("backup.db")).unwrap();

        // Live side: v1 favourite. Backup side: v1 bookmark. After merge
        // v1 should be both.
        live.set_video_flag("v1", "favourite", true).unwrap();
        backup.set_video_flag("v1", "bookmark", true).unwrap();
        backup.set_video_flag("v2", "waiting", true).unwrap();

        live.restore_from_backup(&dir.join("backup.db")).unwrap();
        let flags = live.get_video_flags().unwrap();
        assert!(flags.favourite.contains("v1"));
        assert!(flags.bookmark.contains("v1"));
        assert!(flags.waiting.contains("v2"));
    }

    #[test]
    fn idempotent_when_run_twice() {
        let dir = ScratchDir::new("idempotent");
        let live = Database::open(&dir.join("live.db")).unwrap();
        let backup = Database::open(&dir.join("backup.db")).unwrap();
        backup.set_watched("v1", true).unwrap();
        backup.set_position("v1", 7.5).unwrap();

        let s1 = live.restore_from_backup(&dir.join("backup.db")).unwrap();
        let s2 = live.restore_from_backup(&dir.join("backup.db")).unwrap();

        // First pass adds 1 of each, second adds none (same backup).
        assert_eq!(s1.watched_added, 1);
        assert_eq!(s1.positions_added, 1);
        assert_eq!(s2.watched_added, 0);
        assert_eq!(s2.positions_added, 0);
    }

    #[test]
    fn rejects_unrelated_sqlite_file() {
        let dir = ScratchDir::new("schema-mismatch");
        let live = Database::open(&dir.join("live.db")).unwrap();

        // Create a SQLite file with a completely different schema.
        let bad = dir.join("not-yt-offline.db");
        let conn = Connection::open(&bad).unwrap();
        conn.execute("CREATE TABLE foo (x INT)", []).unwrap();
        drop(conn);

        let err = live.restore_from_backup(&bad).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("missing required table"), "{msg}");
    }

    #[test]
    fn notes_upsert_get_and_empty_delete() {
        let dir = ScratchDir::new("notes");
        let db = Database::open(&dir.join("notes.db")).unwrap();

        // No note yet.
        assert_eq!(db.get_note("video", "v1").unwrap(), None);

        // Set + read back.
        db.set_note("video", "v1", "remember this clip").unwrap();
        assert_eq!(db.get_note("video", "v1").unwrap().as_deref(), Some("remember this clip"));

        // Overwrite.
        db.set_note("video", "v1", "updated text").unwrap();
        assert_eq!(db.get_note("video", "v1").unwrap().as_deref(), Some("updated text"));

        // Channel note keyed separately.
        db.set_note("channel", "youtube/Andrewism", "great anarchist channel").unwrap();
        assert_eq!(
            db.get_note("channel", "youtube/Andrewism").unwrap().as_deref(),
            Some("great anarchist channel"),
        );

        // get_all_notes returns both.
        let all = db.get_all_notes().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.get(&("video".into(), "v1".into())).map(String::as_str), Some("updated text"));

        // Empty body deletes the row.
        db.set_note("video", "v1", "   ").unwrap();
        assert_eq!(db.get_note("video", "v1").unwrap(), None);
        assert_eq!(db.get_all_notes().unwrap().len(), 1);
    }

    #[test]
    fn folder_nesting_and_cycle_guard() {
        let dir = ScratchDir::new("folder-nest");
        let db = Database::open(&dir.join("nest.db")).unwrap();

        let a = db.create_folder("A").unwrap();
        let b = db.create_folder("B").unwrap();
        let c = db.create_folder("C").unwrap();

        // Nest B under A, C under B.
        db.set_folder_parent(b, Some(a)).unwrap();
        db.set_folder_parent(c, Some(b)).unwrap();

        let folders = db.list_folders().unwrap();
        let by_id = |id: i64| folders.iter().find(|f| f.id == id).unwrap();
        assert_eq!(by_id(a).parent_id, None);
        assert_eq!(by_id(b).parent_id, Some(a));
        assert_eq!(by_id(c).parent_id, Some(b));

        // A folder can't be its own parent.
        assert!(db.set_folder_parent(a, Some(a)).is_err());

        // Can't nest A under C (C is A's descendant → cycle).
        assert!(db.set_folder_parent(a, Some(c)).is_err());

        // Moving back to top level works.
        db.set_folder_parent(c, None).unwrap();
        assert_eq!(db.list_folders().unwrap().iter().find(|f| f.id == c).unwrap().parent_id, None);
    }

    #[test]
    fn notes_merge_keeps_later_on_restore() {
        let dir = ScratchDir::new("notes-merge");
        let live = Database::open(&dir.join("live.db")).unwrap();
        let backup = Database::open(&dir.join("backup.db")).unwrap();

        // A note only in the backup gets pulled in; a note newer on the
        // backup side wins the conflict.
        backup.set_note("video", "only-backup", "from backup").unwrap();
        live.set_note("video", "shared", "old live text").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1100));
        backup.set_note("video", "shared", "newer backup text").unwrap();

        let summary = live.restore_from_backup(&dir.join("backup.db")).unwrap();
        assert_eq!(summary.notes_added, 1); // only-backup is the new row
        assert_eq!(live.get_note("video", "only-backup").unwrap().as_deref(), Some("from backup"));
        assert_eq!(live.get_note("video", "shared").unwrap().as_deref(), Some("newer backup text"));
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
