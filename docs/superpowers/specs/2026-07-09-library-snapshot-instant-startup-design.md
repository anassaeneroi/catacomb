# Persistent library snapshot — desktop instant startup

**Date:** 2026-07-09
**Status:** Approved design, ready for implementation plan
**Scope:** Desktop GUI only

## Problem

On the desktop GUI, the first launch after a reboot shows an apparently-empty
library for ~60 seconds before it populates. Root cause (confirmed by
reproduction against the real 73 MB library on `/dev/mapper/InannaBeloved`,
LUKS + btrfs+zstd):

| Filesystem cache | Scan time |
|---|---|
| Cold (first launch after boot) | ~64 s |
| Warm (subsequent launches) | ~96 ms |

The `library::scan_channels_with_cache` startup scan runs on a background thread
and delivers the library over `library_load_rx`; the drain in `App::update()`
swaps it in. The pipeline is correct — the library *does* populate after the
scan — but the cold scan is dominated by walking the directory tree and
`std::fs::metadata`-stat'ing ~11k sidecar/video files through the
encryption/compression layer. The existing `info_cache` only saves JSON
*parsing*, not the stats, so it can't shorten a cold scan. During those ~64 s
the content area looked empty, so the user assumed the app was broken and hit
Rescan (which appears to "fix" it only because the OS cache is warm by then).

Step 1 (already shipped in this work) added a **scanning state** — a centered
spinner + "Scanning library…" — so the wait no longer looks like a broken empty
library. This spec covers Step 2: make subsequent launches render **instantly**.

## Goal

At desktop launch, render the last-known library immediately from a persisted
snapshot, then background-rescan and silently swap in the fresh result. The
scan remains authoritative, so the instant view can only ever be briefly
*stale*, never *wrong*.

## Non-goals

- Web server startup (long-running; scans once and stays up — no instant-startup
  value). The snapshot is written from shared scan code but only the desktop
  reads it.
- Skipping or speeding up the scan itself. The scan runs unchanged.
- Persisting watched/positions/flags in the snapshot (those are separate DB maps
  already loaded synchronously and applied at render).

## Approach (chosen: serialized blob)

Serialize the scanned `Vec<library::Channel>` to JSON and store it in a
single-row-per-root table. At launch, load and seed `self.library` before the
scan thread is spawned. Purely a display optimization layered on the existing,
unchanged scan/drain machinery.

Rejected alternatives:
- **Normalized `snapshot_channels`/`snapshot_videos` tables** — far more code
  (schema, upserts, joins, reconstruction) for no benefit; we only ever read the
  whole library back at once. Violates YAGNI.
- **Extend `info_cache` to skip the stats** — to render instantly we'd have to
  trust the cache and skip `metadata()` entirely, which misses on-disk changes
  and still requires a directory walk. Doesn't achieve instant render.

## Data model

Idempotent `CREATE TABLE IF NOT EXISTS` in `init_schema()`:

```sql
CREATE TABLE IF NOT EXISTS library_snapshot (
    root      TEXT PRIMARY KEY,   -- channels_root.display().to_string()
    json      TEXT NOT NULL,      -- serde_json of Vec<library::Channel>
    saved_at  INTEGER NOT NULL    -- unix secs, for debugging/telemetry
);
```

Keyed by `root` so a changed `backup.directory` never loads another library's
snapshot. One row per root via `INSERT OR REPLACE`.

## Serialization

Add `#[derive(Serialize, Deserialize)]` to:

- `library::Channel`, `library::Video`, `library::Playlist`, `library::ChannelMeta`
- `platform::Platform` (enum)
- `download_options::DownloadOptions`

`PathBuf` and `Option<T>` serialize natively via serde. Add `#[serde(default)]`
on `Video` and `Channel` fields so a future field change deserializes an old
snapshot (missing fields default) instead of discarding the whole thing. If
deserialization still fails, the loader returns `None` and the app falls back to
the Step-1 scanning state — safe.

`DownloadOptions` and `folder_id` are hydrated onto the library from SQLite
after the scan (`apply_channel_options` / `apply_channel_folders`) before the
snapshot is written, so the persisted snapshot already carries them.

## Write path

```rust
// database.rs — error-swallowing, like info_cache_put_many.
pub fn save_library_snapshot(&self, root: &str, library: &[Channel]);
```

A single quick blob write; failure is non-fatal (just means no fresh snapshot
next launch). Serialize with `serde_json::to_string`; on serialize error, no-op.

Called from **both** scan finalization points:

1. The startup libscan thread in `App::new()`, right after `send(library)` — off
   the UI thread, so it never blocks paint. (Serialize a clone or borrow before
   `send` moves the value; see plan for the exact ordering.)
2. The end of `App::rescan()`.

## Read / seed path

```rust
// database.rs — None on missing row OR deserialize failure.
pub fn load_library_snapshot(&self, root: &str) -> Option<Vec<Channel>>;
```

In `App::new()`, **before** spawning the scan thread:

- If `load_library_snapshot(root)` returns `Some(lib)`: `self.library = lib`,
  set `status` to e.g. `"{n} channels (refreshing…)"`, and set an initial
  `library_generation` so the card cache is valid.
- Either way, spawn the scan thread unconditionally. The existing drain in
  `update()` swaps in the fresh result and bumps `library_generation`.

## Data flow

```
Launch → load_library_snapshot(root)
   ├─ Some(lib): self.library = lib      → instant populated render
   │              spawn scan thread → drain swaps in fresh data (silent)
   └─ None (first run / corrupt): empty  → Step-1 scanning state
                  spawn scan thread → drain populates
```

## Edge cases

- **Stale entries**: a since-deleted video shows until the rescan swaps;
  Play before then fails as any missing file would. Self-correcting.
- **Thumbnails**: `thumb_path` is in the snapshot, so thumbnails stream in via
  the existing lazy decode workers immediately.
- **Watched/positions/flags**: not in the snapshot; separate maps loaded
  synchronously — no staleness.
- **Root mismatch**: keyed by root → no cross-library bleed.
- **Big deletion since last launch**: snapshot briefly larger than reality;
  corrected on rescan.
- **Struct evolution**: `#[serde(default)]` + `None`-on-failure loader.
- **Write contention** (web server on same DB): single-row write, quick;
  failure swallowed.

## Testing

Unit (`database.rs`):
- `save_library_snapshot` → `load_library_snapshot` round-trips a small
  `Vec<Channel>` with a nested video and a playlist faithfully.
- `load_library_snapshot` returns `None` for an unknown root.
- `load_library_snapshot` returns `None` for a row containing garbage JSON.

Manual:
- Launch against the real library twice: confirm the second launch renders the
  library instantly (no scanning state), then silently refreshes when the
  background scan lands.
- Confirm first-ever launch (no snapshot row) still shows the Step-1 scanning
  state.

## Interaction with Step 1

Complementary: Step 1's scanning state covers the no-snapshot case (first run or
corrupt snapshot); the snapshot covers every subsequent run. The scanning-state
guard (`library_load_rx.is_some() && self.library.is_empty()`) naturally yields
to the snapshot because seeding makes `self.library` non-empty.
