mod boilerplate;
mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{
    app::container::CaptureSourceContainer,
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{CapturerManagerState, ConverterManagerState, EncoderManagerState},
};

pub(crate) struct RootContainer {
    /// Creates screen capturer states for capture-source containers.
    capturer_manager_state: CapturerManagerState,

    /// Creates frame-converter states for stream-pipeline containers.
    converter_manager_state: ConverterManagerState,

    /// Creates video-encoder states for stream-pipeline containers.
    encoder_manager_state: EncoderManagerState,

    /// Owns all active capture-source containers.
    capture_sources: HashMap<CaptureSourceId, CaptureSourceContainer>,

    /// The numeric value assigned to the next successfully created stream.
    next_stream_id: u16,
}

impl RootContainer {
    pub(crate) fn new() -> eros::Result<Self> {
        Ok(Self {
            capturer_manager_state: CapturerManagerState::new()?,
            converter_manager_state: ConverterManagerState::new()?,
            encoder_manager_state: EncoderManagerState::new()?,
            capture_sources: HashMap::new(),
            next_stream_id: 0,
        })
    }
}
