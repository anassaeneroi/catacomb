//! Per-channel download option overrides.
//!
//! Closes the largest gap with Tartube (its `OptionsManager` — 164 fields
//! attachable to any channel/playlist/video with cascading resolution). We
//! ship a focused v1 with ~10 of the most-used fields. The cascade currently
//! has two levels:
//!
//! - **Channel options** — set per `(platform, handle)` pair via the UI and
//!   stored in the `channel_options` SQLite table.
//! - **Global defaults** — what [`crate::downloader::Downloader`] applies
//!   when no channel options exist.
//!
//! Future expansion (Phase 1 follow-up): folder-level options + a named
//! "options manager" pool that channels can reference by id.

use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::downloader::DownloadQuality;

/// Per-channel overrides applied on top of the standard yt-dlp flags
/// emitted by [`crate::downloader::Downloader::start`]. Every field is
/// optional / empty-by-default so an empty `DownloadOptions` is a no-op.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadOptions {
    /// Quality cap for this channel's videos. When `Some`, overrides the
    /// quality picker the user chose at submit time *for re-checks
    /// originating from the channel*. Explicit submits from the download
    /// dialog still use whatever the user selected there.
    #[serde(default)]
    pub quality: Option<DownloadQuality>,

    /// Force audio-only extraction for this channel. Adds
    /// `--extract-audio --audio-format best --audio-quality 0`.
    #[serde(default)]
    pub audio_only: bool,

    /// Bandwidth cap in kilobytes per second. Maps to `--limit-rate <N>K`.
    /// `None` for no limit.
    #[serde(default)]
    pub limit_rate_kb: Option<u32>,

    /// Skip videos smaller than this many megabytes (yt-dlp `--min-filesize`).
    #[serde(default)]
    pub min_filesize_mb: Option<u32>,

    /// Skip videos larger than this many megabytes (yt-dlp `--max-filesize`).
    /// Useful for filtering "music" channels that occasionally drop full
    /// concerts you don't want.
    #[serde(default)]
    pub max_filesize_mb: Option<u32>,

    /// Only download videos uploaded on or after this date (`YYYYMMDD`).
    /// Maps to yt-dlp `--dateafter`.
    #[serde(default)]
    pub date_after: Option<String>,

    /// Free-form yt-dlp `--match-filter` expression. Power-user feature; the
    /// UI surfaces it as a single textbox with a pointer to yt-dlp docs.
    #[serde(default)]
    pub match_filter: Option<String>,

    /// Subtitle languages to fetch. `["en"]` for English-only,
    /// `["en", "ja"]` for English + Japanese, etc. Empty for the global
    /// default (which is auto + manual via `--write-subs --write-auto-subs`).
    #[serde(default)]
    pub subtitle_langs: Vec<String>,

    /// Per-channel subtitle overrides. Each `None` falls back to the
    /// global `[subtitles]` config. Resolved in [`crate::downloader`]'s
    /// subtitle-flag builder, not here, because `apply()` doesn't have
    /// the global config in scope.
    ///
    /// - `subtitles_enabled`: master on/off for this channel.
    /// - `subtitles_auto`: include auto-generated captions.
    /// - `subtitles_embed`: embed into the container.
    /// - `subtitle_format`: convert to this format (empty string = native).
    #[serde(default)]
    pub subtitles_enabled: Option<bool>,
    #[serde(default)]
    pub subtitles_auto: Option<bool>,
    #[serde(default)]
    pub subtitles_embed: Option<bool>,
    #[serde(default)]
    pub subtitle_format: Option<String>,

    /// Per-channel YouTube player-client override (comma-separated, e.g.
    /// "tv,mweb"). `None` defers to the global
    /// `backup.youtube_player_clients`. Resolved in the downloader, not
    /// `apply()`, since the global default lives in config.
    #[serde(default)]
    pub youtube_player_clients: Option<String>,

    /// Per-channel SponsorBlock override: "off" / "mark" / "remove".
    /// `None` defers to the global `backup.sponsorblock_mode`. Resolved in
    /// the downloader.
    #[serde(default)]
    pub sponsorblock_mode: Option<String>,

    /// Per-channel post-download conversion mode override. `None` defers
    /// to the global `[convert]` config; `Some("off")` forces no
    /// conversion for this channel even when the global default is on.
    /// Other values: "remux-mp4" / "h264-mp4" / "audio". CRF / preset /
    /// audio-format always come from the global config.
    #[serde(default)]
    pub convert_mode: Option<String>,

    /// Raw passthrough — every entry is appended as a separate argument to
    /// yt-dlp. Lets users access any flag we haven't exposed yet. Equivalent
    /// to Tartube's `extra_cmd_string`.
    #[serde(default)]
    pub extra_args: Vec<String>,

    /// Per-channel override for fetching video comments into the info.json
    /// sidecar (yt-dlp `--write-comments`). `None` defers to the global
    /// `backup.fetch_comments`; `Some(true)`/`Some(false)` forces it on/off
    /// for this channel. Resolved in the downloader's comment resolver, which
    /// merges this with the global default. When on, the player's Comments
    /// tab is populated (comment download is slow on popular videos).
    #[serde(default)]
    pub fetch_comments: Option<bool>,

    /// Skip yt-dlp's channel-tab authentication sanity check by passing
    /// `--extractor-args youtubetab:skip=authcheck`.
    ///
    /// yt-dlp warns ("Playlists that require authentication may not
    /// extract correctly…") when it can't confirm a channel page loaded
    /// authenticated. For PUBLIC channels that warning is noise — there's
    /// no auth-gated content to miss — so this silences it. Leave OFF for
    /// channels where you archive members-only / private content with
    /// cookies: there the warning is a real "your cookies aren't working,
    /// you may be getting an incomplete list" signal worth keeping.
    #[serde(default)]
    pub skip_auth_check: bool,
}

impl DownloadOptions {
    /// True when every field is at its default. Lets callers cheaply detect
    /// an effectively-blank options row and avoid storing it.
    pub fn is_empty(&self) -> bool {
        self == &DownloadOptions::default()
    }

    /// Append this channel's option overrides to a yt-dlp `Command`.
    /// Called by [`crate::downloader::Downloader::start`] after the standard
    /// flag set, so an explicit override here wins.
    pub fn apply(&self, cmd: &mut Command) {
        if self.audio_only {
            // Mirror what `start_music` does: extract the best audio in its
            // native format. Don't force a re-encode — that'd lose quality.
            cmd.arg("--extract-audio")
                .arg("--audio-format").arg("best")
                .arg("--audio-quality").arg("0");
        }
        if let Some(rate) = self.limit_rate_kb {
            cmd.arg("--limit-rate").arg(format!("{rate}K"));
        }
        if let Some(min) = self.min_filesize_mb {
            cmd.arg("--min-filesize").arg(format!("{min}M"));
        }
        if let Some(max) = self.max_filesize_mb {
            cmd.arg("--max-filesize").arg(format!("{max}M"));
        }
        if let Some(date) = &self.date_after {
            if !date.is_empty() {
                cmd.arg("--dateafter").arg(date);
            }
        }
        if let Some(filter) = &self.match_filter {
            if !filter.is_empty() {
                cmd.arg("--match-filter").arg(filter);
            }
        }
        // Subtitle flags (langs / write / auto / embed / convert) are
        // emitted by the downloader's subtitle resolver, which merges
        // these per-channel overrides with the global [subtitles] config.
        // apply() handles everything else.
        for arg in &self.extra_args {
            if !arg.is_empty() {
                cmd.arg(arg);
            }
        }
        // Comment fetching (--write-comments) is emitted by the downloader's
        // comment resolver, which merges this per-channel override with the
        // global backup.fetch_comments default. apply() handles the rest.
        if self.skip_auth_check {
            // Its own --extractor-args flag; the youtubetab: namespace is
            // distinct from the POT provider's youtubepot-bgutilhttp: one,
            // so they coexist as separate flags without clobbering.
            cmd.arg("--extractor-args").arg("youtubetab:skip=authcheck");
        }
    }

    /// Deserialize from the JSON blob stored in `channel_options.options_json`.
    /// Bad rows are logged and treated as "no options" so a corrupt or
    /// schema-drifted row doesn't take a channel offline.
    pub fn from_json(s: &str) -> Self {
        match serde_json::from_str::<DownloadOptions>(s) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("channel_options: ignoring malformed row: {e}");
                DownloadOptions::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_options_emit_nothing() {
        let mut cmd = Command::new("yt-dlp");
        DownloadOptions::default().apply(&mut cmd);
        // The Command has just the program name, no extra args.
        let args: Vec<_> = cmd.get_args().collect();
        assert!(args.is_empty());
    }

    #[test]
    fn limit_rate_emits_correct_flag() {
        let mut cmd = Command::new("yt-dlp");
        DownloadOptions {
            limit_rate_kb: Some(500),
            ..Default::default()
        }
        .apply(&mut cmd);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["--limit-rate", "500K"]);
    }

    // Subtitle-flag emission moved to the downloader's resolver (it merges
    // these per-channel langs with the global [subtitles] config). The
    // resolver's behavior is tested in downloader.rs.

    #[test]
    fn skip_auth_check_emits_extractor_arg() {
        let mut cmd = Command::new("yt-dlp");
        DownloadOptions { skip_auth_check: true, ..Default::default() }.apply(&mut cmd);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["--extractor-args", "youtubetab:skip=authcheck"]);
    }

    #[test]
    fn audio_only_adds_extract_audio_chain() {
        let mut cmd = Command::new("yt-dlp");
        DownloadOptions {
            audio_only: true,
            ..Default::default()
        }
        .apply(&mut cmd);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|w| w == ["--extract-audio".to_string(), "--audio-format".to_string()]));
    }

    #[test]
    fn extra_args_pass_through_verbatim() {
        let mut cmd = Command::new("yt-dlp");
        DownloadOptions {
            extra_args: vec!["--no-cache-dir".into(), "--verbose".into()],
            ..Default::default()
        }
        .apply(&mut cmd);
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args, vec!["--no-cache-dir", "--verbose"]);
    }

    #[test]
    fn json_roundtrip() {
        let original = DownloadOptions {
            quality: Some(DownloadQuality::Res1080),
            audio_only: false,
            limit_rate_kb: Some(1024),
            subtitle_langs: vec!["en".into()],
            extra_args: vec!["--no-mtime".into()],
            ..Default::default()
        };
        let json = serde_json::to_string(&original).unwrap();
        let roundtrip = DownloadOptions::from_json(&json);
        assert_eq!(original, roundtrip);
    }

    #[test]
    fn corrupt_json_falls_back_to_default() {
        let parsed = DownloadOptions::from_json("not even json {");
        assert_eq!(parsed, DownloadOptions::default());
    }

    #[test]
    fn is_empty_detects_default() {
        assert!(DownloadOptions::default().is_empty());
        assert!(!DownloadOptions {
            audio_only: true,
            ..Default::default()
        }
        .is_empty());
    }
}
