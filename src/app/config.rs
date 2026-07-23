use std::{fs, path::Path};

use directories::ProjectDirs;
use eros::Context;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "rabbit";
pub const APP_ID: &str = "io.github.zooeywm.rabbit";
const CONFIG_FILE_NAME: &str = "config.toml";

/// Rabbit configuration.
#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip)]
    pub project_dirs: Option<ProjectDirs>,

    #[serde(skip, default = "default_app_name")]
    pub app_name: &'static str,

    pub logging: LoggingConfig,
    pub network: NetworkConfig,
    pub video: VideoConfig,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkConfig {
    pub transport: NetworkTransport,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            transport: NetworkTransport::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NetworkTransport {
    #[default]
    Quic,
    Tcp,
}

impl NetworkTransport {
    pub(crate) const fn listener_protocol(self) -> &'static str {
        match self {
            Self::Quic => "UDP",
            Self::Tcp => "TCP",
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    pub console_level: LogLevel,
    pub file_level: LogLevel,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct VideoConfig {
    pub enable_host_probing: bool,
    pub enable_client_probing: bool,
    pub probe_interval_ms: u64,
    pub display_backend: VideoDisplayPreference,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            enable_host_probing: false,
            enable_client_probing: false,
            probe_interval_ms: 2_000,
            display_backend: VideoDisplayPreference::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VideoDisplayPreference {
    #[default]
    Auto,
    Wayland,
    Slint,
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
            network: NetworkConfig::default(),
            video: VideoConfig::default(),
            project_dirs: None,
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

        config.project_dirs = Some(project_dirs);
        config.app_name = APP_NAME;

        Ok(config)
    }

    fn load_or_create(config_dir: &Path) -> eros::Result<Self> {
        let config_file_path = config_dir.join(CONFIG_FILE_NAME);

        if config_file_path.exists() {
            let content = fs::read_to_string(&config_file_path).with_context(|| {
                format!(
                    "Failed to read configuration from {}",
                    config_file_path.display()
                )
            })?;
            return Ok(toml::from_str(&content).with_context(|| {
                format!(
                    "Failed to parse configuration from {}",
                    config_file_path.display()
                )
            })?);
        }

        let config = Self::default();

        fs::create_dir_all(config_dir).with_context(|| {
            format!(
                "Failed to create configuration directory {}",
                config_dir.display()
            )
        })?;
        let content = toml::to_string_pretty(&config)
            .with_context(|| "Failed to encode the default configuration")?;
        fs::write(&config_file_path, content).with_context(|| {
            format!(
                "Failed to write default configuration to {}",
                config_file_path.display()
            )
        })?;

        Ok(config)
    }
}

const fn default_app_name() -> &'static str {
    APP_NAME
}

#[cfg(test)]
mod tests {
    use crate::app::config::{Config, NetworkTransport, VideoDisplayPreference};

    #[test]
    fn network_transport_defaults_to_quic() {
        assert_eq!(Config::default().network.transport, NetworkTransport::Quic);
    }

    #[test]
    fn network_transport_can_be_configured_as_tcp() {
        let config = toml::from_str::<Config>("[network]\ntransport = \"tcp\"")
            .expect("TCP network transport configuration should deserialize");

        assert_eq!(config.network.transport, NetworkTransport::Tcp);
    }

    #[test]
    fn network_transport_reports_its_listener_protocol() {
        assert_eq!(NetworkTransport::Quic.listener_protocol(), "UDP");
        assert_eq!(NetworkTransport::Tcp.listener_protocol(), "TCP");
    }

    #[test]
    fn host_and_client_video_probing_are_disabled_by_default() {
        let video = Config::default().video;

        assert!(!video.enable_host_probing);
        assert!(!video.enable_client_probing);
    }

    #[test]
    fn host_and_client_video_probing_can_be_configured_independently() {
        for (host, client) in [(true, false), (false, true)] {
            let config = toml::from_str::<Config>(&format!(
                "[video]\nenable_host_probing = {host}\nenable_client_probing = {client}"
            ))
            .expect("Video probing configuration should deserialize");

            assert_eq!(config.video.enable_host_probing, host);
            assert_eq!(config.video.enable_client_probing, client);
        }
    }

    #[test]
    fn video_probe_interval_defaults_to_two_seconds() {
        assert_eq!(Config::default().video.probe_interval_ms, 2_000);
    }

    #[test]
    fn video_probe_interval_can_be_configured_in_milliseconds() {
        let config = toml::from_str::<Config>("[video]\nprobe_interval_ms = 750")
            .expect("Video probe interval configuration should deserialize");

        assert_eq!(config.video.probe_interval_ms, 750);
    }

    #[test]
    fn video_display_backend_defaults_to_auto() {
        assert_eq!(
            Config::default().video.display_backend,
            VideoDisplayPreference::Auto
        );
    }

    #[test]
    fn video_display_backend_can_be_selected_from_config() {
        for (configured, expected) in [
            ("auto", VideoDisplayPreference::Auto),
            ("wayland", VideoDisplayPreference::Wayland),
            ("slint", VideoDisplayPreference::Slint),
        ] {
            let config =
                toml::from_str::<Config>(&format!("[video]\ndisplay_backend = \"{configured}\""))
                    .expect("Video display backend configuration should deserialize");

            assert_eq!(config.video.display_backend, expected);
        }
    }
}

// Focused test: cargo test app::config::tests:: --lib
