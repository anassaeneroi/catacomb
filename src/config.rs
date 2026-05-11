use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub backup: BackupSection,
    #[serde(default)]
    pub player: PlayerSection,
    #[serde(default)]
    pub ui: UiSection,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupSection {
    pub directory: PathBuf,
}

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

fn default_player() -> String { "mpv".to_string() }
fn default_browser() -> String { "firefox".to_string() }
fn default_theme() -> String { "dark".to_string() }

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(|e| e.into())
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)?;
        Ok(())
    }

    pub fn default_with_dir(dir: PathBuf) -> Self {
        Self {
            backup: BackupSection { directory: dir },
            player: PlayerSection::default(),
            ui: UiSection::default(),
        }
    }
}
