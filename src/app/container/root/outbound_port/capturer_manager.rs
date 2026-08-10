use crate::app::container::{
    root::outbound_port::MetricsRecorder, screen_capture::outbound_port::ScreenCapturer,
};
use crate::domain::stream::models::vo::CaptureSourceId;

pub(crate) trait CapturerManagerStateSpec {
    type ScreenCapturerState: 'static;
    type ScreenCapturer: ScreenCapturer
        + MetricsRecorder
        + From<(CaptureSourceId, Self::ScreenCapturerState)>
        + 'static;
}

pub(crate) trait CapturerManager {
    type State: CapturerManagerStateSpec;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<Self>;
}
