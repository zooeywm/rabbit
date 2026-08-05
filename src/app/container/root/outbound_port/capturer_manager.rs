use crate::domain::stream::models::vo::CaptureSourceId;

/// Creates screen capturer states for capture-source containers.
///
/// This port describes dependency creation only. The created state is passed
/// into a dynamically created capture-source container, which determines its
/// runtime scheduling and lifetime.
pub(crate) trait CapturerManager {
    /// The screen capturer state created by this manager.
    type ScreenCapturerState;

    /// Creates a screen capturer state for one physical capture source.
    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState>;
}
