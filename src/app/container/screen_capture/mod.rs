pub(crate) mod inbound;
pub(crate) mod outbound_port;

use crate::{
    app::container::host::outbound_port::MetricsTarget, domain::stream::models::vo::CaptureSourceId,
};

pub(crate) struct ScreenCaptureContainer<State> {
    metrics_target: MetricsTarget,
    state: State,
}

impl<State> ScreenCaptureContainer<State> {
    pub(crate) fn state(&self) -> &State {
        &self.state
    }

    pub(crate) fn state_mut(&mut self) -> &mut State {
        &mut self.state
    }
}

impl<State> AsRef<MetricsTarget> for ScreenCaptureContainer<State> {
    fn as_ref(&self) -> &MetricsTarget {
        &self.metrics_target
    }
}

impl<State> From<(CaptureSourceId, State)> for ScreenCaptureContainer<State> {
    fn from((capture_source_id, state): (CaptureSourceId, State)) -> Self {
        Self {
            metrics_target: MetricsTarget::CaptureSource(capture_source_id),
            state,
        }
    }
}
