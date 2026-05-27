//! Free-space query for the download preflight check.
//!
//! Rust stdlib doesn't have `available_space` on stable, so we call
//! `statvfs(3)` via libc on Unix. On non-Unix the function returns
//! `None`, which the caller treats as "skip the check" — we'd rather
//! let yt-dlp run and surface a real ENOSPC error than refuse to start
//! based on no data.

/// Return the bytes of free space available to the calling user on the
/// filesystem holding `path`. `None` means the query failed (path missing,
/// non-Unix host, libc error) and the caller should skip the preflight.
///
/// Uses `f_frsize * f_bavail` — `f_bavail` is the "available to
/// unprivileged user" count which already excludes the reserved-root
/// blocks; that's the number a non-root download cares about.
#[cfg(unix)]
pub fn available_bytes(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: libc::statvfs reads a valid C string and writes into a
    // zero-initialised libc::statvfs we own. Returning 0 on success is
    // documented; we check that before reading the fields. mem::zeroed()
    // is safe for plain-data libc structs.
    let mut buf: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut buf) };
    if rc != 0 {
        return None;
    }
    // f_frsize is the underlying block size (may differ from f_bsize on
    // some filesystems). f_bavail is in those blocks. Cast both to u64
    // since libc declares them as platform-dependent types.
    let frsize = buf.f_frsize as u64;
    let bavail = buf.f_bavail as u64;
    Some(frsize.saturating_mul(bavail))
}

#[cfg(not(unix))]
pub fn available_bytes(_path: &std::path::Path) -> Option<u64> {
    None
}

/// Floor below which we refuse to start a new download. Picked at the low
/// end of a "typical 1080p YouTube video" range — most videos are smaller
/// than this, but starting with less than 500 MB free is a near-certain
/// path to an aborted job and a half-written file. Configurable later if
/// a user complains; for now a fixed sane default is enough.
pub const FREE_SPACE_FLOOR_BYTES: u64 = 500 * 1024 * 1024;

/// Format a byte count for human display. Duplicates a similar helper in
/// app.rs because the modules can't easily share one (app.rs depends on
/// eframe). 3 GB / 250 MB / 15 KB-style rounding.
pub fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{} MB", bytes / 1_048_576)
    } else if bytes >= 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn root_path_has_some_free_space() {
        // / always exists on Unix; if statvfs returns Some(_) at all,
        // the query plumbing works. We don't assert a specific number
        // because CI runners' disks vary.
        let bytes = available_bytes(std::path::Path::new("/"));
        assert!(bytes.is_some(), "statvfs(/) should succeed on Unix");
    }

    #[test]
    #[cfg(unix)]
    fn missing_path_returns_none() {
        // statvfs on a nonexistent path returns -1; we surface that as
        // None so the caller skips the preflight rather than refusing
        // every download.
        let bytes = available_bytes(std::path::Path::new("/this/path/should/not/exist/anywhere"));
        assert!(bytes.is_none());
    }
}
