use std::{
    fmt,
    net::{IpAddr, SocketAddr},
};

use eros::Context;

use crate::kernel::{
    geometry::PixelSize,
    screen_configuration::{
        ScreenResolutionStatus, ScreenStreamRequestId, ScreenStreamsConfigured,
    },
    screen_manager::ScreenId,
    session::SessionId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirectTarget {
    host: String,
    port: Option<u16>,
}

impl DirectTarget {
    pub(crate) fn new(host: String, port: Option<u16>) -> Self {
        Self { host, port }
    }

    pub(crate) fn parse(input: &str) -> eros::Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            eros::bail!("Direct connection address is empty");
        }

        if let Ok(address) = input.parse::<SocketAddr>() {
            return Ok(Self::new(address.ip().to_string(), Some(address.port())));
        }
        if let Ok(ip) = input.parse::<IpAddr>() {
            return Ok(Self::new(ip.to_string(), None));
        }

        let (host, port) = match input.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => {
                let port = port
                    .parse::<u16>()
                    .with_context(|| format!("Failed to parse direct connection port {port:?}"))?;
                (host, Some(port))
            }
            Some(_) => eros::bail!("Direct connection hostname is empty"),
            None => (input, None),
        };
        if host.chars().any(char::is_whitespace) {
            eros::bail!("Direct connection hostname contains whitespace");
        }

        Ok(Self::new(host.to_string(), port))
    }

    pub(crate) fn host(&self) -> &str {
        &self.host
    }

    pub(crate) fn port(&self) -> Option<u16> {
        self.port
    }

    pub(crate) fn ip(&self) -> Option<IpAddr> {
        self.host.parse().ok()
    }
}

impl fmt::Display for DirectTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.ip(), self.port) {
            (Some(ip), Some(port)) => SocketAddr::new(ip, port).fmt(formatter),
            (_, Some(port)) => write!(formatter, "{}:{port}", self.host),
            (_, None) => self.host.fmt(formatter),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DirectConnectionCompletion {
    Connected(SocketAddr),
    Rejected,
    SelfRejected,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum DirectConnectionState {
    #[default]
    Idle,
    Connecting {
        target: DirectTarget,
    },
    Connected {
        peer: SocketAddr,
    },
    Rejected {
        target: DirectTarget,
    },
    SelfRejected {
        target: DirectTarget,
    },
    Failed {
        target: DirectTarget,
        message: String,
    },
}

impl DirectConnectionState {
    pub(crate) fn begin(&mut self, target: DirectTarget) -> bool {
        if self.is_connecting() {
            return false;
        }

        *self = Self::Connecting { target };
        true
    }

    pub(crate) fn complete(&mut self, completion: DirectConnectionCompletion) -> bool {
        let Self::Connecting { target } = self else {
            return false;
        };
        let target = target.clone();

        *self = match completion {
            DirectConnectionCompletion::Connected(peer) => Self::Connected { peer },
            DirectConnectionCompletion::Rejected => Self::Rejected { target },
            DirectConnectionCompletion::SelfRejected => Self::SelfRejected { target },
            DirectConnectionCompletion::Failed(message) => Self::Failed { target, message },
        };
        true
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::Idle;
    }

    pub(crate) fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ScreenStreamTarget {
    pub(crate) request_id: ScreenStreamRequestId,
    pub(crate) session_id: SessionId,
    pub(crate) screen_id: ScreenId,
    pub(crate) screen_name: String,
    pub(crate) frame_size: PixelSize,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ViewPage {
    #[default]
    Connect,
    Connecting,
    ConnectionError,
    Requests,
    Connected,
    StreamRequest,
    Streaming,
    StreamError,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum WorkspaceSection {
    #[default]
    RemoteDevices,
    ThisDevice,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConnectionRequestView {
    pub(crate) name: String,
    pub(crate) address: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ConnectedDeviceView {
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct HostedScreenStreamView {
    pub(crate) device_name: String,
    pub(crate) screen_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RemoteScreenView {
    pub(crate) name: String,
    pub(crate) resolution: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ViewState {
    pub(crate) section: WorkspaceSection,
    pub(crate) page: ViewPage,
    pub(crate) page_title: String,
    pub(crate) page_subtitle: String,
    pub(crate) status_text: String,
    pub(crate) local_protocol: String,
    pub(crate) local_port: String,
    pub(crate) local_server_online: bool,
    pub(crate) stream_title: String,
    pub(crate) stream_resolution: String,
    pub(crate) connection_requests: Vec<ConnectionRequestView>,
    pub(crate) connected_devices: Vec<ConnectedDeviceView>,
    pub(crate) hosted_screen_streams: Vec<HostedScreenStreamView>,
    pub(crate) remote_screens: Vec<RemoteScreenView>,
}

// Focused test: cargo test app::gui::state::tests
#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::app::gui::state::{
        DirectConnectionCompletion, DirectConnectionState, DirectTarget, ScreenStreamState,
        ScreenStreamTarget,
    };
    use crate::kernel::{
        geometry::PixelSize,
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
        let target = DirectTarget::parse("test.io:23944")
            .expect("Hostname direct target should parse");

        assert_eq!(target.host(), "test.io");
        assert_eq!(target.port(), Some(23944));
        assert_eq!(target.to_string(), "test.io:23944");
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
