mod boilerplate;
mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{
    app::container::CaptureSourceContainer, domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::CapturerManagerState,
};

pub(crate) struct RootContainer {
    /// Creates screen capturer states for capture-source containers.
    capturer_manager_state: CapturerManagerState,

    /// Owns all active capture-source containers.
    capture_sources: HashMap<CaptureSourceId, CaptureSourceContainer>,
}

impl RootContainer {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            capturer_manager_state: CapturerManagerState::new()?,
            capture_sources: HashMap::new(),
        })
    }
}
