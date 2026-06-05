//! Application configuration loaded from `config.toml`.
//!
//! Each top-level section maps to a TOML table.  Missing sections get sane
//! defaults so existing config files continue to work after upgrades.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Root configuration object, serialised from/to `config.toml`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub backup: BackupSection,
    #[serde(default)]
    pub player: PlayerSection,
    #[serde(default)]
    pub ui: UiSection,
    #[serde(default)]
    pub scheduler: SchedulerSection,
    #[serde(default)]
    pub web: WebSection,
    #[serde(default)]
    pub plex: PlexSection,
    #[serde(default)]
    pub subtitles: SubtitlesSection,
    #[serde(default)]
    pub convert: ConvertSection,
}

/// `[convert]` table — global post-download format-conversion defaults.
/// A separate ffmpeg pass runs after the download completes. Per-channel
/// [`crate::download_options::DownloadOptions`] can override `mode`.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ConvertSection {
    /// What to do after download. One of:
    /// - `""` / `"off"`: nothing (keep the downloaded mkv as-is).
    /// - `"remux-mp4"`: fast container change mkv→mp4, no re-encode.
    /// - `"h264-mp4"`: re-encode video to H.264/AAC mp4 at [`Self::crf`].
    /// - `"audio"`: extract audio to [`Self::audio_format`].
    #[serde(default)]
    pub mode: String,
    /// CRF for `h264-mp4` (0–51; lower = bigger/better). 23 is a sane
    /// default if left at 0.
    #[serde(default)]
    pub crf: u8,
    /// x264 preset for `h264-mp4` (ultrafast…veryslow). Empty = "medium".
    #[serde(default)]
    pub preset: String,
    /// Audio format for `audio` mode (mp3 / m4a / opus / flac). Empty =
    /// "mp3".
    #[serde(default)]
    pub audio_format: String,
    /// Keep the original downloaded file alongside the converted one
    /// (renamed to `<stem>.original.<ext>`). When false, the original is
    /// deleted after a successful convert.
    #[serde(default)]
    pub keep_original: bool,
}

/// `[subtitles]` table — global subtitle download defaults. Per-channel
/// [`crate::download_options::DownloadOptions`] can override any of these.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtitlesSection {
    /// Master switch. When false, no subtitle flags are emitted at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Include auto-generated (machine) captions via `--write-auto-subs`.
    /// Off-by-default-ish content is noisy; keep it on globally but let
    /// channels turn it off.
    #[serde(default = "default_true")]
    pub auto_generated: bool,
    /// Embed subtitle tracks into the video container (`--embed-subs`),
    /// in addition to writing sidecar files. Soft subs, toggleable in
    /// the player.
    #[serde(default)]
    pub embed: bool,
    /// Convert downloaded subtitles to this format (`--convert-subs`).
    /// Empty = keep native. Common: "srt" (Plex-friendly), "vtt", "ass".
    #[serde(default)]
    pub format: String,
    /// Comma-separated language codes (`--sub-langs`). Empty = all
    /// available. e.g. "en", "en,ja", "en.*" for all English variants.
    #[serde(default)]
    pub langs: String,
}

impl Default for SubtitlesSection {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_generated: true,
            embed: false,
            format: String::new(),
            langs: String::new(),
        }
    }
}

/// `[backup]` table — where to store downloaded videos.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupSection {
    pub directory: PathBuf,
    /// Maximum simultaneous yt-dlp processes. Extra downloads queue and start
    /// automatically when a slot opens. Set to 0 for no limit (not recommended).
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    /// If true, use the bundled yt-dlp + deno binaries managed by yt-offline
    /// (installed under `~/.local/share/yt-offline/bin/`). If false, use the
    /// `yt-dlp` found on the system PATH.
    #[serde(default)]
    pub use_bundled_ytdlp: bool,
    /// If true and the bundled yt-dlp is in use, spawn the bgutil-pot
    /// HTTP server on startup and pass its extractor-args to every
    /// yt-dlp invocation. YouTube increasingly requires a per-video
    /// Proof-of-Origin token; without one, format URLs come back empty.
    ///
    /// Only effective in tandem with `use_bundled_ytdlp` because the
    /// matching Python plugin gets pip-installed into the bundled venv,
    /// not the system Python. System-yt-dlp users who want POT support
    /// install the plugin and run the server themselves.
    #[serde(default)]
    pub use_pot_provider: bool,
    /// YouTube player clients to try, as a comma-separated list passed to
    /// `--extractor-args youtube:player_client=…`. Empty = let yt-dlp pick
    /// its defaults (recommended). YouTube's bot-detection cracks down on
    /// different clients over time; setting e.g. `tv,mweb` routes around a
    /// client that's currently being captcha-walled. Per-channel overrides
    /// live in [`crate::download_options::DownloadOptions`].
    #[serde(default)]
    pub youtube_player_clients: String,
}

fn default_max_concurrent() -> usize { 3 }
fn default_true() -> bool { true }

/// `[player]` table — external player and browser cookie source.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PlayerSection {
    #[serde(default = "default_player")]
    pub command: String,
    #[serde(default = "default_browser")]
    pub browser: String,
}

impl Default for PlayerSection {
    fn default() -> Self {
        Self { command: default_player(), browser: default_browser() }
    }
}

/// `[ui]` table — egui desktop theme.
///
/// Available themes: `dark`, `light`, `dracula`, `trans`, `emo-nocturnal`, `emo-coffin`, `emo-scene-queen`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UiSection {
    #[serde(default = "default_theme")]
    pub theme: String,
    /// When true, clicking the window close button hides the window
    /// into the system tray instead of exiting. The tray menu's Quit
    /// item (or Ctrl+Q) still terminates the app. Off by default so
    /// users without a working SNI host don't get stuck in an invisible
    /// hidden window.
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// Global egui zoom factor — scales the *entire* desktop UI (fonts,
    /// spacing, widgets), not just the card grid. 1.0 = native. Persisted
    /// here so the chosen size survives restarts and is kept in sync with
    /// the built-in Ctrl + / Ctrl - / Ctrl 0 keyboard zoom.
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
}

fn default_ui_scale() -> f32 { 1.0 }

impl Default for UiSection {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            minimize_to_tray: false,
            ui_scale: default_ui_scale(),
        }
    }
}

/// `[scheduler]` table — periodic background download schedule.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SchedulerSection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_interval_hours")]
    pub interval_hours: u32,
}

impl Default for SchedulerSection {
    fn default() -> Self {
        Self { enabled: false, interval_hours: default_interval_hours() }
    }
}

/// `[plex]` table — Plex-compatible TV-show symlink library.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PlexSection {
    /// Directory where the Plex symlink tree is written.
    /// Leave unset to disable Plex library generation.
    pub library_path: Option<PathBuf>,
}

/// `[web]` table — built-in HTTP server settings.
///
/// `source_url` is **required for AGPL §13 compliance**: set it to a URL
/// where the running source code can be obtained (e.g. your Codeberg repo).
/// It is shown as a "Source" link in the web UI footer.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WebSection {
    #[serde(default = "default_web_port")]
    pub port: u16,
    #[serde(default = "default_web_bind")]
    /// Address to bind the HTTP server to. Defaults to `127.0.0.1` (localhost only).
    /// Set to `0.0.0.0` to accept connections from any interface (not recommended).
    pub bind: String,
    #[serde(default)]
    pub transcode: bool,
    /// Public URL to the source repository, shown in the web UI per AGPL §13.
    pub source_url: Option<String>,
}

impl Default for WebSection {
    fn default() -> Self {
        Self {
            port: default_web_port(),
            bind: default_web_bind(),
            transcode: false,
            source_url: None,
        }
    }
}

fn default_player() -> String { "mpv".to_string() }
fn default_browser() -> String { "firefox".to_string() }
fn default_theme() -> String { "dark".to_string() }
fn default_interval_hours() -> u32 { 24 }
fn default_web_port() -> u16 { 8080 }
fn default_web_bind() -> String { "127.0.0.1".to_string() }

impl Config {
    /// Load and parse `config.toml` from `path`.
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(|e| e.into())
    }

    /// Serialise the config back to `path` in pretty TOML.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    /// Construct a minimal default config pointing `backup.directory` at `dir`.
    pub fn default_with_dir(dir: PathBuf) -> Self {
        Self {
            backup: BackupSection {
                directory: dir,
                max_concurrent: default_max_concurrent(),
                use_bundled_ytdlp: false,
                use_pot_provider: false,
                youtube_player_clients: String::new(),
            },
            player: PlayerSection::default(),
            ui: UiSection::default(),
            scheduler: SchedulerSection::default(),
            web: WebSection::default(),
            plex: PlexSection::default(),
            subtitles: SubtitlesSection::default(),
            convert: ConvertSection::default(),
        }
    }
}
