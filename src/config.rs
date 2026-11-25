use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::discord::DiscordConfig;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// "receiver" or "cable".
    pub bank: String,
    /// "en" or "pl".
    pub language: String,
    pub auto_refresh: bool,
    pub refresh_secs: u64,
    pub discord: DiscordConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            bank: "receiver".to_owned(),
            language: "en".to_owned(),
            auto_refresh: true,
            refresh_secs: 30,
            discord: DiscordConfig::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(appdata) = std::env::var_os("APPDATA") {
        return PathBuf::from(appdata).join("SmxHowler");
    }
    std::env::temp_dir().join("SmxHowler")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load() -> AppConfig {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<AppConfig>(&text) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("Ignoring invalid config {}: {err}", path.display());
                AppConfig::default()
            }
        },
        Err(_) => AppConfig::default(),
    }
}

pub fn save(config: &AppConfig) -> Result<(), String> {
    let dir = config_dir();
    fs::create_dir_all(&dir).map_err(|err| format!("Could not create {}: {err}", dir.display()))?;
    let path = config_path();
    let text = serde_json::to_string_pretty(config)
        .map_err(|err| format!("Could not encode the configuration: {err}"))?;
    fs::write(&path, text).map_err(|err| format!("Could not write {}: {err}", path.display()))
}
