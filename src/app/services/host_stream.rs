//! Host-side screen-stream configuration policy.
//!
//! Given a controller's [`SetScreenStreams`] request and the host's local screen
//! topology, produce the control-plane reply and the concrete pipelines the host
//! should start.

use crate::kernel::{
    frame_pipeline::FramePipelineParameters,
    screen_configuration::{
        RemoteDisplayMode, ResolutionResult, ScreenResolutionOutcome, ScreenResolutionStatus,
        ScreenStreamRequest, ScreenStreamsConfigured, SetScreenStreams,
    },
    screen_manager::{Screen, ScreenId},
    video_encoder::VideoEncoderParameters,
};

/// One host pipeline subscription that should run after configuration succeeds.
#[derive(Debug, Clone, PartialEq)]
pub struct HostStreamPlan {
    pub screen_id: ScreenId,
    pub parameters: FramePipelineParameters,
    pub encoding: VideoEncoderParameters,
}

/// Resolves a controller stream request against the host's published screens.
///
/// Current policy only supports [`RemoteDisplayMode::Preserve`]: the host keeps
/// its native resolution and streams at the client-requested size/rate envelope.
pub fn plan_preserved_streams(
    request: SetScreenStreams,
    lookup: impl Fn(&ScreenId) -> Option<Screen>,
) -> (ScreenStreamsConfigured, Vec<HostStreamPlan>) {
    let SetScreenStreams {
        request_id,
        desired_streams,
    } = request;
    let mut plans = Vec::new();
    let outcomes = desired_streams
        .into_iter()
        .map(|desired| resolve_desired_stream(desired, &lookup, &mut plans))
        .collect();

    (
        ScreenStreamsConfigured {
            request_id,
            outcomes,
        },
        plans,
    )
}

fn resolve_desired_stream(
    desired: ScreenStreamRequest,
    lookup: &impl Fn(&ScreenId) -> Option<Screen>,
    plans: &mut Vec<HostStreamPlan>,
) -> ScreenResolutionOutcome {
    let status = match lookup(&desired.screen_id) {
        Some(screen) => match desired.remote_display {
            RemoteDisplayMode::Preserve => {
                plans.push(HostStreamPlan {
                    screen_id: desired.screen_id,
                    parameters: FramePipelineParameters {
                        frame_size: desired.frame_size,
                        frame_rate_mode: desired.frame_rate_mode,
                    },
                    encoding: VideoEncoderParameters {
                        codec: desired.codec,
                        frame_rate: desired.frame_rate,
                        frame_rate_mode: desired.frame_rate_mode,
                        bitrate: desired.bitrate,
                        fec_percentage: desired.fec_percentage,
                    },
                });
                ScreenResolutionStatus::Configured(ResolutionResult::Preserved {
                    requested: desired.frame_size,
                    actual: screen.resolution,
                })
            }
        },
        None => ScreenResolutionStatus::Failed {
            requested: desired.frame_size,
            actual: None,
        },
    };

    ScreenResolutionOutcome {
        screen_id: desired.screen_id,
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::{ScreenStreamRequest, ScreenStreamRequestId},
        screen_manager::{ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        video_encoder::{VideoBitrate, VideoCodec},
    };

    fn sample_screen(id: u8, width: u32, height: u32) -> Screen {
        Screen {
            id: ScreenId(id),
            name: format!("screen-{id}"),
            resolution: PixelSize { width, height },
            frame_rate: FrameRate::new(60, 1).expect("frame rate"),
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }
    }

    #[test]
    fn plans_preserve_mode_for_known_screens_and_fails_unknown() {
        let screens = [sample_screen(0, 2560, 1600)];
        let request = SetScreenStreams {
            request_id: ScreenStreamRequestId(3),
            desired_streams: vec![
                ScreenStreamRequest {
                    screen_id: ScreenId(0),
                    remote_display: RemoteDisplayMode::Preserve,
                    frame_size: PixelSize {
                        width: 1920,
                        height: 1200,
                    },
                    frame_rate: FrameRate::new(60, 1).expect("frame rate"),
                    frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode::Dynamic,
                    codec: VideoCodec::H264,
                    bitrate: VideoBitrate::new(24_000_000).expect("bitrate"),
                    fec_percentage: crate::kernel::video_encoder::VideoFecPercentage::DEFAULT,
                },
                ScreenStreamRequest {
                    screen_id: ScreenId(9),
                    remote_display: RemoteDisplayMode::Preserve,
                    frame_size: PixelSize {
                        width: 1280,
                        height: 720,
                    },
                    frame_rate: FrameRate::new(30, 1).expect("frame rate"),
                    frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode::Dynamic,
                    codec: VideoCodec::H264,
                    bitrate: VideoBitrate::new(5_000_000).expect("bitrate"),
                    fec_percentage: crate::kernel::video_encoder::VideoFecPercentage::DEFAULT,
                },
            ],
        };

        let (configured, plans) = plan_preserved_streams(request, |id| {
            screens.iter().find(|screen| screen.id == *id).cloned()
        });

        assert_eq!(configured.request_id, ScreenStreamRequestId(3));
        assert_eq!(configured.outcomes.len(), 2);
        assert!(matches!(
            configured.outcomes[0].status,
            ScreenResolutionStatus::Configured(ResolutionResult::Preserved {
                requested: PixelSize {
                    width: 1920,
                    height: 1200
                },
                actual: PixelSize {
                    width: 2560,
                    height: 1600
                },
            })
        ));
        assert!(matches!(
            configured.outcomes[1].status,
            ScreenResolutionStatus::Failed { .. }
        ));
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].screen_id, ScreenId(0));
        assert_eq!(
            plans[0].parameters.frame_size,
            PixelSize {
                width: 1920,
                height: 1200
            }
        );
        assert_eq!(plans[0].encoding.codec, VideoCodec::H264);
        assert_eq!(
            plans[0].encoding.bitrate,
            VideoBitrate::new(24_000_000).expect("bitrate")
        );
    }
}
