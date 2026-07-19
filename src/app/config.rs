use std::{fs, path::Path};

use directories::ProjectDirs;
use eros::Context;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "rabbit";
const CONFIG_FILE_NAME: &str = "config.toml";

/// Rabbit configuration.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip, default = "default_project_dirs")]
    pub project_dirs: ProjectDirs,

    #[serde(skip, default = "default_app_name")]
    pub app_name: &'static str,

    pub logging: LoggingConfig,
    pub video: VideoConfig,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub console_level: LogLevel,
    pub file_level: LogLevel,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct VideoConfig {
    pub enable_probing: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            console_level: LogLevel::Debug,
            file_level: LogLevel::Info,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            logging: LoggingConfig::default(),
            video: VideoConfig::default(),
            project_dirs: default_project_dirs(),
            app_name: APP_NAME,
        }
    }
}

impl Config {
    pub fn new() -> eros::Result<Self> {
        let project_dirs =
            ProjectDirs::from("", "", APP_NAME).context("Failed looking for app project dir")?;

        let config_dir = project_dirs.config_local_dir();
        let mut config = Self::load_or_create(config_dir)?;

        config.project_dirs = project_dirs;
        config.app_name = APP_NAME;

        Ok(config)
    }

    fn load_or_create(config_dir: &Path) -> eros::Result<Self> {
        let config_file_path = config_dir.join(CONFIG_FILE_NAME);

        if config_file_path.exists() {
            let content = fs::read_to_string(&config_file_path)?;
            return Ok(toml::from_str(&content)?);
        }

        let config = Self::default();

        fs::create_dir_all(config_dir)?;
        let content = toml::to_string_pretty(&config)?;
        fs::write(&config_file_path, content)?;

        Ok(config)
    }
}

fn default_project_dirs() -> ProjectDirs {
    ProjectDirs::from("", "", APP_NAME)
        .expect("the current platform does not provide a valid configuration directory")
}

const fn default_app_name() -> &'static str {
    APP_NAME
}

#[cfg(test)]
mod tests {
    use crate::app::config::Config;

    #[test]
    fn video_probing_is_disabled_by_default() {
        assert!(!Config::default().video.enable_probing);
    }

    #[test]
    fn video_probing_can_be_enabled_from_config() {
        let config = toml::from_str::<Config>("[video]\nenable_probing = true")
            .expect("Video probing configuration should deserialize");

        assert!(config.video.enable_probing);
    }
}
