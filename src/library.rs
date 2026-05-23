//! Scanning the `channels/` directory tree into channels, playlists, and videos.
//!
//! # Directory layout expected
//!
//! ```text
//! channels/
//!   <channel-name>/
//!     Title [VIDEO_ID].mkv
//!     Title [VIDEO_ID].webp          ← thumbnail
//!     Title [VIDEO_ID].description
//!     Title [VIDEO_ID].info.json
//!     Title [VIDEO_ID].en.vtt        ← subtitle (lang = "en")
//!     <playlist-name>/
//!       Title [VIDEO_ID].mkv
//!       …
//! ```
//!
//! Files that don't match the `Title [ID].ext` naming convention are silently
//! ignored.  Hidden directories (name starts with `.`) and directories that
//! contain no recognisable video files are skipped.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "webm", "m4v", "mov", "avi"];
const AUDIO_EXTS: &[&str] = &["mp3", "m4a", "opus", "flac", "ogg", "wav", "aac"];
const THUMB_EXTS: &[&str] = &["webp", "jpg", "jpeg", "png"];

/// A single WebVTT subtitle track discovered alongside a video file.
#[derive(Clone, Debug)]
pub struct Subtitle {
    /// ISO 639-1/2 language code extracted from the `.lang.vtt` filename suffix.
    pub lang: String,
    pub path: PathBuf,
}

/// An audio track in the music library.
#[derive(Clone, Debug)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub path: PathBuf,
    pub thumb_path: Option<PathBuf>,
    #[allow(dead_code)]
    pub info_path: Option<PathBuf>,
    pub duration_secs: Option<f64>,
    pub file_size: Option<u64>,
}

/// A fully enriched video entry, ready to serve to the UI.
#[derive(Clone, Debug)]
pub struct Video {
    /// yt-dlp video ID (the part inside `[…]` in the filename).
    pub id: String,
    pub title: String,
    #[allow(dead_code)]
    pub stem: String,
    pub video_path: Option<PathBuf>,
    pub thumb_path: Option<PathBuf>,
    pub description_path: Option<PathBuf>,
    /// Path to the `.info.json` sidecar — used to read duration, chapters, etc.
    pub info_path: Option<PathBuf>,
    pub subtitles: Vec<Subtitle>,
    pub has_live_chat: bool,
    /// Duration read from `info.json`; `None` if the sidecar is missing.
    pub duration_secs: Option<f64>,
    /// Whether `info.json` lists a non-empty `chapters` array. Cached at scan
    /// time so the web layer needn't re-read and parse the sidecar per request.
    pub has_chapters: bool,
    /// Size of the video file on disk; `None` if the video file is missing.
    pub file_size: Option<u64>,
    /// Upload date as `YYYYMMDD` (yt-dlp's native format from info.json).
    /// `None` if the info.json sidecar is missing or lacks the field.
    pub upload_date: Option<String>,
}

/// A sub-directory inside a channel that contains videos (treated as a playlist).
#[derive(Clone, Debug)]
pub struct Playlist {
    pub name: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub videos: Vec<Video>,
}

/// Channel-level metadata pulled from the first available `info.json`.
#[derive(Clone, Debug)]
pub struct ChannelMeta {
    pub subscriber_count: Option<u64>,
    pub channel_url: Option<String>,
    pub uploader: Option<String>,
}

/// A top-level channel directory with all its videos and playlists.
#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub path: PathBuf,
    /// Videos stored directly inside the channel directory (not in a sub-folder).
    pub videos: Vec<Video>,
    /// Sub-directories that contain at least one video.
    pub playlists: Vec<Playlist>,
    pub meta: Option<ChannelMeta>,
    /// Cached sum of `videos.len() + playlists[*].videos.len()`.
    pub total_videos_cached: usize,
    /// Cached sum of all video file sizes.
    pub total_size_cached: u64,
}

impl Channel {
    pub fn total_videos(&self) -> usize {
        self.total_videos_cached
    }

    /// Iterate over every [`Video`] in this channel, including those nested
    /// inside playlists. Used widely; previously open-coded at each call site.
    pub fn all_videos(&self) -> impl Iterator<Item = &Video> {
        self.videos
            .iter()
            .chain(self.playlists.iter().flat_map(|p| p.videos.iter()))
    }
}

/// Find a video by ID across a slice of channels. Returns the matching
/// [`Video`] alongside the channel it belongs to.
pub fn find_video<'a>(channels: &'a [Channel], id: &str) -> Option<(&'a Video, &'a Channel)> {
    for ch in channels {
        if let Some(v) = ch.all_videos().find(|v| v.id == id) {
            return Some((v, ch));
        }
    }
    None
}

/// Scan `root` for channel directories and return them sorted alphabetically.
///
/// Skips hidden directories (names starting with `.`) and directories that
/// contain no recognisable video files.
///
/// Each channel's per-video info.json reads are parallelised across the
/// available CPUs because that's where ~all the time goes for large
/// libraries (one fs read + one JSON parse per video, multiplied by hundreds
/// or thousands).
pub fn scan_channels(root: &Path) -> Vec<Channel> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let dirs: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() { return None; }
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') { return None; }
            Some((name, path))
        })
        .collect();

    // Process channels in parallel. We size the worker pool to min(channels, CPUs)
    // so a small library doesn't spin up needless threads.
    let n_workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(dirs.len().max(1));
    let mut channels = parallel_map(dirs, n_workers, |(name, path)| {
        let (videos, playlists) = scan_channel_dir(&path);
        if videos.is_empty() && playlists.is_empty() { return None; }
        let meta = load_channel_meta(&videos);
        let total_videos_cached =
            videos.len() + playlists.iter().map(|p| p.videos.len()).sum::<usize>();
        let total_size_cached = videos
            .iter()
            .chain(playlists.iter().flat_map(|p| p.videos.iter()))
            .filter_map(|v| v.file_size)
            .sum();
        Some(Channel {
            name,
            path,
            videos,
            playlists,
            meta,
            total_videos_cached,
            total_size_cached,
        })
    })
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    channels.sort_by_key(|c| c.name.to_lowercase());
    channels
}

/// Fan an `items` slice across `n_workers` threads, applying `f` to each item
/// and returning results in the original input order.
///
/// Stdlib-only mini work-stealer: an atomic index hands out the next slot to
/// any worker that's free. Used to parallelise channel-directory scans
/// without dragging in rayon.
fn parallel_map<I, O, F>(items: Vec<I>, n_workers: usize, f: F) -> Vec<O>
where
    I: Send + 'static,
    O: Send + 'static + Default,
    F: Fn(I) -> O + Send + Sync + 'static,
{
    let len = items.len();
    if len == 0 { return Vec::new(); }
    if n_workers <= 1 {
        return items.into_iter().map(f).collect();
    }
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    let items: Vec<Mutex<Option<I>>> = items.into_iter().map(|v| Mutex::new(Some(v))).collect();
    let items = Arc::new(items);
    let results: Vec<Mutex<O>> = (0..len).map(|_| Mutex::new(O::default())).collect();
    let results = Arc::new(results);
    let next = Arc::new(AtomicUsize::new(0));
    let f = Arc::new(f);

    let mut handles = Vec::with_capacity(n_workers);
    for _ in 0..n_workers {
        let items = items.clone();
        let results = results.clone();
        let next = next.clone();
        let f = f.clone();
        handles.push(std::thread::spawn(move || {
            loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= len { break; }
                let input = items[i].lock().unwrap().take().unwrap();
                let out = f(input);
                *results[i].lock().unwrap() = out;
            }
        }));
    }
    for h in handles { let _ = h.join(); }

    Arc::try_unwrap(results)
        .unwrap_or_else(|_| unreachable!("workers joined; refs released"))
        .into_iter()
        .map(|m| m.into_inner().unwrap())
        .collect()
}

fn load_channel_meta(videos: &[Video]) -> Option<ChannelMeta> {
    // Pull channel-level fields out of the first video's info.json
    let info_path = videos.iter().find_map(|v| {
        let p = v.video_path.as_ref()?.with_extension("info.json");
        p.exists().then_some(p)
    })?;
    let text = std::fs::read_to_string(&info_path).ok()?;
    let val: serde_json::Value = serde_json::from_str(&text).ok()?;
    Some(ChannelMeta {
        subscriber_count: val.get("channel_follower_count").and_then(|v| v.as_u64()),
        channel_url: val.get("channel_url").and_then(|v| v.as_str()).map(String::from),
        uploader: val
            .get("uploader")
            .or_else(|| val.get("channel"))
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

enum FileKind {
    Video,
    Thumb,
    Description,
    LiveChat,
    Info,
    Other,
}

fn classify(file_name: &str) -> Option<(&str, FileKind)> {
    if let Some(stem) = file_name.strip_suffix(".live_chat.json") {
        return Some((stem, FileKind::LiveChat));
    }
    if let Some(stem) = file_name.strip_suffix(".info.json") {
        return Some((stem, FileKind::Info));
    }
    let dot = file_name.rfind('.')?;
    let stem = &file_name[..dot];
    if stem.is_empty() { return None; }
    let ext = file_name[dot + 1..].to_lowercase();
    let kind = if VIDEO_EXTS.contains(&ext.as_str()) {
        FileKind::Video
    } else if THUMB_EXTS.contains(&ext.as_str()) {
        FileKind::Thumb
    } else if ext == "description" {
        FileKind::Description
    } else {
        FileKind::Other
    };
    Some((stem, kind))
}

fn parse_stem(stem: &str) -> Option<(String, String)> {
    let close = stem.rfind(']')?;
    let open = stem[..close].rfind('[')?;
    let id = stem[open + 1..close].trim();
    if id.is_empty() { return None; }
    let title = stem[..open].trim().trim_end_matches('-').trim();
    let title = if title.is_empty() { stem } else { title };
    Some((title.to_string(), id.to_string()))
}

struct RawVideo {
    id: String,
    title: String,
    stem: String,
    video_path: Option<PathBuf>,
    thumb_path: Option<PathBuf>,
    description_path: Option<PathBuf>,
    info_path: Option<PathBuf>,
    subtitles: Vec<Subtitle>,
    has_live_chat: bool,
}

fn collect_raw_videos(entries: impl Iterator<Item = std::fs::DirEntry>) -> Vec<RawVideo> {
    let mut by_stem: BTreeMap<String, RawVideo> = BTreeMap::new();
    let mut pending_subs: Vec<(String, String, PathBuf)> = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_file() { continue; }
        let file_name = entry.file_name().to_string_lossy().into_owned();

        // Subtitles: "Title [id].en.vtt" or "Title [id].en.srt" — strip ext then .lang
        let sub_stem = file_name.strip_suffix(".vtt")
            .or_else(|| file_name.strip_suffix(".srt"));
        if let Some(sub_stem) = sub_stem {
            if let Some(dot) = sub_stem.rfind('.') {
                let lang = sub_stem[dot + 1..].to_string();
                let video_stem = sub_stem[..dot].to_string();
                pending_subs.push((video_stem, lang, path));
                continue;
            }
        }

        let Some((stem, kind)) = classify(&file_name) else { continue };
        let Some((title, id)) = parse_stem(stem) else { continue };
        let raw = by_stem.entry(stem.to_string()).or_insert_with(|| RawVideo {
            id,
            title,
            stem: stem.to_string(),
            video_path: None,
            thumb_path: None,
            description_path: None,
            info_path: None,
            subtitles: Vec::new(),
            has_live_chat: false,
        });
        match kind {
            FileKind::Video => { if raw.video_path.is_none() { raw.video_path = Some(path); } }
            FileKind::Thumb => { if raw.thumb_path.is_none() { raw.thumb_path = Some(path); } }
            FileKind::Description => raw.description_path = Some(path),
            FileKind::Info => raw.info_path = Some(path),
            FileKind::LiveChat => raw.has_live_chat = true,
            FileKind::Other => {}
        }
    }
    for (video_stem, lang, path) in pending_subs {
        if let Some(raw) = by_stem.get_mut(&video_stem) {
            raw.subtitles.push(Subtitle { lang, path });
        }
    }
    for raw in by_stem.values_mut() {
        raw.subtitles.sort_by(|a, b| a.lang.cmp(&b.lang));
    }
    by_stem.into_values().collect()
}

fn enrich(raws: Vec<RawVideo>) -> Vec<Video> {
    let mut videos: Vec<Video> = raws.into_iter().map(|raw| {
        // Parse info.json once for both duration and chapter presence.
        let (duration_secs, has_chapters, upload_date) = raw.info_path.as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .map(|val| {
                let dur = val.get("duration").and_then(|v| v.as_f64());
                let chap = val.get("chapters")
                    .and_then(|c| c.as_array())
                    .map(|a| !a.is_empty())
                    .unwrap_or(false);
                // Prefer `upload_date`; fall back to `release_date` for premiere/live content.
                let date = val.get("upload_date")
                    .and_then(|v| v.as_str())
                    .or_else(|| val.get("release_date").and_then(|v| v.as_str()))
                    .map(|s| s.to_string());
                (dur, chap, date)
            })
            .unwrap_or((None, false, None));
        let file_size = raw.video_path.as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len());
        Video {
            id: raw.id,
            title: raw.title,
            stem: raw.stem,
            video_path: raw.video_path,
            thumb_path: raw.thumb_path,
            description_path: raw.description_path,
            info_path: raw.info_path,
            subtitles: raw.subtitles,
            has_live_chat: raw.has_live_chat,
            duration_secs,
            has_chapters,
            file_size,
            upload_date,
        }
    }).collect();
    videos.sort_by_key(|v| v.title.to_lowercase());
    videos
}

/// Scan a single flat directory for video files and return enriched `Video` entries.
///
/// Used when rescanning a playlist directory without a full library reload.
pub fn scan_video_files(dir: &Path) -> Vec<Video> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let raws = collect_raw_videos(entries.flatten().filter(|e| e.path().is_file()));
    enrich(raws)
}

fn scan_channel_dir(dir: &Path) -> (Vec<Video>, Vec<Playlist>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return (Vec::new(), Vec::new()) };

    let mut file_entries = Vec::new();
    let mut playlists = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let videos = scan_video_files(&path);
            if !videos.is_empty() {
                playlists.push(Playlist { name, path, videos });
            }
        } else {
            file_entries.push(entry);
        }
    }

    let raws = collect_raw_videos(file_entries.into_iter());
    let videos = enrich(raws);
    playlists.sort_by_key(|p| p.name.to_lowercase());
    (videos, playlists)
}

// ── Music library ─────────────────────────────────────────────────────────────

/// Scan `root` (the `music/` directory) for audio tracks, recursively.
///
/// The top-level subdirectory name is used as the default artist (overridden
/// by info.json's `artist`/`creator`/`uploader` when present). Deeper levels
/// — e.g. `music/Artist/Album/song.opus` — are walked but the top-level name
/// remains the fallback artist so albums don't reset the attribution.
pub fn scan_music(root: &Path) -> Vec<Track> {
    let mut tracks = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return tracks };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') { continue; }
        if path.is_dir() {
            scan_music_dir(&path, &name, &mut tracks);
        } else if let Some(track) = track_from_path(&path, "") {
            tracks.push(track);
        }
    }
    tracks.sort_by_key(|t| (t.artist.to_lowercase(), t.title.to_lowercase()));
    tracks
}

fn scan_music_dir(dir: &Path, folder_artist: &str, tracks: &mut Vec<Track>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') { continue; }
        if path.is_file() {
            if let Some(track) = track_from_path(&path, folder_artist) {
                tracks.push(track);
            }
        } else if path.is_dir() {
            // Recurse into albums/subfolders while preserving the top-level
            // artist label.
            scan_music_dir(&path, folder_artist, tracks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stem_extracts_id_and_title() {
        let (title, id) = parse_stem("My great video [abc123]").unwrap();
        assert_eq!(title, "My great video");
        assert_eq!(id, "abc123");
    }

    #[test]
    fn parse_stem_trims_trailing_dash() {
        let (title, _id) = parse_stem("Some video - [xyz]").unwrap();
        assert_eq!(title, "Some video");
    }

    #[test]
    fn parse_stem_rejects_missing_brackets() {
        assert!(parse_stem("no brackets here").is_none());
    }

    #[test]
    fn parse_stem_rejects_empty_id() {
        assert!(parse_stem("foo []").is_none());
    }

    #[test]
    fn parse_stem_handles_brackets_in_title() {
        // The last [..] is the id; earlier ones are part of the title.
        let (title, id) = parse_stem("[NSFW] Some title [vidid]").unwrap();
        assert_eq!(id, "vidid");
        assert!(title.contains("[NSFW]"));
    }
}

fn track_from_path(path: &Path, folder_artist: &str) -> Option<Track> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    if !AUDIO_EXTS.contains(&ext.as_str()) { return None; }

    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let (title, id) = parse_stem(&stem)?;

    let dir = path.parent()?;
    let thumb_path = THUMB_EXTS.iter().find_map(|e| {
        let p = dir.join(format!("{stem}.{e}"));
        p.exists().then_some(p)
    });
    let info_path = {
        let p = dir.join(format!("{stem}.info.json"));
        p.exists().then_some(p)
    };

    let (duration_secs, resolved_artist) = info_path.as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|val| {
            let dur = val.get("duration").and_then(|v| v.as_f64());
            let art = val.get("artist")
                .or_else(|| val.get("creator"))
                .or_else(|| val.get("uploader"))
                .and_then(|v| v.as_str())
                .map(String::from)
                .unwrap_or_else(|| folder_artist.to_string());
            (dur, art)
        })
        .unwrap_or_else(|| (None, folder_artist.to_string()));

    let artist = if resolved_artist.is_empty() {
        "Unknown".to_string()
    } else {
        resolved_artist
    };

    Some(Track {
        id,
        title,
        artist,
        path: path.to_path_buf(),
        thumb_path,
        info_path,
        duration_secs,
        file_size: std::fs::metadata(path).ok().map(|m| m.len()),
    })
}
