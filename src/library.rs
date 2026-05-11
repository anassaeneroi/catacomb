//! Scanning the `channels/` directory tree into channels, playlists, and videos.
//!
//! yt-dlp's default output template produces files named `Title [VIDEOID].ext`,
//! so every file that belongs to one video shares the stem `Title [VIDEOID]`.
//! We group files by that stem.

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
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub name: String,
    pub path: PathBuf,
    pub videos: Vec<Video>,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub path: PathBuf,
    pub videos: Vec<Video>,
    pub playlists: Vec<Playlist>,
}

impl Channel {
    pub fn total_videos(&self) -> usize {
        self.videos.len() + self.playlists.iter().map(|p| p.videos.len()).sum::<usize>()
    }
}

pub fn scan_channels(root: &Path) -> Vec<Channel> {
    let mut channels = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return channels;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let (videos, playlists) = scan_channel_dir(&path);
        channels.push(Channel { name, path, videos, playlists });
    }
    channels.sort_by_key(|c| c.name.to_lowercase());
    channels
}

enum FileKind {
    Video,
    Thumb,
    Description,
    LiveChat,
    Other,
}

fn classify(file_name: &str) -> Option<(&str, FileKind)> {
    if let Some(stem) = file_name.strip_suffix(".live_chat.json") {
        return Some((stem, FileKind::LiveChat));
    }
    if let Some(stem) = file_name.strip_suffix(".info.json") {
        return Some((stem, FileKind::Other));
    }
    let dot = file_name.rfind('.')?;
    let stem = &file_name[..dot];
    if stem.is_empty() {
        return None;
    }
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
    if id.is_empty() {
        return None;
    }
    let title = stem[..open].trim().trim_end_matches('-').trim();
    let title = if title.is_empty() { stem } else { title };
    Some((title.to_string(), id.to_string()))
}

pub fn scan_video_files(dir: &Path) -> Vec<Video> {
    let mut by_stem: BTreeMap<String, Video> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, kind)) = classify(&file_name) else {
            continue;
        };
        let Some((title, id)) = parse_stem(stem) else {
            continue;
        };
        let video = by_stem.entry(stem.to_string()).or_insert_with(|| Video {
            id,
            title,
            stem: stem.to_string(),
            video_path: None,
            thumb_path: None,
            description_path: None,
            has_live_chat: false,
        });
        match kind {
            FileKind::Video => {
                if video.video_path.is_none() {
                    video.video_path = Some(path);
                }
            }
            FileKind::Thumb => {
                if video.thumb_path.is_none() {
                    video.thumb_path = Some(path);
                }
            }
            FileKind::Description => video.description_path = Some(path),
            FileKind::LiveChat => video.has_live_chat = true,
            FileKind::Other => {}
        }
    }
    let mut videos: Vec<Video> = by_stem.into_values().collect();
    videos.sort_by_key(|v| v.title.to_lowercase());
    videos
}

fn scan_channel_dir(dir: &Path) -> (Vec<Video>, Vec<Playlist>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return (Vec::new(), Vec::new());
    };

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

    let mut by_stem: BTreeMap<String, Video> = BTreeMap::new();
    for entry in file_entries {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some((stem, kind)) = classify(&file_name) else {
            continue;
        };
        let Some((title, id)) = parse_stem(stem) else {
            continue;
        };
        let video = by_stem.entry(stem.to_string()).or_insert_with(|| Video {
            id,
            title,
            stem: stem.to_string(),
            video_path: None,
            thumb_path: None,
            description_path: None,
            has_live_chat: false,
        });
        match kind {
            FileKind::Video => {
                if video.video_path.is_none() {
                    video.video_path = Some(path);
                }
            }
            FileKind::Thumb => {
                if video.thumb_path.is_none() {
                    video.thumb_path = Some(path);
                }
            }
            FileKind::Description => video.description_path = Some(path),
            FileKind::LiveChat => video.has_live_chat = true,
            FileKind::Other => {}
        }
    }

    let mut videos: Vec<Video> = by_stem.into_values().collect();
    videos.sort_by_key(|v| v.title.to_lowercase());
    playlists.sort_by_key(|p| p.name.to_lowercase());
    (videos, playlists)
}
