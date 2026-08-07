use crate::app::container::capture_source::outbound_port::ScreenCapturer;
use crate::domain::stream::models::vo::CaptureSourceId;

pub(crate) trait CapturerManagerStateSpec {
    type ScreenCapturerState: 'static;
    type ScreenCapturer: ScreenCapturer + From<Self::ScreenCapturerState> + 'static;
}

pub(crate) trait CapturerManager {
    type State: CapturerManagerStateSpec;

    fn screen_capturer_state_factory(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<Self>;
}
