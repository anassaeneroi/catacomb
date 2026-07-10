# Persistent Library Snapshot — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the desktop GUI render the last-known library instantly at launch from a persisted snapshot, then background-rescan and swap in the fresh result.

**Architecture:** Serialize the scanned `Vec<library::Channel>` to a JSON blob in a new one-row-per-root SQLite table. On desktop launch, load and seed `self.library` before spawning the (unchanged) background scan thread. The scan stays authoritative and swaps in truth via the existing `library_load_rx` drain, so the snapshot is a display optimization that can only be briefly stale, never wrong.

**Tech Stack:** Rust, rusqlite (r2d2 pool), serde / serde_json (already dependencies), eframe/egui.

## Global Constraints

- Desktop GUI only. Do **not** touch `web.rs` startup or the web in-memory snapshot machinery.
- Snapshot writes and loads are **error-swallowing / non-fatal** — a failure just means no fresh snapshot next launch (same tolerance as `info_cache_put_many`).
- The background scan runs unchanged and remains authoritative.
- New DB schema goes in `init_schema()` as idempotent `CREATE TABLE IF NOT EXISTS` (no migration framework in this repo).
- Never commit `cookies.txt`, `config.toml`, or `catacomb.db` (gitignored).
- Commits are SSH-signed: `export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock` before `git commit`. End commit messages with `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`.
- Spec: `docs/superpowers/specs/2026-07-09-library-snapshot-instant-startup-design.md`.

---

### Task 1: DB snapshot persistence layer

Make the library structs serializable and add save/load to the DB, fully tested headlessly. Derives are scaffolding for the persistence API, so they live in this task.

**Files:**
- Modify: `src/library.rs` — add `Serialize, Deserialize` derives to `Subtitle`, `Video`, `Playlist`, `ChannelMeta`, `Channel`; add `#[serde(default)]` to `Video` and `Channel` fields for forward-compat.
- Modify: `src/database.rs` — add `library_snapshot` table to `init_schema()`; add `save_library_snapshot` + `load_library_snapshot`; add unit tests.

**Interfaces:**
- Produces:
  - `Database::save_library_snapshot(&self, root: &std::path::Path, library: &[crate::library::Channel])` — serializes to JSON, `INSERT OR REPLACE` into `library_snapshot`. Error-swallowing (returns `()`).
  - `Database::load_library_snapshot(&self, root: &std::path::Path) -> Option<Vec<crate::library::Channel>>` — `None` on missing row **or** deserialize failure.

- [ ] **Step 1: Add serde derives to the library structs**

In `src/library.rs`, confirm the serde import at the top of the file (add if absent):

```rust
use serde::{Deserialize, Serialize};
```

Change each derive line as follows:

```rust
// Subtitle
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Subtitle {
```

```rust
// Playlist
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Playlist {
```

```rust
// ChannelMeta
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelMeta {
```

For `Video`, add the derives **and** `#[serde(default)]` on every field so old snapshots missing a future field still deserialize:

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Video {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub stem: String,
    #[serde(default)]
    pub video_path: Option<PathBuf>,
    #[serde(default)]
    pub thumb_path: Option<PathBuf>,
    #[serde(default)]
    pub description_path: Option<PathBuf>,
    #[serde(default)]
    pub info_path: Option<PathBuf>,
    #[serde(default)]
    pub subtitles: Vec<Subtitle>,
    #[serde(default)]
    pub has_live_chat: bool,
    #[serde(default)]
    pub duration_secs: Option<f64>,
    #[serde(default)]
    pub has_chapters: bool,
    #[serde(default)]
    pub file_size: Option<u64>,
    #[serde(default)]
    pub mtime_unix: Option<u64>,
    #[serde(default)]
    pub upload_date: Option<String>,
}
```

For `Channel`, add the derives and `#[serde(default)]` on every field (keep the existing doc comments):

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Channel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path: PathBuf,
    #[serde(default)]
    pub platform: Platform,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub videos: Vec<Video>,
    #[serde(default)]
    pub playlists: Vec<Playlist>,
    #[serde(default)]
    pub meta: Option<ChannelMeta>,
    #[serde(default)]
    pub total_videos_cached: usize,
    #[serde(default)]
    pub total_size_cached: u64,
    #[serde(default)]
    pub download_options: DownloadOptions,
    #[serde(default)]
    pub folder_id: Option<i64>,
}
```

`Platform` uses `#[serde(default)]` on the `Channel.platform` field, which requires `Platform: Default`. If `Platform` does not already derive/impl `Default`, add a default in `src/platform.rs` by annotating the YouTube variant:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Platform {
    #[default]
    YouTube,
    // …remaining variants unchanged
}
```

(If `Platform` already impls `Default`, leave it as-is.) Confirm `folder_id`'s real type in `src/library.rs` (the tail of the `Channel` struct) and match it exactly — the snippet assumes `Option<i64>`.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --release 2>&1 | grep -E "^error" | head`
Expected: no output (no `error` lines). Fix any missing-`Default` / trait errors before proceeding.

- [ ] **Step 3: Write the failing persistence tests**

In `src/database.rs`, inside the existing `#[cfg(test)] mod tests { … }` block (create one at end of file if none exists, with `use super::*;`), add:

```rust
#[test]
fn library_snapshot_round_trips() {
    use crate::library::{Channel, Playlist, Video};
    use crate::platform::Platform;
    let db = Database::open_in_memory().unwrap();
    let root = std::path::Path::new("/tmp/lib-root");

    let video = Video {
        id: "abc123".into(),
        title: "Test Video".into(),
        stem: "Test Video [abc123]".into(),
        video_path: Some("/tmp/lib-root/channels/Chan/Test [abc123].mp4".into()),
        thumb_path: None,
        description_path: None,
        info_path: None,
        subtitles: vec![],
        has_live_chat: false,
        duration_secs: Some(42.0),
        has_chapters: true,
        file_size: Some(1024),
        mtime_unix: Some(1_700_000_000),
        upload_date: Some("20250101".into()),
    };
    let mut chan = Channel {
        name: "Chan".into(),
        path: "/tmp/lib-root/channels/Chan".into(),
        platform: Platform::YouTube,
        source_url: Some("https://youtube.com/@chan".into()),
        videos: vec![video.clone()],
        playlists: vec![Playlist {
            name: "PL".into(),
            path: "/tmp/lib-root/channels/Chan/PL".into(),
            videos: vec![video.clone()],
        }],
        meta: None,
        total_videos_cached: 2,
        total_size_cached: 2048,
        download_options: Default::default(),
        folder_id: Some(7),
    };
    chan.download_options = Default::default();

    let library = vec![chan];
    db.save_library_snapshot(root, &library);
    let loaded = db.load_library_snapshot(root).expect("snapshot present");
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "Chan");
    assert_eq!(loaded[0].videos.len(), 1);
    assert_eq!(loaded[0].videos[0].id, "abc123");
    assert_eq!(loaded[0].playlists[0].videos[0].duration_secs, Some(42.0));
    assert_eq!(loaded[0].folder_id, Some(7));
}

#[test]
fn library_snapshot_missing_root_is_none() {
    let db = Database::open_in_memory().unwrap();
    assert!(db.load_library_snapshot(std::path::Path::new("/nope")).is_none());
}

#[test]
fn library_snapshot_garbage_json_is_none() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.conn();
    conn.execute(
        "INSERT INTO library_snapshot (root, json, saved_at) VALUES (?1, ?2, ?3)",
        rusqlite::params!["/tmp/bad", "{not valid json", 0i64],
    )
    .unwrap();
    assert!(db.load_library_snapshot(std::path::Path::new("/tmp/bad")).is_none());
}
```

(Adjust the `Video`/`Channel`/`Playlist` field list if the struct in `library.rs` differs from Task 1 Step 1 — the constructor must name every field.)

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test --release library_snapshot 2>&1 | grep -E "error\[|cannot find|test result"`
Expected: FAIL — compile errors (`no method named save_library_snapshot`/`load_library_snapshot`, and `library_snapshot` table missing).

- [ ] **Step 5: Add the table to `init_schema()`**

In `src/database.rs`, inside `init_schema()`'s `execute_batch(…)` schema string, add:

```sql
CREATE TABLE IF NOT EXISTS library_snapshot (
    root      TEXT PRIMARY KEY,
    json      TEXT NOT NULL,
    saved_at  INTEGER NOT NULL
);
```

- [ ] **Step 6: Implement save/load**

Add these methods to the `impl Database` block in `src/database.rs` (near `info_cache_put_many`):

```rust
/// Persist the scanned library as a JSON blob keyed by its root path, so the
/// desktop GUI can render it instantly on the next launch before the (slow,
/// disk-bound) rescan finishes. Error-swallowing: a failure just means no
/// fresh snapshot next launch — non-fatal, like `info_cache_put_many`.
pub fn save_library_snapshot(&self, root: &std::path::Path, library: &[crate::library::Channel]) {
    let Ok(json) = serde_json::to_string(library) else { return };
    let root = root.display().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let conn = self.conn();
    let _ = conn.execute(
        "INSERT OR REPLACE INTO library_snapshot (root, json, saved_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![root, json, now],
    );
}

/// Load the last-persisted library for `root`. Returns `None` if there is no
/// snapshot row or the stored JSON no longer deserializes (e.g. after a struct
/// change) — callers fall back to the scanning state.
pub fn load_library_snapshot(&self, root: &std::path::Path) -> Option<Vec<crate::library::Channel>> {
    let root = root.display().to_string();
    let conn = self.conn();
    let json: String = conn
        .query_row(
            "SELECT json FROM library_snapshot WHERE root = ?1",
            rusqlite::params![root],
            |r| r.get(0),
        )
        .ok()?;
    serde_json::from_str(&json).ok()
}
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test --release library_snapshot 2>&1 | grep "test result"`
Expected: `test result: ok. 3 passed` (0 failed).

- [ ] **Step 8: Run the full suite for regressions**

Run: `cargo test --release 2>&1 | grep "test result"`
Expected: both lines `ok`, 0 failed (unit count is now +3, i.e. 137 passed; integration 12 passed).

- [ ] **Step 9: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/library.rs src/database.rs src/platform.rs
git commit -m "feat(db): persist library snapshot as JSON blob keyed by root

Add serde derives to the library structs and a library_snapshot table with
save_library_snapshot/load_library_snapshot. Load returns None on a missing
row or unparsable JSON. Groundwork for desktop instant startup.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Desktop instant-startup wiring

Seed `self.library` from the snapshot at launch, and write the snapshot after every scan. This is egui glue verified by build + a live run (no headless unit test for the frame loop).

**Files:**
- Modify: `src/app.rs` — in `App::new()` load + seed the snapshot before spawning the libscan thread; write the snapshot in the libscan thread and at the end of `rescan()`.

**Interfaces:**
- Consumes: `Database::save_library_snapshot`, `Database::load_library_snapshot` (Task 1).

- [ ] **Step 1: Seed the library from the snapshot in `App::new()`**

In `src/app.rs`, `App::new()`, find the block after the DB is opened and before the libscan thread is spawned. It currently reads:

```rust
        let status = "Scanning library…".to_string();
        {
            let db = db.clone();
            let channels_root = channels_root.clone();
            let ctx = cc.egui_ctx.clone();
```

Replace the `let status = …;` line with a snapshot load that seeds both `library` and `status`:

```rust
        // Instant startup: if we persisted the library last run, show it
        // immediately while the (slow, disk-bound) rescan runs in the
        // background and swaps in fresh data via `library_load_rx`.
        let seeded_library: Vec<library::Channel> =
            db.load_library_snapshot(&channels_root).unwrap_or_default();
        let status = if seeded_library.is_empty() {
            "Scanning library…".to_string()
        } else {
            format!(
                "{} channels, {} videos (refreshing…)",
                seeded_library.len(),
                seeded_library.iter().map(|c| c.total_videos()).sum::<usize>()
            )
        };
        {
            let db = db.clone();
            let channels_root = channels_root.clone();
            let ctx = cc.egui_ctx.clone();
```

- [ ] **Step 2: Use the seeded library for the struct field**

Still in `App::new()`, find the line (a bit below the thread block):

```rust
        let library: Vec<library::Channel> = Vec::new();
```

Replace it with:

```rust
        let library: Vec<library::Channel> = seeded_library;
```

(`self.library` is assigned from this local in the `Self { … library, … }` literal. The scanning-state guard `library_load_rx.is_some() && self.library.is_empty()` now naturally yields to a seeded library — a populated `self.library` skips the spinner and renders instantly.)

- [ ] **Step 3: Write the snapshot from the libscan thread**

Still in `App::new()`, in the spawned libscan thread body, locate:

```rust
                    if let Err(e) = db.sync_search_index(&library::build_search_entries(&library)) {
                        eprintln!("search index sync failed: {e}");
                    }
                    let _ = library_load_tx.send(library);
                    ctx.request_repaint(); // wake update() to swap the result in
```

Insert a snapshot write **before** the `send` (which moves `library`):

```rust
                    if let Err(e) = db.sync_search_index(&library::build_search_entries(&library)) {
                        eprintln!("search index sync failed: {e}");
                    }
                    db.save_library_snapshot(&channels_root, &library);
                    let _ = library_load_tx.send(library);
                    ctx.request_repaint(); // wake update() to swap the result in
```

(`channels_root` and `db` are already the thread's cloned copies.)

- [ ] **Step 4: Write the snapshot at the end of `rescan()`**

In `src/app.rs`, `fn rescan(&mut self)`, find the closing status assignment:

```rust
        self.status = format!(
            "Rescanned: {} channels, {} videos",
            self.library.len(),
            self.library.iter().map(|c| c.total_videos()).sum::<usize>()
        );
    }
```

Insert a snapshot write just before the `self.status = …`:

```rust
        self.db.save_library_snapshot(&self.channels_root, &self.library);
        self.status = format!(
            "Rescanned: {} channels, {} videos",
            self.library.len(),
            self.library.iter().map(|c| c.total_videos()).sum::<usize>()
        );
    }
```

- [ ] **Step 5: Build**

Run: `cargo build --release 2>&1 | grep -E "^error" | head`
Expected: no output. (Ignore the ~39 upstream egui `f32: From<f64>` warnings.)

- [ ] **Step 6: Full test suite**

Run: `cargo test --release 2>&1 | grep "test result"`
Expected: both lines `ok`, 0 failed.

- [ ] **Step 7: Manual verification against the real library**

The frame loop can't be unit-tested; verify live. Reproduction recipe (Wayland/KWin box; foreground `sleep` is blocked in the harness Bash tool, so put launch+wait+capture in a script file and run that):

1. **First launch after building** (snapshot table exists but is empty for this root the first time, OR carries the previous run's data): run the GUI with `CWD=/home/luna` (its `config.toml` points at the real library), XWayland forced:
   ```bash
   env -u WAYLAND_DISPLAY WINIT_UNIX_BACKEND=x11 DISPLAY=:0 \
     /home/luna/code/catacomb/target/release/catacomb
   ```
   Let it finish one full scan and quit (this writes a fresh snapshot).
2. **Second launch:** run it again the same way and screenshot within ~2s of the window appearing (`xdotool search --pid`, `windowactivate`+`windowraise`+two `mousemove --window` nudges, then `import -window <wid> out.png`).
   - Expected: the library list is **already populated** (channels + videos visible), status shows `"… (refreshing…)"`, **no** "Scanning library…" spinner. A moment later the background scan lands and the status flips to `"N channels, M videos"`.
3. **First-ever launch (no snapshot):** `DELETE FROM library_snapshot;` via `sqlite3 <backup.directory>/catacomb.db` (or test in a throwaway dir), relaunch — expected: the Step-1 "Scanning library…" spinner shows, then populates. Confirms the fallback still works.

Record the two screenshots (seeded-instant and scanning-fallback) in the session scratchpad as evidence.

- [ ] **Step 8: Commit**

```bash
export SSH_AUTH_SOCK=/tmp/luna-ssh-agent.sock
git add src/app.rs
git commit -m "feat(desktop): instant startup from persisted library snapshot

Seed self.library from load_library_snapshot before spawning the scan thread,
so a warm launch renders the library immediately (status '… refreshing…')
instead of a blank/scanning window. Write the snapshot after every scan (the
startup thread and rescan). The background scan is unchanged and still swaps
in authoritative data; the snapshot only ever shows briefly-stale state.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Data model (`library_snapshot` table) → Task 1 Step 5. ✓
- Serialization (derives + `#[serde(default)]`) → Task 1 Step 1. ✓
- Write path (`save_library_snapshot`, both finalization points) → Task 1 Step 6, Task 2 Steps 3–4. ✓
- Read/seed path (`load_library_snapshot`, seed before spawn) → Task 1 Step 6, Task 2 Steps 1–2. ✓
- Edge cases: garbage JSON → None (Task 1 Step 3 test); root mismatch (keyed by root, Task 1 Step 6); struct evolution (`#[serde(default)]`, Task 1 Step 1). ✓
- Testing: round-trip / missing / garbage unit tests (Task 1) + manual seeded-vs-fallback verification (Task 2 Step 7). ✓
- Interaction with Step 1 scanning state (seeded library skips the spinner) → Task 2 Step 2 note. ✓

**Placeholder scan:** none — all steps carry concrete code/commands. The one conditional ("if `Platform` already impls `Default`") is a real branch with both outcomes specified.

**Type consistency:** `save_library_snapshot(&self, root: &Path, library: &[Channel])` and `load_library_snapshot(&self, root: &Path) -> Option<Vec<Channel>>` are named and typed identically in Task 1 (definition) and Task 2 (call sites). Snapshot written before `send` moves `library`. ✓
