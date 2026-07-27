//! Control-plane messages on the session control channel.
//!
//! Domain types for stream configuration live in [`crate::kernel::screen_configuration`].
//! Wire codecs live in [`wire`]; this module owns the public control surface.
//!
//! Adding a message: domain type → wire tag → codec → role checks in `session`.
//! Bump [`crate::kernel::protocol`] version accordingly.

mod wire;

use crate::kernel::{
    geometry::{FrameRate, PixelSize},
    screen_configuration::{
        RequestKeyFrame, ScreenStreamsConfigured, SetScreenStreams, StopScreenStream,
    },
    screen_manager::{ScreenId, ScreenLayout},
    transport::TransportMessage,
};

#[derive(Debug, Clone, PartialEq)]
pub struct ScreenInfo {
    pub id: ScreenId,
    pub name: String,
    pub resolution: PixelSize,
    pub frame_rate: FrameRate,
    pub layout: ScreenLayout,
}

#[derive(Debug)]
pub struct OutgoingScreenList(pub(crate) TransportMessage);

#[derive(Debug, Clone, PartialEq)]
pub enum ControlMessage {
    ScreenList(Vec<ScreenInfo>),
    SetScreenStreams(SetScreenStreams),
    ScreenStreamsConfigured(ScreenStreamsConfigured),
    StopScreenStream(StopScreenStream),
    RequestKeyFrame(RequestKeyFrame),
}

#[cfg(test)]
mod tests {
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::{
            RemoteDisplayMode, ScreenStreamRequest, ScreenStreamRequestId, SetScreenStreams,
        },
        screen_manager::{Screen, ScreenId, ScreenLayout, ScreenRect, ScreenTransform},
        session_control::{
            ControlMessage, OutgoingScreenList, ScreenInfo,
            wire::{
                WireFrameRate, WirePixelSize, WireScreenInfo, WireScreenLayout, WireScreenRect,
                WireScreenTransform,
            },
        },
        transport::TransportMessage,
    };

    #[test]
    fn screen_list_round_trip_preserves_frame_rate() {
        let expected =
            FrameRate::new(143_855, 1_000).expect("Test screen frame rate should be valid");
        let screens = [Screen {
            id: ScreenId(2),
            name: "HDMI-A-1".to_string(),
            resolution: PixelSize {
                width: 2560,
                height: 1440,
            },
            frame_rate: expected,
            layout: ScreenLayout {
                rect: ScreenRect {
                    x: 0,
                    y: 0,
                    width: 2560,
                    height: 1440,
                },
                scale: 1.0,
                transform: ScreenTransform::Normal,
            },
        }];
        let message = TransportMessage::from(
            OutgoingScreenList::try_from(screens.as_slice())
                .expect("Screen list with a frame rate should encode"),
        );

        let ControlMessage::ScreenList(decoded) =
            ControlMessage::try_from(message).expect("Screen list with a frame rate should decode")
        else {
            panic!("Decoded control message should be a screen list");
        };

        assert_eq!(decoded[0].frame_rate, expected);
    }

    #[test]
    fn screen_info_rejects_invalid_frame_rate() {
        let wire = WireScreenInfo {
            id: 3,
            name: b"eDP-1".to_vec(),
            resolution: WirePixelSize {
                width: 1920,
                height: 1080,
            },
            frame_rate: WireFrameRate {
                numerator: 0,
                denominator: 1,
            },
            layout: WireScreenLayout {
                rect: WireScreenRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                scale: 1.0,
                transform: WireScreenTransform::Normal,
            },
        };

        let error = ScreenInfo::try_from(wire).expect_err("Zero frame rate should be rejected");

        assert!(
            format!("{error:?}").contains("Failed to decode ScreenInfo 3 frame rate"),
            "Invalid frame rate error should identify the ScreenInfo boundary: {error:?}"
        );
    }

    #[test]
    fn screen_stream_request_round_trip_preserves_client_settings() {
        let expected = ScreenStreamRequest {
            screen_id: ScreenId(4),
            remote_display: RemoteDisplayMode::Preserve,
            frame_size: PixelSize {
                width: 1920,
                height: 1080,
            },
            frame_rate: FrameRate::new(59_940, 1_000)
                .expect("Test stream frame rate should be valid"),
        };
        let message = TransportMessage::try_from(SetScreenStreams {
            request_id: ScreenStreamRequestId(9),
            desired_streams: vec![expected],
        })
        .expect("Screen stream request should encode");

        let ControlMessage::SetScreenStreams(decoded) =
            ControlMessage::try_from(message).expect("Screen stream request should decode")
        else {
            panic!("Decoded control message should be a screen stream request");
        };

        assert_eq!(decoded.request_id, ScreenStreamRequestId(9));
        assert_eq!(decoded.desired_streams, vec![expected]);
    }
}
