use eros::Context;

use crate::{
    app::container::{
        CaptureSourceContainer, StreamPipelineContainer,
        root::outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec,
        },
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

use super::{CaptureSourceFor, RootContainer};

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> RootContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
{
    fn get_or_create_capture_source(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<&mut CaptureSourceFor<CapMgrSt, CvtMgrSt, EcdMgrSt>> {
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

    fn create_stream_pipeline_container(
        &mut self,
    ) -> eros::Result<
        StreamPipelineContainer<CvtMgrSt::EncoderFrameConverterState, EcdMgrSt::VideoEncoderState>,
    > {
        let encoder_frame_converter_state = self.create_encoder_frame_converter()?;

        let video_encoder_state = self.create_video_encoder()?;

        Ok(StreamPipelineContainer::new(
            encoder_frame_converter_state,
            video_encoder_state,
        ))
    }

    pub(crate) fn start_stream(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<StreamId> {
        let stream_id = StreamId::new(self.next_stream_id);

        let next_stream_id = self
            .next_stream_id
            .checked_add(1)
            .with_context(|| "Stream ID space is exhausted")?;

        let stream_pipeline = self.create_stream_pipeline_container()?;

        let capture_source = self.get_or_create_capture_source(capture_source_id)?;

        capture_source.add_stream(stream_id, stream_pipeline)?;

        self.next_stream_id = next_stream_id;

        Ok(stream_id)
    }

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
