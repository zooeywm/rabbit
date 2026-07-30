use std::str::FromStr;

use directories::ProjectDirs;
use eros::Context;
use serde::Deserialize;

use ::config::{Config as ConfigLoader, Environment, File};

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub logging: LoggingConfig,
}

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct LoggingConfig {
    pub console_level: LogLevel,
    pub file_level: LogLevel,
}

#[derive(Deserialize, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
    Off,
}

impl FromStr for LogLevel {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "trace" => Ok(Self::Trace),
            "debug" => Ok(Self::Debug),
            "info" => Ok(Self::Info),
            "warn" => Ok(Self::Warn),
            "error" => Ok(Self::Error),
            "off" => Ok(Self::Off),
            _ => Err("expected trace, debug, info, warn, error, or off"),
        }
    }
}

impl Config {
    pub fn load(project_dirs: &ProjectDirs) -> eros::Result<Self> {
        let config_path = project_dirs.config_local_dir().join("config.toml");

        Ok(ConfigLoader::builder()
            .add_source(File::from(config_path).required(false))
            .add_source(
                Environment::with_prefix("RABBIT")
                    .prefix_separator("_")
                    .separator("__"),
            )
            .build()
            .with_context(|| "Failed to load config sources")?
            .try_deserialize()
            .with_context(|| "Failed to deserialize config")?)
    }
}
