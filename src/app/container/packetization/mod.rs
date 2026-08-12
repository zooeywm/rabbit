pub(crate) mod inbound;
pub(crate) mod outbound_port;

use crate::{
    app::container::root::outbound_port::MetricsTarget,
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

pub(crate) struct PacketizerContainer<State> {
    metrics_target: MetricsTarget,
    state: State,
}

impl<State> PacketizerContainer<State> {
    pub(crate) fn new(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        state: State,
    ) -> Self {
        Self {
            metrics_target: MetricsTarget::Stream {
                capture_source_id,
                stream_id,
            },
            state,
        }
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<State> AsRef<MetricsTarget> for PacketizerContainer<State> {
    fn as_ref(&self) -> &MetricsTarget {
        &self.metrics_target
    }
}
