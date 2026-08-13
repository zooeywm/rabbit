use crate::domain::stream::models::vo::CaptureSourceId;

pub(crate) trait ScreenCapturerState: Sized + 'static {
    fn new(capture_source_id: CaptureSourceId) -> eros::Result<Self>;
}
