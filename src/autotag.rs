//! Smart auto-tagging — heuristic grouping suggestions for the library.
//!
//! Looks at each *unfiled* channel's source platform and the duration
//! distribution of its videos and proposes a folder group: "Music",
//! "Shorts", "Long-form & Podcasts", or "Streams & VODs". Suggestions are
//! advisory only — the user applies them from the Maintenance view, which
//! creates the folder (if it doesn't exist yet) and assigns the channels via
//! the existing folder machinery. Channels already in a folder are left
//! untouched. Roadmap 3.4.
//!
//! Everything here is pure arithmetic over already-scanned metadata
//! (`Channel`/`Video`), so it's cheap enough to recompute on demand without a
//! background job.

use serde::Serialize;

use crate::library::Channel;
use crate::platform::Platform;

/// A single channel's suggested placement.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ChannelSuggestion {
    /// Platform dir-name (`channels`/`tiktok`/…) — half of the assignment key.
    pub platform: String,
    pub platform_label: String,
    /// Channel handle (its folder name) — the other half of the key.
    pub handle: String,
    /// Friendly name for display (uploader if known, else the handle).
    pub display_name: String,
    /// Human-readable justification, e.g. "median length 42 s; ~15/mo".
    pub reason: String,
    /// 0.0–1.0 signal strength; drives UI emphasis (strong vs tentative).
    pub confidence: f32,
}

/// A proposed folder and the channels that look like they belong in it.
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GroupSuggestion {
    /// Suggested folder name (created on apply if absent).
    pub group: String,
    pub channels: Vec<ChannelSuggestion>,
}

// Duration thresholds (seconds).
const SHORTS_MAX: f64 = 90.0;
const LONGFORM_MIN: f64 = 25.0 * 60.0;
// A channel needs at least this many videos before we trust the signal.
const MIN_VIDEOS: usize = 3;

/// Compute grouping suggestions for every unfiled channel that has enough
/// videos to form a signal. Channels already assigned to a folder are skipped.
pub fn suggest(channels: &[Channel]) -> Vec<GroupSuggestion> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<&'static str, Vec<ChannelSuggestion>> = BTreeMap::new();

    for ch in channels {
        if ch.folder_id.is_some() || ch.total_videos() < MIN_VIDEOS {
            continue;
        }
        let durations: Vec<f64> = ch
            .all_videos()
            .filter_map(|v| v.duration_secs)
            .filter(|d| *d > 0.0)
            .collect();

        let (Some(group), confidence) = classify(ch, &durations) else {
            continue;
        };

        let mut reason = group_reason(ch, &durations);
        if let Some(per_month) = cadence_per_month(ch) {
            reason.push_str(&format!("; ~{per_month}/mo"));
        }

        groups.entry(group).or_default().push(ChannelSuggestion {
            platform: ch.platform.dir_name().to_string(),
            platform_label: ch.platform.display_name().to_string(),
            handle: ch.name.clone(),
            display_name: ch
                .meta
                .as_ref()
                .and_then(|m| m.uploader.clone())
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| ch.name.clone()),
            reason,
            confidence,
        });
    }

    groups
        .into_iter()
        .map(|(group, mut channels)| {
            // Strongest signal first within each group.
            channels.sort_by(|a, b| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            GroupSuggestion {
                group: group.to_string(),
                channels,
            }
        })
        .collect()
}

/// Choose a group + confidence for a channel, or `(None, _)` when no signal is
/// strong enough to suggest anything (the common mid-length YouTube case).
fn classify(ch: &Channel, durations: &[f64]) -> (Option<&'static str>, f32) {
    match ch.platform {
        Platform::Bandcamp | Platform::SoundCloud => return (Some("Music"), 0.95),
        Platform::Twitch => return (Some("Streams & VODs"), 0.9),
        Platform::TikTok => return (Some("Shorts"), 0.9),
        _ => {}
    }
    // The user already downloads this channel as audio → treat as music.
    if ch.download_options.audio_only {
        return (Some("Music"), 0.85);
    }
    match median(durations) {
        Some(m) if m < SHORTS_MAX => (Some("Shorts"), 0.7),
        Some(m) if m >= LONGFORM_MIN => (Some("Long-form & Podcasts"), 0.7),
        _ => (None, 0.0),
    }
}

fn group_reason(ch: &Channel, durations: &[f64]) -> String {
    match ch.platform {
        Platform::Bandcamp | Platform::SoundCloud => return "music platform".to_string(),
        Platform::Twitch => return "Twitch channel".to_string(),
        Platform::TikTok => return "TikTok channel".to_string(),
        _ => {}
    }
    if ch.download_options.audio_only {
        return "downloaded as audio-only".to_string();
    }
    match median(durations) {
        Some(m) => format!("median length {}", fmt_dur(m)),
        None => "no duration data".to_string(),
    }
}

fn median(durations: &[f64]) -> Option<f64> {
    if durations.is_empty() {
        return None;
    }
    let mut v = durations.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    Some(if v.len() % 2 == 0 {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    })
}

fn fmt_dur(secs: f64) -> String {
    let s = secs.round() as u64;
    if s < 90 {
        format!("{s} s")
    } else {
        format!("{} min", (s + 30) / 60)
    }
}

/// Rough videos-per-month from the spread of upload dates. `None` if fewer than
/// two videos carry a parseable `YYYYMMDD`.
fn cadence_per_month(ch: &Channel) -> Option<u64> {
    let mut dates: Vec<u32> = ch
        .all_videos()
        .filter_map(|v| v.upload_date.as_deref())
        .filter_map(|d| d.parse::<u32>().ok())
        .filter(|d| *d > 0)
        .collect();
    if dates.len() < 2 {
        return None;
    }
    dates.sort_unstable();
    let months = month_span(*dates.first()?, *dates.last()?).max(1);
    Some((dates.len() as u64 / months).max(1))
}

/// Whole-month span between two `YYYYMMDD` integers (clamped at 0).
fn month_span(a: u32, b: u32) -> u64 {
    let (ay, am) = ((a / 10000) as i64, ((a / 100) % 100) as i64);
    let (by, bm) = ((b / 10000) as i64, ((b / 100) % 100) as i64);
    ((by - ay) * 12 + (bm - am)).max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Channel, Video};

    fn vid(dur: f64, date: &str) -> Video {
        Video {
            id: "x".into(),
            title: "t".into(),
            stem: "t".into(),
            video_path: None,
            thumb_path: None,
            description_path: None,
            info_path: None,
            subtitles: Vec::new(),
            has_live_chat: false,
            duration_secs: Some(dur),
            has_chapters: false,
            file_size: None,
            mtime_unix: None,
            upload_date: Some(date.into()),
        }
    }

    fn channel(platform: Platform, durations: &[f64]) -> Channel {
        let videos: Vec<Video> = durations.iter().map(|d| vid(*d, "20240101")).collect();
        let total = videos.len();
        Channel {
            name: "chan".into(),
            path: std::path::PathBuf::from("/tmp/chan"),
            platform,
            source_url: None,
            videos,
            playlists: Vec::new(),
            meta: None,
            total_videos_cached: total,
            total_size_cached: 0,
            download_options: Default::default(),
            folder_id: None,
        }
    }

    #[test]
    fn shorts_detected_by_median_duration() {
        let groups = suggest(&[channel(Platform::YouTube, &[30.0, 45.0, 20.0])]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].group, "Shorts");
        assert_eq!(groups[0].channels.len(), 1);
    }

    #[test]
    fn longform_detected_by_median_duration() {
        let groups = suggest(&[channel(Platform::YouTube, &[3600.0, 2700.0, 4000.0])]);
        assert_eq!(groups[0].group, "Long-form & Podcasts");
    }

    #[test]
    fn music_platform_grouped_regardless_of_duration() {
        let groups = suggest(&[channel(Platform::Bandcamp, &[200.0, 240.0, 180.0])]);
        assert_eq!(groups[0].group, "Music");
    }

    #[test]
    fn mid_length_youtube_yields_no_suggestion() {
        // ~8 min median: ambiguous, deliberately not suggested.
        let groups = suggest(&[channel(Platform::YouTube, &[480.0, 500.0, 460.0])]);
        assert!(groups.is_empty());
    }

    #[test]
    fn filed_or_tiny_channels_are_skipped() {
        let mut filed = channel(Platform::YouTube, &[30.0, 30.0, 30.0]);
        filed.folder_id = Some(7);
        let tiny = channel(Platform::YouTube, &[30.0]); // < MIN_VIDEOS
        assert!(suggest(&[filed, tiny]).is_empty());
    }
}
