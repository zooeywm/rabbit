use crate::domain::stream::models::vo::CaptureSourceId;

pub(crate) trait CapturerManagerStateSpec {
    type ScreenCapturerState;
}

pub(crate) trait CapturerManager {
    type ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState>;
}
