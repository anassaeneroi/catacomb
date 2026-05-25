//! Plex-compatible TV-show library generator.
//!
//! Creates a directory tree of symlinks under a configurable Plex library root.
//! Each channel becomes a show; videos are arranged as episodes numbered by
//! upload date, grouped into year-based seasons.
//!
//! Per-episode metadata (title, plot, aired date, runtime, thumbnail) is
//! written as Kodi-format `.nfo` sidecars that Plex's "Personal Media (TV
//! Shows)" agent (or the XBMC NFO agent) reads. A show-level `tvshow.nfo`
//! carries the channel title and uploader.
//!
//! # Output structure
//!
//! ```text
//! <plex_root>/
//!   Channel Name/
//!     .plexmatch               ← legacy show-title hint
//!     tvshow.nfo               ← channel-level metadata
//!     Season 2023/
//!       Channel Name - S2023E001 - Video Title.mkv      → symlink to real file
//!       Channel Name - S2023E001 - Video Title.nfo      ← episode metadata
//!       Channel Name - S2023E001 - Video Title-thumb.jpg → symlink to thumb
//!       Channel Name - S2023E001 - Video Title.en.srt
//!     Season 2024/
//!       …
//! ```
//!
//! Existing symlinks are left untouched; the function is safe to re-run after
//! new downloads. NFO files are rewritten on each run so updated metadata
//! (e.g. corrected titles) propagates.

use std::collections::BTreeMap;
use std::path::Path;

use crate::library::{Channel, Video};

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Replace characters that are invalid in filenames on common filesystems.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Extract the four-digit year from a `YYYYMMDD` upload_date, or 0 if unavailable.
fn year_of(date: &str) -> u32 {
    if date.len() >= 4 { date[..4].parse().unwrap_or(0) } else { 0 }
}

/// Convert `YYYYMMDD` → `YYYY-MM-DD` for NFO `<aired>` tags. Returns empty string
/// for malformed input.
fn aired_date(date: &str) -> String {
    if date.len() >= 8 {
        format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..8])
    } else {
        String::new()
    }
}

/// Minimal XML escaping for text-node content in NFO files.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Read the channel-level description and uploader from the first available
/// video's info.json for use in `tvshow.nfo`.
fn channel_meta_from_info(videos: &[&Video]) -> (String, String) {
    for v in videos {
        let Some(p) = v.info_path.as_ref() else { continue };
        let Ok(text) = std::fs::read_to_string(p) else { continue };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let plot = val.get("channel_description")
            .or_else(|| val.get("uploader_description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let uploader = val.get("uploader")
            .or_else(|| val.get("channel"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        return (plot, uploader);
    }
    (String::new(), String::new())
}

/// Read a video's full description text. Prefers the `.description` sidecar
/// (full text including newlines); falls back to the `description` field of
/// info.json if the sidecar is missing.
fn read_description(v: &Video) -> String {
    if let Some(p) = v.description_path.as_ref() {
        if let Ok(s) = std::fs::read_to_string(p) {
            return s;
        }
    }
    v.info_path.as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|val| val.get("description").and_then(|v| v.as_str()).map(String::from))
        .unwrap_or_default()
}

/// Create a symlink at `link` pointing to `target`.  Skips if the link already
/// exists (including broken symlinks).  Returns an error only on creation failure.
fn make_symlink(target: &Path, link: &Path) -> Result<(), String> {
    // symlink_metadata succeeds even for broken symlinks, unlike exists()
    if link.symlink_metadata().is_ok() {
        return Ok(());
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| format!("{}: {e}", link.display()))?;
    #[cfg(not(unix))]
    return Err("symlinks are not supported on this platform".to_string());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_invalid_chars() {
        assert_eq!(sanitize("a/b:c*d?e\"f<g>h|i\\j"), "a-b-c-d-e-f-g-h-i-j");
    }

    #[test]
    fn year_of_extracts_year() {
        assert_eq!(year_of("20240315"), 2024);
        assert_eq!(year_of(""), 0);
        assert_eq!(year_of("bad"), 0);
    }

    #[test]
    fn aired_date_formats_yyyymmdd() {
        assert_eq!(aired_date("20240315"), "2024-03-15");
        assert_eq!(aired_date("bad"), "");
    }

    #[test]
    fn xml_escape_escapes_special_chars() {
        assert_eq!(xml_escape("a & b < c > d"), "a &amp; b &lt; c &gt; d");
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Result of a Plex library generation run.
pub struct GenerateResult {
    pub links_created: usize,
    pub errors: Vec<String>,
}

/// Generate (or refresh) the Plex symlink tree under `plex_root` from `channels`.
///
/// Safe to call repeatedly: existing symlinks are skipped, missing ones are added.
pub fn generate(channels: &[Channel], plex_root: &Path) -> GenerateResult {
    let mut links_created = 0;
    let mut errors = Vec::new();

    for ch in channels {
        let all_videos: Vec<&Video> = ch.videos.iter()
            .chain(ch.playlists.iter().flat_map(|p| p.videos.iter()))
            .collect();

        if all_videos.is_empty() { continue; }

        // Prefix the show folder with the platform name when it's not YouTube
        // so multi-platform libraries don't collide on shared creator names
        // (e.g. a YouTube and a TikTok account both called "MusicianName").
        let base_name = sanitize(&ch.name);
        let show_name = if ch.platform == crate::platform::Platform::YouTube {
            base_name
        } else {
            format!("{} - {}", ch.platform.display_name(), base_name)
        };
        let show_dir = plex_root.join(&show_name);

        if let Err(e) = std::fs::create_dir_all(&show_dir) {
            errors.push(format!("mkdir {}: {e}", show_dir.display()));
            continue;
        }

        // .plexmatch: helps Plex identify the show by title
        let _ = std::fs::write(
            show_dir.join(".plexmatch"),
            format!("title: {}\n", ch.name),
        );

        // tvshow.nfo: channel-level metadata (title, uploader, plot).
        let (channel_plot, uploader) = channel_meta_from_info(&all_videos);
        let tvshow_nfo = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <tvshow>\n  \
               <title>{}</title>\n  \
               <studio>{}</studio>\n  \
               <plot>{}</plot>\n\
             </tvshow>\n",
            xml_escape(&ch.name),
            xml_escape(&uploader),
            xml_escape(&channel_plot),
        );
        let _ = std::fs::write(show_dir.join("tvshow.nfo"), tvshow_nfo);

        // Sort all videos by upload date for deterministic episode numbering
        let mut dated: Vec<(&Video, String)> = all_videos.iter()
            .map(|v| (*v, v.upload_date.clone().unwrap_or_default()))
            .collect();
        dated.sort_by(|a, b| a.1.cmp(&b.1));

        // Group into year-based seasons
        let mut by_year: BTreeMap<u32, Vec<&Video>> = BTreeMap::new();
        for (v, date) in &dated {
            by_year.entry(year_of(date)).or_default().push(v);
        }

        for (year, vids) in &by_year {
            let season_label = if *year > 0 {
                format!("Season {year}")
            } else {
                "Season 01".to_string()
            };
            let season_num = if *year > 0 { *year } else { 1 };
            let season_dir = show_dir.join(&season_label);

            if let Err(e) = std::fs::create_dir_all(&season_dir) {
                errors.push(format!("mkdir {}: {e}", season_dir.display()));
                continue;
            }

            for (ep_idx, v) in vids.iter().enumerate() {
                let ep = ep_idx as u32 + 1;
                let title = sanitize(&v.title);
                let stem = format!("{show_name} - S{season_num:04}E{ep:03} - {title}");

                // Video file symlink
                if let Some(ref src) = v.video_path {
                    let ext = src.extension().and_then(|e| e.to_str()).unwrap_or("mkv");
                    let link = season_dir.join(format!("{stem}.{ext}"));
                    match make_symlink(src, &link) {
                        Ok(()) => links_created += 1,
                        Err(e) => errors.push(e),
                    }
                }

                // Thumbnail symlink — Plex looks for `<stem>-thumb.jpg`.
                if let Some(ref thumb) = v.thumb_path {
                    let ext = thumb.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
                    let link = season_dir.join(format!("{stem}-thumb.{ext}"));
                    match make_symlink(thumb, &link) {
                        Ok(()) => links_created += 1,
                        Err(e) => errors.push(e),
                    }
                }

                // Episode NFO sidecar — overwritten on every run so metadata
                // updates (renames, re-fetched info.json) take effect.
                let aired = v.upload_date.as_deref()
                    .map(aired_date)
                    .unwrap_or_default();
                let runtime_min = v.duration_secs
                    .map(|s| (s / 60.0).round() as u64)
                    .unwrap_or(0);
                let plot = read_description(v);
                let nfo = format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                     <episodedetails>\n  \
                       <title>{}</title>\n  \
                       <season>{}</season>\n  \
                       <episode>{}</episode>\n  \
                       <aired>{}</aired>\n  \
                       <runtime>{}</runtime>\n  \
                       <plot>{}</plot>\n\
                     </episodedetails>\n",
                    xml_escape(&v.title),
                    season_num,
                    ep,
                    xml_escape(&aired),
                    runtime_min,
                    xml_escape(&plot),
                );
                let nfo_path = season_dir.join(format!("{stem}.nfo"));
                if let Err(e) = std::fs::write(&nfo_path, nfo) {
                    errors.push(format!("write {}: {e}", nfo_path.display()));
                }

                // Subtitle symlinks — Plex picks these up automatically when
                // the stem matches: "Show - S01E01 - Title.en.srt"
                for sub in &v.subtitles {
                    let ext = sub.path.extension().and_then(|e| e.to_str()).unwrap_or("srt");
                    let link = season_dir.join(format!("{stem}.{}.{ext}", sub.lang));
                    match make_symlink(&sub.path, &link) {
                        Ok(()) => links_created += 1,
                        Err(e) => errors.push(e),
                    }
                }
            }
        }
    }

    GenerateResult { links_created, errors }
}
