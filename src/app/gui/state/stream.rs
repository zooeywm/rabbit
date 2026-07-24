use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_configuration::{
        ScreenResolutionStatus, ScreenStreamRequestId, ScreenStreamsConfigured,
    },
    screen_manager::ScreenId,
    session::SessionId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenStreamTarget {
    pub(crate) request_id: ScreenStreamRequestId,
    pub(crate) session_id: SessionId,
    pub(crate) screen_id: ScreenId,
    pub(crate) screen_name: String,
    pub(crate) frame_size: PixelSize,
    pub(crate) frame_rate: FrameRate,
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

    pub(crate) fn reset(&mut self) {
        *self = Self::Idle;
    }
}
