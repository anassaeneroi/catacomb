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
const THUMB_EXTS: &[&str] = &["webp", "jpg", "jpeg", "png"];

/// A single WebVTT subtitle track discovered alongside a video file.
#[derive(Clone, Debug)]
pub struct Subtitle {
    /// ISO 639-1/2 language code extracted from the `.lang.vtt` filename suffix.
    pub lang: String,
    pub path: PathBuf,
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
    /// Size of the video file on disk; `None` if the video file is missing.
    pub file_size: Option<u64>,
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
}

/// Scan `root` for channel directories and return them sorted alphabetically.
///
/// Skips hidden directories (names starting with `.`) and directories that
/// contain no recognisable video files.
pub fn scan_channels(root: &Path) -> Vec<Channel> {
    let mut channels = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return channels };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') { continue; }
        let (videos, playlists) = scan_channel_dir(&path);
        if videos.is_empty() && playlists.is_empty() { continue; }
        let meta = load_channel_meta(&videos);
        let total_videos_cached =
            videos.len() + playlists.iter().map(|p| p.videos.len()).sum::<usize>();
        let total_size_cached = videos
            .iter()
            .chain(playlists.iter().flat_map(|p| p.videos.iter()))
            .filter_map(|v| v.file_size)
            .sum();
        channels.push(Channel {
            name,
            path,
            videos,
            playlists,
            meta,
            total_videos_cached,
            total_size_cached,
        });
    }
    channels.sort_by_key(|c| c.name.to_lowercase());
    channels
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

        // Subtitles have stems like "Title [id].en.vtt" — strip the .vtt and trailing .lang
        if let Some(sub_stem) = file_name.strip_suffix(".vtt") {
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
        let duration_secs = raw.info_path.as_ref().and_then(|p| {
            let text = std::fs::read_to_string(p).ok()?;
            let val: serde_json::Value = serde_json::from_str(&text).ok()?;
            val.get("duration").and_then(|v| v.as_f64())
        });
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
            file_size,
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
