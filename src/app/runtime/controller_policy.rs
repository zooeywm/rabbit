//! Controller-side stream request admission before the wire is touched.

use crate::kernel::{
    capability::validate_controller_set_screen_streams, connection_request::PeerCapabilities,
    domain_error::DomainError, screen_configuration::SetScreenStreams,
};

/// Validates a controller `SetScreenStreams` against local and remote host budgets.
pub fn evaluate_controller_set_screen_streams(
    request: &SetScreenStreams,
    controller: &PeerCapabilities,
    host: &PeerCapabilities,
    admits_streams: bool,
) -> Result<(), DomainError> {
    validate_controller_set_screen_streams(request, controller, host, admits_streams)
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
        screen_manager::ScreenId,
        video_encoder::{VideoBitrate, VideoCodec},
    };

    #[test]
    fn rejects_when_remote_host_has_no_screen_budget() {
        let host = PeerCapabilities {
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
                frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode::Dynamic,
                codec: VideoCodec::H264,
                bitrate: VideoBitrate::new(21_000_000).expect("bitrate"),
                fec_percentage: crate::kernel::video_encoder::VideoFecPercentage::DEFAULT,
            }],
        };
        let err = evaluate_controller_set_screen_streams(
            &request,
            &PeerCapabilities::default(),
            &host,
            true,
        )
        .expect_err("must reject");
        assert_eq!(
            err.kind,
            crate::kernel::domain_error::DomainErrorKind::Capability
        );
    }
}
