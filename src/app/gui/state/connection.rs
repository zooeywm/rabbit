use std::{
    fmt,
    net::{IpAddr, SocketAddr},
};

use eros::Context;

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
