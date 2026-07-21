use crate::app::config::VideoDisplayPreference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoDisplayBackend {
    Wayland,
    Slint,
}

impl VideoDisplayBackend {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Wayland => "wayland",
            Self::Slint => "slint",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VideoDisplaySelection {
    pub(crate) backend: VideoDisplayBackend,
    pub(crate) fallback_reason: Option<String>,
}

pub(crate) fn select_video_display_backend(
    preference: VideoDisplayPreference,
    wayland_error: Option<String>,
) -> eros::Result<VideoDisplaySelection> {
    match (preference, wayland_error) {
        (VideoDisplayPreference::Auto, None) | (VideoDisplayPreference::Wayland, None) => {
            Ok(VideoDisplaySelection {
                backend: VideoDisplayBackend::Wayland,
                fallback_reason: None,
            })
        }
        (VideoDisplayPreference::Auto, Some(reason)) => Ok(VideoDisplaySelection {
            backend: VideoDisplayBackend::Slint,
            fallback_reason: Some(reason),
        }),
        (VideoDisplayPreference::Wayland, Some(reason)) => {
            eros::bail!("Requested Wayland video display is unavailable: {}", reason)
        }
        (VideoDisplayPreference::Slint, _) => Ok(VideoDisplaySelection {
            backend: VideoDisplayBackend::Slint,
            fallback_reason: None,
        }),
    }
}

// Focused test: cargo test app::gui::video_view::backend::tests --lib
#[cfg(test)]
mod tests {
    use crate::app::{
        config::VideoDisplayPreference,
        gui::video_view::backend::{VideoDisplayBackend, select_video_display_backend},
    };

    #[test]
    fn auto_prefers_wayland_when_initialization_succeeds() {
        let selection = select_video_display_backend(VideoDisplayPreference::Auto, None)
            .expect("Auto display selection should accept Wayland");

        assert_eq!(selection.backend, VideoDisplayBackend::Wayland);
        assert_eq!(selection.backend.name(), "wayland");
        assert_eq!(selection.fallback_reason, None);
    }

    #[test]
    fn auto_falls_back_to_slint_with_the_wayland_failure_reason() {
        let selection = select_video_display_backend(
            VideoDisplayPreference::Auto,
            Some(String::from("compositor has no linux-dmabuf protocol")),
        )
        .expect("Auto display selection should fall back to Slint");

        assert_eq!(selection.backend, VideoDisplayBackend::Slint);
        assert_eq!(selection.backend.name(), "slint");
        assert_eq!(
            selection.fallback_reason.as_deref(),
            Some("compositor has no linux-dmabuf protocol")
        );
    }

    #[test]
    fn explicit_wayland_does_not_fall_back() {
        let error = select_video_display_backend(
            VideoDisplayPreference::Wayland,
            Some(String::from("parent wl_surface is unavailable")),
        )
        .expect_err("Explicit Wayland selection should reject initialization failure");

        assert!(format!("{error:?}").contains("parent wl_surface is unavailable"));
    }

    #[test]
    fn explicit_slint_ignores_wayland_availability() {
        let selection = select_video_display_backend(
            VideoDisplayPreference::Slint,
            Some(String::from("Wayland was not attempted")),
        )
        .expect("Explicit Slint selection should not require Wayland");

        assert_eq!(selection.backend, VideoDisplayBackend::Slint);
        assert_eq!(selection.fallback_reason, None);
    }
}
