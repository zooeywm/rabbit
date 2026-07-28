mod connection;
mod stream;
mod view_model;

pub(crate) use connection::{DirectConnectionCompletion, DirectConnectionState, DirectTarget};
pub(crate) use stream::{ScreenStreamState, ScreenStreamTarget};
pub(crate) use view_model::{
    ConnectedDeviceView, ConnectionRequestView, HostedScreenStreamView, RemoteScreenView, ViewPage,
    ViewState, WorkspaceSection, format_frame_rate, parse_stream_settings,
};

// Focused test: cargo test app::gui::state::tests
#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::app::gui::state::{
        DirectConnectionCompletion, DirectConnectionState, DirectTarget, ScreenStreamState,
        ScreenStreamTarget, format_frame_rate, parse_stream_settings,
    };
    use crate::kernel::{
        geometry::{FrameRate, PixelSize},
        screen_configuration::{
            ResolutionResult, ScreenResolutionOutcome, ScreenResolutionStatus,
            ScreenStreamRequestId, ScreenStreamsConfigured,
        },
        screen_manager::ScreenId,
        session::SessionId,
    };

    #[test]
    fn direct_connection_flow_preserves_the_target_until_completion() {
        let target = DirectTarget::new(Ipv4Addr::LOCALHOST.to_string(), None);
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 52732);
        let mut state = DirectConnectionState::default();

        assert!(state.begin(target.clone()));
        assert!(!state.begin(DirectTarget::new(
            Ipv4Addr::LOCALHOST.to_string(),
            Some(52733)
        )));
        assert_eq!(
            state,
            DirectConnectionState::Connecting {
                target: target.clone()
            }
        );

        assert!(state.complete(DirectConnectionCompletion::Connected(peer)));
        assert_eq!(state, DirectConnectionState::Connected { peer });
    }

    #[test]
    fn direct_connection_flow_distinguishes_remote_and_self_rejection() {
        let target = DirectTarget::new(Ipv4Addr::LOCALHOST.to_string(), Some(52731));
        let mut state = DirectConnectionState::default();

        assert!(state.begin(target.clone()));
        assert!(state.complete(DirectConnectionCompletion::Rejected));
        assert_eq!(
            state,
            DirectConnectionState::Rejected {
                target: target.clone()
            }
        );

        assert!(state.begin(target.clone()));
        assert!(state.complete(DirectConnectionCompletion::SelfRejected));
        assert_eq!(state, DirectConnectionState::SelfRejected { target });
    }

    #[test]
    fn direct_target_accepts_hostname_with_port() {
        let target =
            DirectTarget::parse("test.io:23944").expect("Hostname direct target should parse");

        assert_eq!(target.host(), "test.io");
        assert_eq!(target.port(), Some(23944));
        assert_eq!(target.to_string(), "test.io:23944");
    }

    #[test]
    fn screen_frame_rate_display_omits_redundant_decimal_zeroes() {
        assert_eq!(
            format_frame_rate(FrameRate::new(120_000, 1_000).expect("Frame rate should be valid")),
            "120"
        );
        assert_eq!(
            format_frame_rate(FrameRate::new(143_855, 1_000).expect("Frame rate should be valid")),
            "143.855"
        );
    }

    #[test]
    fn stream_settings_parse_even_resolution_and_decimal_frame_rate() {
        let (size, frame_rate) = parse_stream_settings("2560", "1440", "143.855")
            .expect("Valid stream settings should parse");

        assert_eq!(
            size,
            PixelSize {
                width: 2560,
                height: 1440
            }
        );
        assert_eq!(frame_rate.numerator(), 143_855);
        assert_eq!(frame_rate.denominator(), 1_000);
    }

    #[test]
    fn stream_settings_reject_odd_resolution_and_zero_frame_rate() {
        assert!(parse_stream_settings("1919", "1080", "60").is_err());
        assert!(parse_stream_settings("1920", "1080", "0").is_err());
    }

    #[test]
    fn screen_stream_progresses_from_request_to_first_video_frame() {
        let target = ScreenStreamTarget {
            request_id: ScreenStreamRequestId(7),
            session_id: SessionId(2),
            screen_id: ScreenId(1),
            screen_name: "eDP-1".to_string(),
            frame_size: PixelSize {
                width: 1920,
                height: 1200,
            },
            frame_rate: FrameRate::new(120, 1).expect("Test frame rate should be valid"),
        };
        let mut state = ScreenStreamState::default();
        state.begin(target.clone());

        assert!(state.apply_configuration(&ScreenStreamsConfigured {
            request_id: target.request_id,
            outcomes: vec![ScreenResolutionOutcome {
                screen_id: target.screen_id,
                status: ScreenResolutionStatus::Configured(ResolutionResult::Preserved {
                    requested: target.frame_size,
                    actual: target.frame_size,
                }),
            }],
        }));
        assert_eq!(state, ScreenStreamState::WaitingForVideo(target.clone()));

        assert!(state.receive_video(target.session_id, target.screen_id));
        assert_eq!(state, ScreenStreamState::Streaming(target));
    }
}
