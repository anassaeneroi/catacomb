//! Scanning the `channels/` directory tree into channels, playlists, and videos.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const VIDEO_EXTS: &[&str] = &["mkv", "mp4", "webm", "m4v", "mov", "avi"];
const THUMB_EXTS: &[&str] = &["webp", "jpg", "jpeg", "png"];

#[derive(Clone, Debug)]
pub struct Video {
    pub id: String,
    pub title: String,
    #[allow(dead_code)]
    pub stem: String,
    pub video_path: Option<PathBuf>,
    pub thumb_path: Option<PathBuf>,
    pub description_path: Option<PathBuf>,
    pub has_live_chat: bool,
    pub duration_secs: Option<f64>,
    pub file_size: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub name: String,
    #[allow(dead_code)]
    pub path: PathBuf,
    pub videos: Vec<Video>,
}

#[derive(Clone, Debug)]
pub struct ChannelMeta {
    pub subscriber_count: Option<u64>,
    pub channel_url: Option<String>,
    pub uploader: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub path: PathBuf,
    pub videos: Vec<Video>,
    pub playlists: Vec<Playlist>,
    pub meta: Option<ChannelMeta>,
}

impl Channel {
    pub fn total_videos(&self) -> usize {
        self.videos.len() + self.playlists.iter().map(|p| p.videos.len()).sum::<usize>()
    }
}

pub fn scan_channels(root: &Path) -> Vec<Channel> {
    let mut channels = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else { return channels };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() { continue; }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (videos, playlists) = scan_channel_dir(&path);
        let meta = load_channel_meta(&videos);
        channels.push(Channel { name, path, videos, playlists, meta });
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
    has_live_chat: bool,
}

fn collect_raw_videos(entries: impl Iterator<Item = std::fs::DirEntry>) -> Vec<RawVideo> {
    let mut by_stem: BTreeMap<String, RawVideo> = BTreeMap::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_file() { continue; }
        let file_name = entry.file_name().to_string_lossy().into_owned();
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
            has_live_chat: raw.has_live_chat,
            duration_secs,
            file_size,
        }
    }).collect();
    videos.sort_by_key(|v| v.title.to_lowercase());
    videos
}

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
