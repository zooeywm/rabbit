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
    pub recording: RecordingConfig,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct NetworkConfig {
    pub transport: NetworkTransport,
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

/// Local screen recording path (`rabbit record`).
///
/// Screen and duration are CLI options; only the file/directory path lives here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingConfig {
    /// Output file path, or a directory (timestamped `.mp4` is created inside).
    /// Empty defaults to the standard Videos directory under `rabbit/`.
    pub output_path: String,
}

impl Default for RecordingConfig {
    fn default() -> Self {
        Self {
            output_path: String::new(),
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
            console_level: LogLevel::Info,
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
            recording: RecordingConfig::default(),
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

    /// Resolves the recording output file path from config.
    ///
    /// Empty `output_path` → standard Videos dir / `rabbit` / `rabbit-<timestamp>.mp4`.
    pub fn resolve_recording_output_path(&self) -> eros::Result<std::path::PathBuf> {
        use std::path::PathBuf;

        let configured = self.recording.output_path.trim();
        if configured.is_empty() {
            let base = default_videos_rabbit_dir()?;
            fs::create_dir_all(&base).with_context(|| {
                format!("Failed to create recording directory {}", base.display())
            })?;
            return Ok(base.join(default_recording_file_name()));
        }

        let expanded = expand_user_path(configured);
        let path = PathBuf::from(expanded);
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "mp4" | "m4v" | "mov"))
        {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create recording parent directory {}",
                        parent.display()
                    )
                })?;
            }
            return Ok(path);
        }

        fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create recording directory {}", path.display()))?;
        Ok(path.join(default_recording_file_name()))
    }
}

/// Standard user Videos directory + `rabbit` (e.g. `~/Videos/rabbit`).
pub fn default_videos_rabbit_dir() -> eros::Result<std::path::PathBuf> {
    use directories::UserDirs;
    use std::path::PathBuf;

    if let Some(user_dirs) = UserDirs::new()
        && let Some(videos) = user_dirs.video_dir()
    {
        return Ok(videos.join(APP_NAME));
    }

    // Fallback when XDG video dir is unavailable.
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home).join("Videos").join(APP_NAME));
    }

    Ok(PathBuf::from("Videos").join(APP_NAME))
}

fn default_recording_file_name() -> String {
    let now = time::OffsetDateTime::now_local().unwrap_or_else(|_| time::OffsetDateTime::now_utc());
    let format = time::macros::format_description!("[year][month][day]-[hour][minute][second]");
    let stamp = now.format(&format).unwrap_or_else(|_| "recording".into());
    format!("rabbit-{stamp}.mp4")
}

fn expand_user_path(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}{}{rest}", std::path::MAIN_SEPARATOR);
    }
    if path == "~"
        && let Ok(home) = std::env::var("HOME")
    {
        return home;
    }
    path.to_owned()
}

const fn default_app_name() -> &'static str {
    APP_NAME
}

#[cfg(test)]
mod tests {
    use crate::app::config::{Config, NetworkTransport, RecordingConfig, VideoDisplayPreference};

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

    #[test]
    fn recording_config_only_has_output_path() {
        let defaults = Config::default().recording;
        assert!(defaults.output_path.is_empty());

        let config = toml::from_str::<Config>(
            r#"
[recording]
output_path = "~/Videos/rabbit-out.mp4"
"#,
        )
        .expect("recording config should deserialize");
        assert_eq!(config.recording.output_path, "~/Videos/rabbit-out.mp4");
        let _ = RecordingConfig::default();
    }

    #[test]
    fn resolve_recording_output_path_uses_file_or_directory() {
        let dir = std::env::temp_dir().join(format!("rabbit-record-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        let file = dir.join("clip.mp4");
        let mut config = Config::default();
        config.recording.output_path = file.to_string_lossy().into_owned();
        let resolved = config.resolve_recording_output_path().expect("file path");
        assert_eq!(resolved, file);

        config.recording.output_path = dir.to_string_lossy().into_owned();
        let resolved = config
            .resolve_recording_output_path()
            .expect("directory path");
        assert_eq!(resolved.parent(), Some(dir.as_path()));
        assert!(
            resolved
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("rabbit-") && name.ends_with(".mp4"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_recording_dir_is_under_videos_rabbit() {
        let dir = crate::app::config::default_videos_rabbit_dir().expect("videos dir");
        assert!(
            dir.ends_with("rabbit") || dir.components().any(|c| c.as_os_str() == "rabbit"),
            "expected .../rabbit, got {}",
            dir.display()
        );
        let parent = dir.file_name().and_then(|n| n.to_str());
        assert_eq!(parent, Some("rabbit"));
    }
}

// Focused test: cargo test app::config::tests:: --lib
