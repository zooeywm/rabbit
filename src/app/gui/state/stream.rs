use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_configuration::{
        ScreenResolutionStatus, ScreenStreamRequestId, ScreenStreamsConfigured,
    },
    screen_manager::ScreenId,
    session::SessionId,
    video_encoder::{VideoBitrate, VideoCodec},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenStreamTarget {
    pub(crate) request_id: ScreenStreamRequestId,
    pub(crate) session_id: SessionId,
    pub(crate) screen_id: ScreenId,
    pub(crate) screen_name: String,
    pub(crate) frame_size: PixelSize,
    pub(crate) frame_rate: FrameRate,
    pub(crate) frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode,
    pub(crate) codec: VideoCodec,
    pub(crate) bitrate: VideoBitrate,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ScreenStreamState {
    #[default]
    Idle,
    Requesting(ScreenStreamTarget),
    WaitingForVideo(ScreenStreamTarget),
    Streaming(ScreenStreamTarget),
    Failed {
        target: ScreenStreamTarget,
        message: String,
    },
}

impl ScreenStreamState {
    pub(crate) fn begin(&mut self, target: ScreenStreamTarget) {
        *self = Self::Requesting(target);
    }

    pub(crate) fn apply_configuration(&mut self, configured: &ScreenStreamsConfigured) -> bool {
        let Self::Requesting(target) = self else {
            return false;
        };
        if target.request_id != configured.request_id {
            return false;
        }

        let target = target.clone();
        let outcome = configured
            .outcomes
            .iter()
            .find(|outcome| outcome.screen_id == target.screen_id);
        *self = match outcome.map(|outcome| &outcome.status) {
            Some(ScreenResolutionStatus::Configured(_)) => Self::WaitingForVideo(target),
            Some(ScreenResolutionStatus::Failed { .. }) => Self::Failed {
                target,
                message: "The remote device could not configure this screen".to_string(),
            },
            None => Self::Failed {
                target,
                message: "The remote device did not report a result for this screen".to_string(),
            },
        };
        true
    }

    pub(crate) fn receive_video(&mut self, session_id: SessionId, screen_id: ScreenId) -> bool {
        let target = match self {
            Self::Requesting(target) | Self::WaitingForVideo(target)
                if target.session_id == session_id && target.screen_id == screen_id =>
            {
                target.clone()
            }
            _ => return false,
        };
        *self = Self::Streaming(target);
        true
    }

    pub(crate) fn waiting_target(&self) -> Option<&ScreenStreamTarget> {
        match self {
            Self::WaitingForVideo(target) => Some(target),
            _ => None,
        }
    }

    pub(crate) fn begin_recovery(&mut self, session_id: SessionId, screen_id: ScreenId) -> bool {
        let Self::Streaming(target) = self else {
            return false;
        };
        if target.session_id != session_id || target.screen_id != screen_id {
            return false;
        }
        *self = Self::WaitingForVideo(target.clone());
        true
    }

    pub(crate) fn fail(
        &mut self,
        session_id: SessionId,
        screen_id: ScreenId,
        message: String,
    ) -> bool {
        let target = match self {
            Self::Requesting(target) | Self::WaitingForVideo(target) | Self::Streaming(target)
                if target.session_id == session_id && target.screen_id == screen_id =>
            {
                target.clone()
            }
            _ => return false,
        };
        *self = Self::Failed { target, message };
        true
    }

    pub(crate) fn fail_session(&mut self, session_id: SessionId, message: String) -> bool {
        let (target_session_id, screen_id) = match self {
            Self::Requesting(target) | Self::WaitingForVideo(target) | Self::Streaming(target) => {
                (target.session_id, target.screen_id)
            }
            Self::Idle | Self::Failed { .. } => return false,
        };
        if target_session_id != session_id {
            return false;
        }

        self.fail(session_id, screen_id, message)
    }

    pub(crate) fn active_screen(&self) -> Option<(SessionId, ScreenId)> {
        match self {
            Self::Requesting(target)
            | Self::WaitingForVideo(target)
            | Self::Streaming(target)
            | Self::Failed { target, .. } => Some((target.session_id, target.screen_id)),
            Self::Idle => None,
        }
    }

    pub(crate) fn active_request_id(&self) -> Option<ScreenStreamRequestId> {
        match self {
            Self::Requesting(target)
            | Self::WaitingForVideo(target)
            | Self::Streaming(target)
            | Self::Failed { target, .. } => Some(target.request_id),
            Self::Idle => None,
        }
    }

    pub(crate) fn streaming_target(&self) -> Option<&ScreenStreamTarget> {
        match self {
            Self::Streaming(target) => Some(target),
            _ => None,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::Idle;
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::ScreenStreamRequestId,
        screen_manager::ScreenId,
        session::SessionId,
        video_encoder::{VideoBitrate, VideoCodec, VideoFrameRateMode},
    };

    use super::{ScreenStreamState, ScreenStreamTarget};

    fn target() -> ScreenStreamTarget {
        ScreenStreamTarget {
            request_id: ScreenStreamRequestId(7),
            session_id: SessionId(3),
            screen_id: ScreenId(2),
            screen_name: "screen".to_string(),
            frame_size: PixelSize {
                width: 1920,
                height: 1080,
            },
            frame_rate: FrameRate::new(60, 1).expect("frame rate"),
            frame_rate_mode: VideoFrameRateMode::Dynamic,
            codec: VideoCodec::H264,
            bitrate: VideoBitrate::new(10_000_000).expect("bitrate"),
        }
    }

    #[test]
    fn decoder_recovery_returns_stream_to_first_frame_wait() {
        let target = target();
        let mut state = ScreenStreamState::Streaming(target.clone());

        assert!(state.begin_recovery(target.session_id, target.screen_id));
        assert_eq!(state.waiting_target(), Some(&target));
        assert!(!state.begin_recovery(target.session_id, target.screen_id));
        assert!(state.receive_video(target.session_id, target.screen_id));
        assert_eq!(state.streaming_target(), Some(&target));
    }
}
