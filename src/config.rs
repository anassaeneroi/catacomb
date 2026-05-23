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
}

fn default_max_concurrent() -> usize { 3 }

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
}

impl Default for UiSection {
    fn default() -> Self {
        Self { theme: default_theme() }
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
            },
            player: PlayerSection::default(),
            ui: UiSection::default(),
            scheduler: SchedulerSection::default(),
            web: WebSection::default(),
            plex: PlexSection::default(),
        }
    }
}
