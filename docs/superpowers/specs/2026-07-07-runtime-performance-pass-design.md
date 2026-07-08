# Runtime performance pass — Phase 1 (low-risk wins)

**Date:** 2026-07-07
**Scope:** Runtime performance only, no file/structure refactoring. Three
target areas were requested (desktop GUI, web server, library/DB/scan). This
phase ships the low-risk, behavior-preserving wins; the one invasive change
(desktop row virtualization) is deferred to its own follow-up.

## Goal

Make the app faster without changing observable behavior:

- Cold-cache library scans (thousands of `info.json` sidecars) commit to
  SQLite far fewer times and stop re-parsing SQL per row.
- Every SQLite connection uses WAL + sane durability/concurrency pragmas,
  which also removes `SQLITE_BUSY` failures under the parallel scanner
  (a stability gain).
- The desktop card-sort path stops allocating lowercased strings inside sort
  comparators.
- The release binary drops unwinding overhead (`panic = "abort"`), and a
  documented opt-in gives local builders SIMD tuning.

Non-goals: desktop row virtualization (deferred), any web-handler change (the
web path is already ETag + body-cached and inherits the DB speedups), and any
structural/file refactor.

## Changes

### 1. SQLite connection pragmas via `with_init` — `database.rs`

Today `Database::open` / `open_in_memory` build the pool with no per-connection
initialization, so every pooled connection runs on SQLite defaults
(`journal_mode=DELETE`, `synchronous=FULL`, no `busy_timeout`). Cold-cache
scans therefore fsync once per `info_cache_put`, and concurrent writers from
the parallel scanner can hit `SQLITE_BUSY` immediately instead of waiting.

Set pragmas on **every** connection through
`SqliteConnectionManager::file(path).with_init(|conn| { ... })`:

- `PRAGMA journal_mode = WAL;` — better write throughput + reader/writer
  concurrency. Creates `catacomb.db-wal` / `catacomb.db-shm` sidecars (SQLite
  manages their lifecycle). **Must be gitignored** alongside `catacomb.db`.
- `PRAGMA synchronous = NORMAL;` — safe with WAL; commits append to the WAL
  without a full fsync per commit.
- `PRAGMA busy_timeout = 5000;` — wait up to 5s for a lock rather than
  failing a concurrent write. Removes the parallel-scanner `SQLITE_BUSY`
  failure mode.
- `PRAGMA foreign_keys = ON;` — FK enforcement is **per-connection**. It is
  currently set once on a single connection (`init_schema`, ~line 410), which
  means most pooled connections run with FKs *off*. Moving it into `with_init`
  makes enforcement consistent. The standalone `execute("PRAGMA foreign_keys
  = ON")` line in `init_schema` is then removed as redundant.

The in-memory pool (`open_in_memory`, used by tests) gets the same `with_init`
except **not** WAL (an in-memory DB has no file; WAL is a no-op/undesirable
there) — apply `foreign_keys`, `busy_timeout`, and leave journal/synchronous
at their in-memory defaults.

`with_init` runs on each new physical connection; because pragmas like
`synchronous`, `busy_timeout`, and `foreign_keys` are per-connection they must
live here, not in a one-shot at open time. `journal_mode=WAL` persists in the
file header but re-asserting it per connection is harmless and idempotent.

### 2. `prepare_cached` for the per-video scan queries — `database.rs`

`info_cache_get` and `info_cache_put` call `conn.prepare(...)` /
`conn.execute(...)` — a fresh SQL parse on every one of thousands of videos per
scan. Switch both to `conn.prepare_cached(...)`, which reuses the compiled
statement from the connection's statement cache. Behavior is identical; only
the per-call SQL-compile cost is removed.

Other one-shot queries (folder loads, settings reads, etc.) are **not** changed
— they run once per operation, so statement caching buys nothing there. Scope
stays on the two functions that run in the per-video inner loop.

### 3. Batch cold-cache writes in one transaction — `library.rs` + `database.rs`

`enrich_with_cache` currently upserts each cache **miss** via an individual
auto-committed `info_cache_put`. On a first-ever (cold) scan, every video is a
miss, so a channel with N videos does N separate commits.

Restructure `enrich_with_cache` so cache misses are collected into a
`Vec<(path, mtime, dur, has_chapters, upload_date)>` while building the
`Video`s, then flushed once via a new
`Database::info_cache_put_many(&[...])` that wraps all inserts in a single
transaction (using a `prepare_cached` statement inside the tx). Cache **hits**
still short-circuit with no write, exactly as today. Net effect: a cold scan of
a channel commits once instead of once-per-video. With WAL+NORMAL already in
place this is a smaller marginal win than it would be on the old journal, but
it keeps cold scans cheap and is a clean, contained change.

If threading the miss-collection through the existing closure proves to tangle
`enrich_with_cache` readability, fall back to keeping per-row `info_cache_put`
(now `prepare_cached` + WAL, already much cheaper) and drop this item — it is
the lowest-priority change in the pass.

### 4. Sort without per-comparison allocations — `app.rs`

In `compute_cards`, the `SortMode::Title` and `SortMode::ChannelAsc` arms call
`.to_lowercase()` inside the `sort_by` comparator, allocating two strings per
comparison (O(n log n) allocations). Replace with `sort_by_cached_key` (or a
precomputed key vector) so each element is lowercased once. This arm only runs
when the sort/search/view key changes (the result is cached by `cards_take`),
so it is not a per-frame cost — but the fix is nearly free and removes a
needless allocation storm on large libraries when the user changes sort.

### 5. Release-profile compiler settings — `Cargo.toml`

The `[profile.release]` block is already well-tuned (`opt-level = 3`,
`lto = "thin"`, `codegen-units = 1`, `strip = "debuginfo"`). Full LTO is
deliberately avoided (it broke linking with bundled SQLite + rust-lld — keep
thin). Two runtime-oriented changes:

- **Add `panic = "abort"`.** Removes unwinding tables / landing pads: smaller
  binary, marginally faster hot paths, faster panic→exit. The crash handler
  installs a `panic::set_hook` ([crash.rs](../../../src/crash.rs)) which
  **still fires** under abort, so crash logging is preserved. `cargo test`
  (incl. `cargo test --release`) is unaffected — Cargo automatically forces
  `-C panic=unwind` when building test/bench harnesses, so the existing suite
  and the only `catch_unwind` (a `#[cfg(test)]` block in `crash.rs`) keep
  working. Accepted tradeoff: a panic inside a `parallel_map` scan worker
  ([library.rs](../../../src/library.rs)) aborts the whole process instead of
  failing just that worker — acceptable because such a panic indicates a
  genuine invariant break, and the scan workers only do fallible I/O/JSON work
  that already uses `.ok()` rather than panicking.

- **`target-cpu`: local opt-in only, no repo change.** A committed
  `target-cpu=native` / `x86-64-v3` would SIGILL on any CPU older than the
  build host and would leak into the CI Windows cross-compile (it's a
  `RUSTFLAGS` setting, not a profile key). Instead, document in
  [docs/PACKAGING.md](../../../docs/PACKAGING.md) (or the build section of the
  README) that users compiling for their own machine can get SIMD gains with
  `RUSTFLAGS="-C target-cpu=native" cargo build --release`. Distributed and CI
  builds stay portable.

## Out of scope / deferred

- **Desktop row virtualization** (`ScrollArea::show_rows` for List/Card/Grid).
  Highest per-frame win but changes scroll internals and needs fixed row
  heights (title line-clamping). Deferred to a dedicated follow-up so it can be
  tested in isolation.
- **Web handlers.** Already ETag + body-cached; no change this pass.

## Testing / verification

- `cargo build --release` clean.
- `cargo test --release` — existing `tests/api.rs` integration tests spawn the
  real `--web` binary against a throwaway dir and must still pass (this
  exercises the new pragmas end-to-end, including WAL sidecar creation).
- Add a unit test for `info_cache_put_many` round-trip (put many → get each
  back) if item #3 lands.
- Manual smoke: run against a real library dir, confirm scan completes and the
  `catacomb.db-wal` file appears, and a warm rescan is a fast no-op.
- `.gitignore`: add `catacomb.db-wal` and `catacomb.db-shm` (or `catacomb.db*`).
- Confirm `cargo test --release` still builds/passes with `panic = "abort"`
  set (Cargo forces unwind for the harness — this is the check that proves it).
- Cross-compile smoke: `cargo check --target x86_64-pc-windows-gnu` stays green
  (per CLAUDE.md) — `panic = "abort"` is portable, but verify.

## Risk

Low. All four changes preserve observable behavior. The only new on-disk
artifact is the WAL sidecar pair, handled by gitignore. `busy_timeout` and
`foreign_keys` becoming consistent across connections is strictly more correct
than today.
