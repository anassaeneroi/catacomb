//! Source-platform classification.
//!
//! yt-dlp can pull from ~1,800 sites; this module organises the supported
//! ones into a small enum with three concerns:
//!
//! 1. **URL → platform**: pattern-match host to a [`Platform`] variant.
//! 2. **Platform → directory**: each platform owns a sibling folder next to
//!    `channels/` (which remains YouTube for backward compatibility).
//! 3. **Folder → display**: icon and human label for the UI.
//!
//! Anything yt-dlp accepts that doesn't match a known host lands in
//! [`Platform::Other`] and goes into `other/<creator>/`.
//!
//! Per-creator metadata (the original source URL, so re-checks don't have to
//! guess from a folder name) is written to a `.source-url` sidecar at the
//! root of each creator folder. The library scanner picks it up.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a download came from. Drives output paths and sidebar grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    YouTube,
    TikTok,
    Twitch,
    Vimeo,
    Bandcamp,
    SoundCloud,
    Odysee,
    Other,
}

impl Default for Platform {
    fn default() -> Self { Self::YouTube }
}

impl Platform {
    /// Folder name used under the backup root. YouTube keeps the legacy
    /// `channels/` for backward compatibility with existing libraries.
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::YouTube    => "channels",
            Self::TikTok     => "tiktok",
            Self::Twitch     => "twitch",
            Self::Vimeo      => "vimeo",
            Self::Bandcamp   => "bandcamp",
            Self::SoundCloud => "soundcloud",
            Self::Odysee     => "odysee",
            Self::Other      => "other",
        }
    }

    /// Human-readable label for the UI (sidebar / tooltips).
    pub fn display_name(self) -> &'static str {
        match self {
            Self::YouTube    => "YouTube",
            Self::TikTok     => "TikTok",
            Self::Twitch     => "Twitch",
            Self::Vimeo      => "Vimeo",
            Self::Bandcamp   => "Bandcamp",
            Self::SoundCloud => "SoundCloud",
            Self::Odysee     => "Odysee",
            Self::Other      => "Other",
        }
    }

    /// Single-character icon shown in the sidebar. Falls back to a generic
    /// camera glyph for `Other` so unknown sources still get a visual hook.
    pub fn icon(self) -> &'static str {
        match self {
            Self::YouTube    => "▶",
            Self::TikTok     => "♪",
            Self::Twitch     => "📺",
            Self::Vimeo      => "🎬",
            Self::Bandcamp   => "🎵",
            Self::SoundCloud => "☁",
            Self::Odysee     => "◈",
            Self::Other      => "📼",
        }
    }

    /// Inverse of [`Self::dir_name`]. Available for callers that need to
    /// recover the platform from a stored folder name.
    #[allow(dead_code)]
    pub fn from_dir_name(name: &str) -> Option<Self> {
        Some(match name {
            "channels"   => Self::YouTube,
            "tiktok"     => Self::TikTok,
            "twitch"     => Self::Twitch,
            "vimeo"      => Self::Vimeo,
            "bandcamp"   => Self::Bandcamp,
            "soundcloud" => Self::SoundCloud,
            "odysee"     => Self::Odysee,
            "other"      => Self::Other,
            _ => return None,
        })
    }

    /// Pick the platform from a URL's host portion. Substring matching is
    /// intentionally loose so subdomains (`m.youtube.com`, `*.bandcamp.com`)
    /// still resolve correctly.
    pub fn from_url(url: &str) -> Self {
        let s = url.to_lowercase();
        if s.contains("youtube.com") || s.contains("youtu.be") || s.contains("youtube-nocookie.com") {
            Self::YouTube
        } else if s.contains("tiktok.com") || s.contains("vm.tiktok.com") {
            Self::TikTok
        } else if s.contains("twitch.tv") {
            Self::Twitch
        } else if s.contains("vimeo.com") {
            Self::Vimeo
        } else if s.contains(".bandcamp.com") || s.contains("//bandcamp.com") {
            Self::Bandcamp
        } else if s.contains("soundcloud.com") {
            Self::SoundCloud
        } else if s.contains("odysee.com") {
            Self::Odysee
        } else {
            Self::Other
        }
    }

    /// Iterate over every variant. Used by the sidebar UI and the scanner.
    pub fn all() -> &'static [Platform] {
        &[
            Platform::YouTube,
            Platform::TikTok,
            Platform::Twitch,
            Platform::Vimeo,
            Platform::Bandcamp,
            Platform::SoundCloud,
            Platform::Odysee,
            Platform::Other,
        ]
    }

    /// Whether downloads from this platform are audio-only by default. The
    /// download dialog can use this to flip the Music toggle automatically.
    #[allow(dead_code)]
    pub fn is_audio_first(self) -> bool {
        matches!(self, Self::Bandcamp | Self::SoundCloud)
    }

    /// yt-dlp `--impersonate` target tuned for each source. Returns `None`
    /// when impersonation should be skipped entirely (e.g. Twitch's OAuth
    /// flow can object to mismatched TLS fingerprints).
    ///
    /// Targets follow yt-dlp's format. TikTok matches the patterns it
    /// expects from its first-party mobile app so a desktop fingerprint
    /// doesn't trip its bot scoring; everything else gets a recent desktop
    /// Chrome.
    pub fn impersonate_target(self) -> Option<&'static str> {
        match self {
            // Twitch's auth surface dislikes TLS-fingerprint impersonation;
            // omit the flag so curl_cffi doesn't break OAuth/Helix calls.
            Self::Twitch => None,
            // TikTok's API is mobile-first — pretend to be Chrome on Android.
            Self::TikTok => Some("Chrome-Android-131"),
            // Default: recent desktop Chrome on macOS.
            _ => Some("Chrome-146:Macos-26"),
        }
    }
}

/// What kind of URL we're looking at — drives the yt-dlp `-o` template.
///
/// Reused across all platforms; the platform comes from [`Platform::from_url`].
#[derive(Debug, Clone)]
pub enum UrlKind {
    /// A creator profile / channel URL. `handle` becomes the on-disk folder.
    Channel { handle: String },
    /// A playlist / album / collection.
    Playlist,
    /// A single video / track / VOD.
    Video,
    /// We can't tell from the URL alone — let yt-dlp figure it out.
    Unknown,
}

/// Fully classified URL: where it's from and what it points at.
#[derive(Debug, Clone)]
pub struct UrlInfo {
    pub platform: Platform,
    pub kind: UrlKind,
}

/// Classify any URL the user pastes. Returns a platform + URL kind.
///
/// Detection is best-effort: enough to pick an output folder and a label.
/// yt-dlp handles the actual extraction, so we don't need precise matching.
pub fn classify_url(url: &str) -> UrlInfo {
    let platform = Platform::from_url(url);
    let kind = match platform {
        Platform::YouTube    => classify_youtube(url),
        Platform::TikTok     => classify_tiktok(url),
        Platform::Twitch     => classify_twitch(url),
        Platform::Vimeo      => classify_vimeo(url),
        Platform::Bandcamp   => classify_bandcamp(url),
        Platform::SoundCloud => classify_soundcloud(url),
        Platform::Odysee     => classify_odysee(url),
        Platform::Other      => UrlKind::Unknown,
    };
    UrlInfo { platform, kind }
}

// ── Per-platform URL → UrlKind ────────────────────────────────────────────────

fn classify_youtube(url: &str) -> UrlKind {
    if url.contains("playlist?list=") { return UrlKind::Playlist; }
    if let Some(h) = extract_after(url, "/@") { return UrlKind::Channel { handle: h.to_string() }; }
    if let Some(h) = extract_after(url, "/channel/") { return UrlKind::Channel { handle: h.to_string() }; }
    if let Some(h) = extract_after(url, "/c/") { return UrlKind::Channel { handle: h.to_string() }; }
    if url.contains("watch?v=") || url.contains("youtu.be/") { return UrlKind::Video; }
    UrlKind::Unknown
}

fn classify_tiktok(url: &str) -> UrlKind {
    // tiktok.com/@user/video/<id> → Video (specific clip)
    // tiktok.com/@user             → Channel (whole profile)
    if let Some(rest) = url.split("/@").nth(1) {
        let handle = rest.split('/').next().unwrap_or("");
        if handle.is_empty() { return UrlKind::Unknown; }
        if rest.contains("/video/") { return UrlKind::Video; }
        return UrlKind::Channel { handle: handle.to_string() };
    }
    if url.contains("vm.tiktok.com") { return UrlKind::Video; }
    UrlKind::Unknown
}

fn classify_twitch(url: &str) -> UrlKind {
    // twitch.tv/<user>           → Channel (bare profile)
    // twitch.tv/<user>/clips     → Channel (clips-only listing)
    // twitch.tv/<user>/videos    → Channel (VOD listing)
    // twitch.tv/videos/<id>      → Video (VOD)
    // twitch.tv/<user>/clip/<id> → Video (single clip)
    if let Some(rest) = url.split("twitch.tv/").nth(1) {
        let first = rest.split('/').next().unwrap_or("").trim_end_matches('?');
        if first.is_empty() { return UrlKind::Unknown; }
        if first == "videos" { return UrlKind::Video; }
        if rest.contains("/clip/") || rest.contains("/video/") {
            return UrlKind::Video;
        }
        let nested = rest.trim_start_matches(first).trim_start_matches('/');
        let nested = nested.split('?').next().unwrap_or("");
        // Bare channel + channel-scoped listing pages all collapse to Channel.
        // yt-dlp's extractors recognise each variant and pull the right
        // subset (clips/highlights/uploads/all) without further hints.
        if nested.is_empty()
            || nested == "videos"
            || nested == "about"
            || nested == "clips"
        {
            return UrlKind::Channel { handle: first.to_string() };
        }
    }
    UrlKind::Unknown
}

fn classify_vimeo(url: &str) -> UrlKind {
    // vimeo.com/<numeric_id>     → Video
    // vimeo.com/user<id>         → Channel
    // vimeo.com/channels/<name>  → Channel
    // vimeo.com/<word>           → Channel (profile)
    if let Some(rest) = url.split("vimeo.com/").nth(1) {
        let first = rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("");
        if first.is_empty() { return UrlKind::Unknown; }
        if first == "channels" {
            if let Some(name) = rest.split('/').nth(1) {
                if !name.is_empty() {
                    return UrlKind::Channel { handle: name.to_string() };
                }
            }
            return UrlKind::Unknown;
        }
        // All digits → single video.
        if first.chars().all(|c| c.is_ascii_digit()) {
            return UrlKind::Video;
        }
        return UrlKind::Channel { handle: first.to_string() };
    }
    UrlKind::Unknown
}

fn classify_bandcamp(url: &str) -> UrlKind {
    // <artist>.bandcamp.com           → Channel (whole discography)
    // <artist>.bandcamp.com/track/<x> → Video (track)
    // <artist>.bandcamp.com/album/<x> → Playlist (album)
    if let Some(host_start) = url.find("://") {
        let host_and_path = &url[host_start + 3..];
        if let Some(host_end) = host_and_path.find('/') {
            let host = &host_and_path[..host_end];
            let path = &host_and_path[host_end..];
            if let Some(artist) = host.strip_suffix(".bandcamp.com") {
                if path.starts_with("/track/") { return UrlKind::Video; }
                if path.starts_with("/album/") { return UrlKind::Playlist; }
                return UrlKind::Channel { handle: artist.to_string() };
            }
        } else if let Some(artist) = host_and_path.strip_suffix(".bandcamp.com") {
            return UrlKind::Channel { handle: artist.to_string() };
        }
    }
    UrlKind::Unknown
}

fn classify_soundcloud(url: &str) -> UrlKind {
    // soundcloud.com/<user>                  → Channel
    // soundcloud.com/<user>/<track>          → Video
    // soundcloud.com/<user>/sets/<playlist>  → Playlist
    if let Some(rest) = url.split("soundcloud.com/").nth(1) {
        let first = rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("");
        if first.is_empty() { return UrlKind::Unknown; }
        if rest.contains("/sets/") { return UrlKind::Playlist; }
        let tail = rest.trim_start_matches(first).trim_start_matches('/');
        let tail = tail.split(|c| c == '?' || c == '#').next().unwrap_or("");
        if tail.is_empty() {
            return UrlKind::Channel { handle: first.to_string() };
        }
        UrlKind::Video
    } else {
        UrlKind::Unknown
    }
}

fn classify_odysee(url: &str) -> UrlKind {
    // odysee.com/@channel:n         → Channel
    // odysee.com/@channel:n/title:m → Video
    if let Some(rest) = url.split("/@").nth(1) {
        let head = rest.split('/').next().unwrap_or("");
        if head.is_empty() { return UrlKind::Unknown; }
        // Strip ":<claimid>" so the on-disk folder is just the channel handle.
        let handle = head.split(':').next().unwrap_or(head).to_string();
        let tail = rest.trim_start_matches(head).trim_start_matches('/');
        let tail = tail.split(|c| c == '?' || c == '#').next().unwrap_or("");
        if tail.is_empty() {
            return UrlKind::Channel { handle };
        }
        UrlKind::Video
    } else {
        UrlKind::Unknown
    }
}

fn extract_after<'a>(url: &'a str, marker: &str) -> Option<&'a str> {
    let start = url.find(marker)? + marker.len();
    let rest = &url[start..];
    let end = rest.find(|c| c == '/' || c == '?' || c == '&' || c == '#').unwrap_or(rest.len());
    if end == 0 { None } else { Some(&rest[..end]) }
}

// ── Filesystem layout ─────────────────────────────────────────────────────────

/// Absolute path to a given platform's video folder, derived from the
/// configured YouTube `channels_root` (its parent is treated as the
/// implicit library root, with each platform as a sibling folder).
pub fn platform_root(channels_root: &Path, platform: Platform) -> PathBuf {
    if platform == Platform::YouTube {
        // YouTube keeps the legacy path verbatim.
        return channels_root.to_path_buf();
    }
    channels_root.with_file_name(platform.dir_name())
}

/// Where the `.source-url` sidecar lives for a creator folder.
pub fn source_url_path(creator_dir: &Path) -> PathBuf {
    creator_dir.join(".source-url")
}

/// Persist the originating URL alongside the downloaded files so a future
/// re-check can recover it without heuristic URL rebuilding.
pub fn write_source_url(creator_dir: &Path, url: &str) {
    let _ = std::fs::write(source_url_path(creator_dir), url.trim());
}

/// Read back a previously-written `.source-url`. Returns `None` if the file
/// is missing or unreadable.
pub fn read_source_url(creator_dir: &Path) -> Option<String> {
    let s = std::fs::read_to_string(source_url_path(creator_dir)).ok()?;
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_urls_route_to_youtube() {
        assert_eq!(Platform::from_url("https://www.youtube.com/@x"), Platform::YouTube);
        assert_eq!(Platform::from_url("https://youtu.be/abc"), Platform::YouTube);
        assert_eq!(Platform::from_url("https://m.youtube.com/watch?v=x"), Platform::YouTube);
    }

    #[test]
    fn other_platforms_route_correctly() {
        assert_eq!(Platform::from_url("https://www.tiktok.com/@u"), Platform::TikTok);
        assert_eq!(Platform::from_url("https://www.twitch.tv/x"), Platform::Twitch);
        assert_eq!(Platform::from_url("https://vimeo.com/12345"), Platform::Vimeo);
        assert_eq!(Platform::from_url("https://artist.bandcamp.com/album/x"), Platform::Bandcamp);
        assert_eq!(Platform::from_url("https://soundcloud.com/x"), Platform::SoundCloud);
        assert_eq!(Platform::from_url("https://odysee.com/@x"), Platform::Odysee);
        assert_eq!(Platform::from_url("https://example.com/video.mp4"), Platform::Other);
    }

    #[test]
    fn tiktok_handle_extracted() {
        let info = classify_url("https://www.tiktok.com/@coolperson");
        assert_eq!(info.platform, Platform::TikTok);
        match info.kind {
            UrlKind::Channel { handle } => assert_eq!(handle, "coolperson"),
            _ => panic!("expected Channel"),
        }
    }

    #[test]
    fn tiktok_video_url_classified_as_video() {
        let info = classify_url("https://www.tiktok.com/@coolperson/video/7123456789");
        assert!(matches!(info.kind, UrlKind::Video));
    }

    #[test]
    fn twitch_channel_url() {
        let info = classify_url("https://www.twitch.tv/streamername");
        assert!(matches!(info.kind, UrlKind::Channel { .. }));
    }

    #[test]
    fn twitch_vod_url_is_video() {
        let info = classify_url("https://www.twitch.tv/videos/123456789");
        assert!(matches!(info.kind, UrlKind::Video));
    }

    #[test]
    fn twitch_clips_listing_is_channel() {
        let info = classify_url("https://www.twitch.tv/streamername/clips");
        match info.kind {
            UrlKind::Channel { handle } => assert_eq!(handle, "streamername"),
            _ => panic!("expected Channel with clips listing"),
        }
    }

    #[test]
    fn vimeo_numeric_is_video() {
        let info = classify_url("https://vimeo.com/123456");
        assert!(matches!(info.kind, UrlKind::Video));
    }

    #[test]
    fn vimeo_user_is_channel() {
        let info = classify_url("https://vimeo.com/staffpicks");
        assert!(matches!(info.kind, UrlKind::Channel { .. }));
    }

    #[test]
    fn bandcamp_subdomain_is_channel() {
        let info = classify_url("https://amazingartist.bandcamp.com/");
        assert!(matches!(info.kind, UrlKind::Channel { .. }));
    }

    #[test]
    fn bandcamp_album_is_playlist() {
        let info = classify_url("https://amazingartist.bandcamp.com/album/my-album");
        assert!(matches!(info.kind, UrlKind::Playlist));
    }

    #[test]
    fn soundcloud_user_is_channel() {
        let info = classify_url("https://soundcloud.com/awesomeartist");
        assert!(matches!(info.kind, UrlKind::Channel { .. }));
    }

    #[test]
    fn soundcloud_set_is_playlist() {
        let info = classify_url("https://soundcloud.com/awesomeartist/sets/myset");
        assert!(matches!(info.kind, UrlKind::Playlist));
    }

    #[test]
    fn odysee_handle_strips_claim_id() {
        let info = classify_url("https://odysee.com/@somebody:abc");
        match info.kind {
            UrlKind::Channel { handle } => assert_eq!(handle, "somebody"),
            _ => panic!("expected Channel"),
        }
    }

    #[test]
    fn platform_root_youtube_is_channels_root() {
        let cr = Path::new("/foo/channels");
        assert_eq!(platform_root(cr, Platform::YouTube), cr);
    }

    #[test]
    fn platform_root_others_are_siblings() {
        let cr = Path::new("/foo/channels");
        assert_eq!(platform_root(cr, Platform::TikTok), Path::new("/foo/tiktok"));
        assert_eq!(platform_root(cr, Platform::Twitch), Path::new("/foo/twitch"));
    }

    #[test]
    fn twitch_skips_impersonation() {
        // TLS fingerprint impersonation can interfere with Twitch's OAuth/Helix.
        assert!(Platform::Twitch.impersonate_target().is_none());
    }

    #[test]
    fn tiktok_uses_mobile_profile() {
        let t = Platform::TikTok.impersonate_target().unwrap();
        assert!(t.to_lowercase().contains("android"));
    }

    #[test]
    fn youtube_uses_desktop_chrome() {
        let t = Platform::YouTube.impersonate_target().unwrap();
        assert!(t.starts_with("Chrome-"));
    }
}
