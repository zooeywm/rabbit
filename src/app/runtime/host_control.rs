//! Host-facing session control classification shared by GUI and headless shells.

use crate::{
    app::runtime::host_policy::{HostStreamEvaluation, evaluate_set_screen_streams},
    kernel::{
        connection_request::PeerCapabilities,
        domain_error::DomainError,
        screen_manager::{Screen, ScreenId},
        session::SessionMessage,
        session_control::ControlMessage,
    },
};

/// Decision produced from a host-role session message.
#[derive(Debug)]
pub enum HostControlDecision {
    /// Controller asked for streams; policy may accept or reject.
    SetScreenStreams(Result<HostStreamEvaluation, DomainError>),
    RequestKeyFrame(ScreenId),
    StopScreenStream(ScreenId),
    /// Message is not meaningful for the local host role.
    Ignore,
}

/// Classifies a received session message for host-side handling.
pub fn classify_host_session_message(
    message: SessionMessage,
    local: &PeerCapabilities,
    peer: &PeerCapabilities,
    admits_streams: bool,
    lookup: impl Fn(&ScreenId) -> Option<Screen>,
) -> HostControlDecision {
    match message {
        SessionMessage::Control(ControlMessage::SetScreenStreams(request)) => {
            HostControlDecision::SetScreenStreams(evaluate_set_screen_streams(
                request,
                lookup,
                local,
                peer,
                admits_streams,
            ))
        }
        SessionMessage::Control(ControlMessage::RequestKeyFrame(request)) => {
            HostControlDecision::RequestKeyFrame(request.screen_id)
        }
        SessionMessage::Control(ControlMessage::StopScreenStream(stop)) => {
            HostControlDecision::StopScreenStream(stop.screen_id)
        }
        SessionMessage::Control(_)
        | SessionMessage::Video(_)
        | SessionMessage::KeyFrameRequired(_) => HostControlDecision::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::RequestKeyFrame,
        screen_configuration::{
            RemoteDisplayMode, ScreenStreamRequest, ScreenStreamRequestId, SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        session_control::ControlMessage as _,
    };

    fn screen(id: u8) -> Screen {
        Screen {
            id: ScreenId(id),
            name: "s".into(),
            resolution: PixelSize {
                width: 100,
                height: 100,
            },
            frame_rate: FrameRate::new(60, 1).expect("fps"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }

    #[test]
    fn classifies_key_frame_and_set_streams() {
        let decision = classify_host_session_message(
            SessionMessage::Control(ControlMessage::RequestKeyFrame(RequestKeyFrame {
                screen_id: ScreenId(3),
            })),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
            |_| None,
        );
        assert!(matches!(
            decision,
            HostControlDecision::RequestKeyFrame(ScreenId(3))
        ));

        let request = SetScreenStreams {
            request_id: ScreenStreamRequestId(1),
            desired_streams: vec![ScreenStreamRequest {
                screen_id: ScreenId(0),
                remote_display: RemoteDisplayMode::Preserve,
                frame_size: PixelSize {
                    width: 100,
                    height: 100,
                },
                frame_rate: FrameRate::new(60, 1).expect("fps"),
            }],
        };
        let decision = classify_host_session_message(
            SessionMessage::Control(ControlMessage::SetScreenStreams(request)),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
            |id| (id.get() == 0).then(|| screen(0)),
        );
        assert!(matches!(
            decision,
            HostControlDecision::SetScreenStreams(Ok(_))
        ));
    }
}
