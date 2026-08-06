use crate::domain::stream::models::vo::CaptureSourceId;

pub(crate) trait CapturerManagerStateSpec {
    type ScreenCapturerState;
}

pub(crate) trait CapturerManager {
    type State: CapturerManagerStateSpec;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>;
}
