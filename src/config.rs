use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub backup: BackupSection,
    #[serde(default)]
    pub player: PlayerSection,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupSection {
    pub directory: PathBuf,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct PlayerSection {
    #[serde(default = "default_player")]
    pub command: String,
}

fn default_player() -> String {
    "mpv".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let contents = std::fs::read_to_string(path)?;
        toml::from_str(&contents).map_err(|e| e.into())
    }
}
