use std::path::PathBuf;

use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub summary: SummarySection,
}

#[derive(Debug, Deserialize)]
pub struct SummarySection {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

impl Default for SummarySection {
    fn default() -> Self {
        Self {
            enabled: false,
            command: String::new(),
            max_length: 500,
        }
    }
}

fn default_max_length() -> usize {
    500
}

/// Load config from `~/.config/sannai/config.toml`, falling back to defaults.
pub fn load_config() -> Config {
    let path = config_path();
    if !path.exists() {
        return Config::default();
    }

    match std::fs::read_to_string(&path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!("Failed to parse config at {}: {}", path.display(), e);
                Config::default()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to read config at {}: {}", path.display(), e);
            Config::default()
        }
    }
}

fn config_path() -> PathBuf {
    if let Ok(path) = std::env::var("SANNAI_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("sannai")
        .join("config.toml")
}
