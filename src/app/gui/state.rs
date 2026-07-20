use std::{
    fmt,
    net::{IpAddr, SocketAddr},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DirectTarget {
    ip: IpAddr,
    port: Option<u16>,
}

impl DirectTarget {
    pub(crate) fn new(ip: IpAddr, port: Option<u16>) -> Self {
        Self { ip, port }
    }
}

impl fmt::Display for DirectTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.port {
            Some(port) => SocketAddr::new(self.ip, port).fmt(formatter),
            None => self.ip.fmt(formatter),
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

#[derive(Debug, Default, PartialEq, Eq)]
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
        let target = *target;

        *self = match completion {
            DirectConnectionCompletion::Connected(peer) => Self::Connected { peer },
            DirectConnectionCompletion::Rejected => Self::Rejected { target },
            DirectConnectionCompletion::SelfRejected => Self::SelfRejected { target },
            DirectConnectionCompletion::Failed(message) => Self::Failed { target, message },
        };
        true
    }

    pub(crate) fn is_connecting(&self) -> bool {
        matches!(self, Self::Connecting { .. })
    }

    pub(crate) fn status(&self) -> Option<String> {
        match self {
            Self::Idle => None,
            Self::Connecting { target } => Some(format!("Connecting to {target}...")),
            Self::Connected { peer } => Some(format!("Connected to {peer}")),
            Self::Rejected { target } => Some(format!("Connection to {target} was rejected")),
            Self::SelfRejected { target } => Some(format!(
                "Cannot connect to this Rabbit instance at {target}"
            )),
            Self::Failed { target, message } => {
                Some(format!("Failed to connect to {target}: {message}"))
            }
        }
    }
}

// Focused test: cargo test app::gui::state::tests
#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use crate::app::gui::state::{DirectConnectionCompletion, DirectConnectionState, DirectTarget};

    #[test]
    fn direct_connection_flow_preserves_the_target_until_completion() {
        let target = DirectTarget::new(IpAddr::V4(Ipv4Addr::LOCALHOST), None);
        let peer = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 52732);
        let mut state = DirectConnectionState::default();

        assert!(state.begin(target));
        assert!(!state.begin(DirectTarget::new(
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Some(52733)
        )));
        assert_eq!(
            state.status().as_deref(),
            Some("Connecting to 127.0.0.1...")
        );

        assert!(state.complete(DirectConnectionCompletion::Connected(peer)));
        assert_eq!(
            state.status().as_deref(),
            Some("Connected to 127.0.0.1:52732")
        );
    }

    #[test]
    fn direct_connection_flow_distinguishes_remote_and_self_rejection() {
        let target = DirectTarget::new(IpAddr::V4(Ipv4Addr::LOCALHOST), Some(52731));
        let mut state = DirectConnectionState::default();

        assert!(state.begin(target));
        assert!(state.complete(DirectConnectionCompletion::Rejected));
        assert_eq!(
            state.status().as_deref(),
            Some("Connection to 127.0.0.1:52731 was rejected")
        );

        assert!(state.begin(target));
        assert!(state.complete(DirectConnectionCompletion::SelfRejected));
        assert_eq!(
            state.status().as_deref(),
            Some("Cannot connect to this Rabbit instance at 127.0.0.1:52731")
        );
    }

    #[test]
    fn direct_connection_flow_keeps_failure_context() {
        let target = DirectTarget::new(IpAddr::V4(Ipv4Addr::LOCALHOST), Some(52731));
        let mut state = DirectConnectionState::default();

        assert!(state.begin(target));
        assert!(state.complete(DirectConnectionCompletion::Failed("timed out".into())));
        assert_eq!(
            state.status().as_deref(),
            Some("Failed to connect to 127.0.0.1:52731: timed out")
        );
    }
}
