use eros::Context;

use crate::{
    app::container::{CaptureSourceContainer, root::outbound_port::CapturerManager},
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

use super::RootContainer;

impl RootContainer {
    /// Returns the existing capture-source container or creates a new one.
    fn get_or_create_capture_source(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<&mut CaptureSourceContainer> {
        if !self.capture_sources.contains_key(&capture_source_id) {
            let screen_capturer_state = self.create_screen_capturer(capture_source_id)?;

            let capture_source = CaptureSourceContainer::new(screen_capturer_state);

            self.capture_sources
                .insert(capture_source_id, capture_source);
        }

        Ok(self
            .capture_sources
            .get_mut(&capture_source_id)
            .with_context(|| "Capture source container was not found after creation")?)
    }

    /// Removes one stream and immediately removes its capture source when the
    /// stream was its final consumer.
    pub(crate) fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let capture_source_id = self
            .capture_sources
            .iter()
            .find_map(|(capture_source_id, capture_source)| {
                capture_source
                    .contains_stream(stream_id)
                    .then_some(*capture_source_id)
            })
            .with_context(|| "Stream does not exist")?;

        let remove_capture_source = self
            .capture_sources
            .get_mut(&capture_source_id)
            .with_context(|| "Capture source container disappeared while removing stream")?
            .remove_stream(stream_id)?;

        if remove_capture_source {
            self.capture_sources.remove(&capture_source_id);
        }

        Ok(())
    }
}
