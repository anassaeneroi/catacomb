//! Library health scanning and repair.
//!
//! Two kinds of problems are detected over the scanned library:
//!
//! * **Duplicates** — the same YouTube video ID stored more than once (either
//!   under different titles in one folder, or across folders). Each duplicate
//!   group lists every copy with a "score" so the UI can recommend keeping the
//!   most complete one and removing the rest.
//! * **Missing assets** — a downloaded video lacking its thumbnail, `info.json`,
//!   or `.description` sidecar. These can be re-fetched from YouTube with yt-dlp
//!   (subtitles are fetched alongside, since their absence isn't a reliable
//!   signal — many videos legitimately have none).
//!
//! Deletion is never automatic: [`scan`] only reports, and [`remove_files`]
//! refuses any path outside the library root.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::library::{Channel, Video};

/// One stored copy of a video (its on-disk files plus a completeness score).
#[derive(Serialize, Clone)]
pub struct DuplicateCopy {
    /// Human-readable directory the copy lives in, relative to the library root.
    pub location: String,
    /// Every file belonging to this copy (video + all sidecars).
    pub files: Vec<PathBuf>,
    /// Size of the video file in bytes, if present.
    pub file_size: Option<u64>,
    /// Whether an actual video file (not just sidecars) is present.
    pub has_video: bool,
    /// True for the copy the UI recommends keeping (best score in the group).
    pub recommended_keep: bool,
}

/// A set of copies that all share one video ID.
#[derive(Serialize, Clone)]
pub struct DuplicateGroup {
    pub id: String,
    pub title: String,
    pub copies: Vec<DuplicateCopy>,
}

/// A video that is missing one or more sidecar assets.
#[derive(Serialize, Clone)]
pub struct MissingAssets {
    pub id: String,
    pub title: String,
    /// Directory containing the video, relative to the library root.
    pub location: String,
    pub missing_thumbnail: bool,
    pub missing_info: bool,
    pub missing_description: bool,
}

/// The full result of a [`scan`].
#[derive(Serialize, Clone, Default)]
pub struct HealthReport {
    pub duplicates: Vec<DuplicateGroup>,
    pub missing: Vec<MissingAssets>,
}

/// The directory a video lives in, inferred from whichever path is known.
fn video_dir(v: &Video) -> Option<PathBuf> {
    v.video_path
        .as_ref()
        .or(v.info_path.as_ref())
        .or(v.thumb_path.as_ref())
        .or(v.description_path.as_ref())
        .or(v.subtitles.first().map(|s| &s.path))
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Display a path relative to `root` (falls back to the full path).
fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).display().to_string()
}

/// Every file on disk whose name begins with `<stem>.` in `dir`.
///
/// This captures the video plus all sidecars (thumbnail, info.json, description,
/// subtitles, live_chat.json, …) without listing each suffix explicitly. The
/// trailing dot prevents matching a different video whose stem is a prefix.
fn files_for_stem(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let prefix = format!("{stem}.");
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// Scan the library for duplicates and missing assets.
pub fn scan(root: &Path, channels: &[Channel]) -> HealthReport {
    // Collect every video together with its source channel name.
    let mut all: Vec<&Video> = Vec::new();
    for ch in channels {
        all.extend(ch.videos.iter());
        for pl in &ch.playlists {
            all.extend(pl.videos.iter());
        }
    }

    // ── Duplicates: group by video ID ──────────────────────────────────────
    let mut by_id: BTreeMap<&str, Vec<&Video>> = BTreeMap::new();
    for v in &all {
        by_id.entry(v.id.as_str()).or_default().push(v);
    }

    let mut duplicates = Vec::new();
    for (id, vids) in &by_id {
        if vids.len() < 2 {
            continue;
        }
        // Build a copy per video, then drop phantoms with no locatable files
        // (e.g. a stem known only by a leftover live_chat.json).
        let mut copies: Vec<DuplicateCopy> = vids
            .iter()
            .filter_map(|v| {
                let dir = video_dir(v)?;
                let files = files_for_stem(&dir, &v.stem);
                if files.is_empty() {
                    return None;
                }
                Some(DuplicateCopy {
                    location: rel(root, &dir),
                    files,
                    file_size: v.file_size,
                    has_video: v.video_path.is_some(),
                    recommended_keep: false,
                })
            })
            .collect();
        if copies.len() < 2 {
            continue; // not a real duplicate once phantoms are removed
        }
        // Score each copy: a present video file dominates, then file size,
        // then the number of sidecar files. The highest score is kept.
        let best_idx = copies
            .iter()
            .enumerate()
            .max_by_key(|(_, c)| (c.has_video, c.file_size.unwrap_or(0), c.files.len()))
            .map(|(i, _)| i)
            .unwrap_or(0);
        copies[best_idx].recommended_keep = true;
        let title = vids
            .iter()
            .find(|v| v.video_path.is_some())
            .unwrap_or(&vids[0])
            .title
            .clone();
        duplicates.push(DuplicateGroup {
            id: id.to_string(),
            title,
            copies,
        });
    }

    // ── Missing assets: per video, only for ones with an actual video file ──
    // Dedup by ID so a duplicate group isn't reported many times here.
    let mut seen = std::collections::HashSet::new();
    let mut missing = Vec::new();
    for v in &all {
        if v.video_path.is_none() || !seen.insert(v.id.as_str()) {
            continue;
        }
        let missing_thumbnail = v.thumb_path.is_none();
        let missing_info = v.info_path.is_none();
        let missing_description = v.description_path.is_none();
        if missing_thumbnail || missing_info || missing_description {
            missing.push(MissingAssets {
                id: v.id.clone(),
                title: v.title.clone(),
                location: video_dir(v).as_deref().map(|d| rel(root, d)).unwrap_or_default(),
                missing_thumbnail,
                missing_info,
                missing_description,
            });
        }
    }

    HealthReport { duplicates, missing }
}

/// True if `target` resolves to a location inside `root`.
fn is_within(root: &Path, target: &Path) -> bool {
    match (root.canonicalize(), target.canonicalize()) {
        (Ok(r), Ok(t)) => t.starts_with(r),
        _ => false,
    }
}

/// Delete the given files, refusing any path that escapes `root`.
///
/// Returns the number of files removed and a list of human-readable errors
/// (including refusals for out-of-bounds paths).
pub fn remove_files(root: &Path, paths: &[PathBuf]) -> (usize, Vec<String>) {
    let mut removed = 0;
    let mut errors = Vec::new();
    for p in paths {
        if !is_within(root, p) {
            errors.push(format!("refused (outside library): {}", p.display()));
            continue;
        }
        match std::fs::remove_file(p) {
            Ok(()) => removed += 1,
            Err(e) => errors.push(format!("{}: {e}", p.display())),
        }
    }
    (removed, errors)
}

/// Look up a video's directory and filename stem by ID, for repair targeting.
/// Returns `(dir, stem)` of the first matching copy with a known location.
pub fn locate(channels: &[Channel], id: &str) -> Option<(PathBuf, String)> {
    for ch in channels {
        for v in ch.videos.iter().chain(ch.playlists.iter().flat_map(|p| p.videos.iter())) {
            if v.id == id {
                if let Some(dir) = video_dir(v) {
                    return Some((dir, v.stem.clone()));
                }
            }
        }
    }
    None
}
