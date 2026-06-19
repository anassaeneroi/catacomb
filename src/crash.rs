//! Persistent panic logging so a crashed catacomb leaves a paper trail.
//!
//! GUI users don't see stderr (the binary is launched without a terminal
//! from a `.desktop` file or the IDE), so panics today vanish entirely.
//! This module installs a `panic::set_hook` early in `main` that appends
//! a structured entry to `<channels_root>/catacomb.crash.log` for every
//! panic from any thread.
//!
//! The default panic handler still runs after ours so dev runs keep
//! getting the familiar stderr output.
//!
//! # Format
//!
//! Each entry is a small block — easy to grep, easy to attach to a bug
//! report:
//!
//! ```text
//! ── 2026-05-27T18:42:11Z  thread: "tokio-runtime-worker"  ──────────────
//! panic at src/web.rs:482:9:
//!   called `Option::unwrap()` on a `None` value
//! backtrace:
//!   …
//! ```
//!
//! A backtrace is captured only when `RUST_BACKTRACE` is set; we don't
//! force it because capture is slow and most users won't have symbols.

use std::fs::OpenOptions;
use std::io::Write;
use std::panic;
use std::path::{Path, PathBuf};

/// Cap the log at ~256 KB. When the file grows past this, the next write
/// rotates it to `*.1` (overwriting any previous rotation). Bounded both
/// in disk use and in how many panics we keep around — the most recent
/// dozen panics is what a bug-report attacher actually wants.
const LOG_SIZE_CAP_BYTES: u64 = 256 * 1024;

/// Install a global panic hook that logs to `<dir>/catacomb.crash.log`
/// while preserving the default stderr behavior.
///
/// `dir` is the same directory the SQLite database lives in (typically
/// `channels_root` in config). Pass the *absolute* path so the log
/// remains discoverable regardless of where the user's `cwd` was when
/// they launched the binary.
///
/// Safe to call once at process startup. Calling it more than once
/// replaces the previous hook (which is fine: the new dir wins).
pub fn install(dir: &Path) {
    let path = dir.join("catacomb.crash.log");
    // Don't fail startup if the parent dir doesn't exist yet — log it,
    // continue. The DB-open path further down the chain will surface a
    // permission / missing-dir error in a more legible way.
    let _ = std::fs::create_dir_all(dir);

    // Stash the default hook so we can chain to it: our hook does the
    // file write, then defers to the standard formatter that prints to
    // stderr (useful in dev / when run from a terminal).
    let default = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Best-effort log. A panic inside the panic handler is a hard
        // abort — wrap the IO in a closure that swallows all errors.
        let _ = write_entry(&path, info);
        default(info);
    }));
}

/// Append a single panic entry to the log, rotating if the file is over
/// the size cap. All IO errors are intentionally swallowed; this is a
/// best-effort diagnostic, not a load-bearing data path.
fn write_entry(path: &Path, info: &panic::PanicHookInfo<'_>) -> std::io::Result<()> {
    // Rotation: if the existing log is over the cap, move it aside.
    // A single rename is cheaper than parsing+truncating and gives the
    // user one previous generation for free.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() >= LOG_SIZE_CAP_BYTES {
            let rotated = with_suffix(path, ".1");
            let _ = std::fs::rename(path, &rotated);
        }
    }

    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    let ts = now_iso8601();
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("<unnamed>");
    let location = info
        .location()
        .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
        .unwrap_or_else(|| "<unknown>".to_string());
    // PanicHookInfo carries the payload as `&dyn Any`; downcast the two
    // forms the stdlib produces (`&str` for `panic!("literal")` and
    // `String` for `panic!("{x}")`).
    let payload = info.payload();
    let msg = payload
        .downcast_ref::<&str>()
        .copied()
        .map(str::to_string)
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());

    writeln!(f)?;
    writeln!(f, "── {ts}  thread: {thread_name:?}  ─────────────────────────")?;
    writeln!(f, "panic at {location}:")?;
    for line in msg.lines() {
        writeln!(f, "  {line}")?;
    }
    // Capture a backtrace only when RUST_BACKTRACE is set (matches the
    // default panic-handler behavior; capturing is otherwise too slow on
    // hot paths and most users won't have symbols anyway).
    if std::env::var_os("RUST_BACKTRACE").is_some() {
        let bt = std::backtrace::Backtrace::force_capture();
        writeln!(f, "backtrace:")?;
        for line in bt.to_string().lines() {
            writeln!(f, "  {line}")?;
        }
    }
    f.flush()?;
    Ok(())
}

/// Build `<path>.<suffix>` next to the original — used to rotate logs.
/// We append `.1` rather than `.<count>` because keeping one previous
/// generation is enough; more would just bloat the install directory.
fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    PathBuf::from(s)
}

/// Minimal RFC3339-ish timestamp without pulling in `chrono`. Goes
/// `2026-05-27T18:42:11Z` — accurate to the second, UTC. We compute it
/// from `SystemTime` via the Howard Hinnant civil-time formula so we
/// don't need a date library.
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (y, mo, d, h, mi, s) = civil_from_unix(secs);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// Convert UNIX seconds → (year, month, day, hour, minute, second) UTC.
/// Adapted from Howard Hinnant's `civil_from_days`
/// (<https://howardhinnant.github.io/date_algorithms.html#civil_from_days>).
/// All branchless integer math; no leap-year edge cases.
fn civil_from_unix(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, h, mi, s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_round_trips_known_dates() {
        // 2024-01-01T00:00:00Z is 1704067200
        assert_eq!(civil_from_unix(1_704_067_200), (2024, 1, 1, 0, 0, 0));
        // 2026-05-22T18:42:11Z is 1779475331
        assert_eq!(civil_from_unix(1_779_475_331), (2026, 5, 22, 18, 42, 11));
        // Unix epoch
        assert_eq!(civil_from_unix(0), (1970, 1, 1, 0, 0, 0));
        // Leap day
        assert_eq!(civil_from_unix(1_582_934_400), (2020, 2, 29, 0, 0, 0));
    }

    #[test]
    fn write_entry_appends_panic_to_file() {
        // Build a real PanicHookInfo via std::panic::catch_unwind. We
        // can't construct one directly (the type's fields are private).
        // The simplest portable approach: install our hook against a
        // tempfile, then trigger a controlled panic in another thread
        // (`std::thread::spawn` so the panic doesn't unwind the test
        // runner), and assert the file contents afterward.
        let mut tmp = std::env::temp_dir();
        tmp.push(format!("catacomb-crash-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let log = tmp.join("catacomb.crash.log");

        // Install the hook scoped to this test. We swap it back at the
        // end so other tests run on the default hook.
        let old = std::panic::take_hook();
        super::install(&tmp);
        let _ = std::thread::Builder::new()
            .name("crash-test".into())
            .spawn(|| {
                panic!("deliberate test panic — please ignore");
            })
            .unwrap()
            .join();
        std::panic::set_hook(old);

        let body = std::fs::read_to_string(&log).expect("crash log should exist");
        assert!(body.contains("deliberate test panic"));
        assert!(body.contains("crash-test"));
        assert!(body.contains("panic at "));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn iso_format_has_expected_shape() {
        let s = now_iso8601();
        // YYYY-MM-DDTHH:MM:SSZ — 20 chars
        assert_eq!(s.len(), 20);
        assert!(s.ends_with('Z'));
        assert_eq!(s.as_bytes()[4], b'-');
        assert_eq!(s.as_bytes()[7], b'-');
        assert_eq!(s.as_bytes()[10], b'T');
    }
}
