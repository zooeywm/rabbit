//! Host-side stream admission policy shared by GUI and headless hosts.

use crate::{
    app::services::host_stream::{HostStreamPlan, plan_preserved_streams},
    kernel::{
        capability::validate_set_screen_streams,
        connection_request::PeerCapabilities,
        domain_error::DomainError,
        screen_configuration::{ScreenStreamsConfigured, SetScreenStreams},
        screen_manager::{Screen, ScreenId},
    },
};

/// Result of evaluating a controller `SetScreenStreams` request on the host.
#[derive(Debug, Clone, PartialEq)]
pub struct HostStreamEvaluation {
    pub configured: ScreenStreamsConfigured,
    pub plans: Vec<HostStreamPlan>,
}

/// Validates capabilities/phase, then plans preserved streams against local topology.
pub fn evaluate_set_screen_streams(
    request: SetScreenStreams,
    lookup: impl Fn(&ScreenId) -> Option<Screen>,
    local: &PeerCapabilities,
    peer: &PeerCapabilities,
    admits_streams: bool,
) -> Result<HostStreamEvaluation, DomainError> {
    validate_set_screen_streams(&request, local, peer, admits_streams)?;
    let (configured, plans) = plan_preserved_streams(request, lookup);
    Ok(HostStreamEvaluation { configured, plans })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        connection_request::EncoderProfileTag,
        geometry::{FrameRate, PixelSize},
        screen_configuration::{
            RemoteDisplayMode, ScreenStreamRequest, ScreenStreamRequestId, SetScreenStreams,
        },
        screen_manager::{ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        video_encoder::{VideoBitrate, VideoCodec},
    };

    fn screen(id: u8) -> Screen {
        Screen {
            id: ScreenId(id),
            name: format!("s{id}"),
            resolution: PixelSize {
                width: 1920,
                height: 1080,
            },
            frame_rate: FrameRate::new(60, 1).expect("fps"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }

    #[test]
    fn evaluation_fails_closed_on_capability() {
        let peer = PeerCapabilities {
            max_screens: 0,
            encoder_profiles: vec![EncoderProfileTag::H264Hardware],
            absolute_pointer: false,
            reliable_input: false,
        };
        let request = SetScreenStreams {
            request_id: ScreenStreamRequestId(1),
            desired_streams: vec![ScreenStreamRequest {
                screen_id: ScreenId(0),
                remote_display: RemoteDisplayMode::Preserve,
                frame_size: PixelSize {
                    width: 1920,
                    height: 1080,
                },
                frame_rate: FrameRate::new(60, 1).expect("fps"),
                codec: VideoCodec::H264,
                bitrate: VideoBitrate::new(21_000_000).expect("bitrate"),
            }],
        };
        let err = evaluate_set_screen_streams(
            request,
            |_| Some(screen(0)),
            &PeerCapabilities::default(),
            &peer,
            true,
        )
        .expect_err("max_screens 0 must fail");
        assert_eq!(
            err.kind,
            crate::kernel::domain_error::DomainErrorKind::Capability
        );
    }

    #[test]
    fn evaluation_plans_known_screen() {
        let request = SetScreenStreams {
            request_id: ScreenStreamRequestId(2),
            desired_streams: vec![ScreenStreamRequest {
                screen_id: ScreenId(0),
                remote_display: RemoteDisplayMode::Preserve,
                frame_size: PixelSize {
                    width: 1280,
                    height: 720,
                },
                frame_rate: FrameRate::new(30, 1).expect("fps"),
                codec: VideoCodec::H264,
                bitrate: VideoBitrate::new(5_000_000).expect("bitrate"),
            }],
        };
        let evaluation = evaluate_set_screen_streams(
            request,
            |id| (id.get() == 0).then(|| screen(0)),
            &PeerCapabilities::default(),
            &PeerCapabilities::default(),
            true,
        )
        .expect("should plan");
        assert_eq!(evaluation.plans.len(), 1);
        assert_eq!(evaluation.plans[0].screen_id, ScreenId(0));
    }
}
