use eros::Context;

use crate::{
    app::container::root::outbound_port::{
        CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
        EncoderManager, EncoderManagerStateSpec,
    },
    app::container::{
        CaptureSourceContainer, CaptureWorker, StreamPipelineContainer, StreamPipelineWorker,
        root::{CapturedFrameFor, RootContainer},
        stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

type StreamPipelineFor<CvtMgrSt, EcdMgrSt> = StreamPipelineContainer<
    <CvtMgrSt as ConverterManagerStateSpec>::EncoderFrameConverterState,
    <EcdMgrSt as EncoderManagerStateSpec>::VideoEncoderState,
>;

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> RootContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    Self: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    StreamPipelineFor<CvtMgrSt, EcdMgrSt>:
        EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
            + VideoEncoder<
                EncoderInput = <StreamPipelineFor<CvtMgrSt, EcdMgrSt> as EncoderFrameConverter>::EncoderInput,
            >
            + 'static,
{
    fn stream_pipeline_state_factory(
        &mut self,
    ) -> impl FnOnce()
    -> eros::Result<(
        CvtMgrSt::EncoderFrameConverterState,
        EcdMgrSt::VideoEncoderState,
    )>
    + Send
    + 'static {
        let create_converter_state = self.encoder_frame_converter_state_factory();
        let create_encoder_state = self.video_encoder_state_factory();

        move || Ok((create_converter_state()?, create_encoder_state()?))
    }

    pub(crate) async fn start_stream(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<StreamId> {
        let stream_id = StreamId::new(self.next_stream_id);
        let next_stream_id = self
            .next_stream_id
            .checked_add(1)
            .with_context(|| "Stream ID space is exhausted")?;

        let create_screen_capturer_state = if self.capture_sources.contains_key(&capture_source_id)
        {
            None
        } else {
            Some(self.screen_capturer_state_factory(capture_source_id))
        };

        let create_pipeline_states = self.stream_pipeline_state_factory();
        let stream_pipeline = StreamPipelineWorker::spawn::<CapturedFrameFor<CapMgrSt>, _, _>(
            stream_id,
            create_pipeline_states,
        )?;

        if let Some(create_screen_capturer_state) = create_screen_capturer_state {
            let capture_worker = match CaptureWorker::spawn::<CapMgrSt::ScreenCapturer, _>(
                create_screen_capturer_state,
                stream_id,
                stream_pipeline.slot(),
            )
            .await
            {
                Ok(capture_worker) => capture_worker,
                Err(error) => {
                    let _ = stream_pipeline.shutdown().await;
                    return Err(error);
                }
            };

            self.capture_sources.insert(
                capture_source_id,
                CaptureSourceContainer::new(capture_worker, stream_id, stream_pipeline),
            );
        } else {
            self.capture_sources
                .get_mut(&capture_source_id)
                .with_context(|| "Capture source container disappeared while adding stream")?
                .add_stream(stream_id, stream_pipeline)
                .await?;
        }

        self.next_stream_id = next_stream_id;

        Ok(stream_id)
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
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
            .remove_stream(stream_id)
            .await?;

        if remove_capture_source {
            let capture_source = self
                .capture_sources
                .remove(&capture_source_id)
                .with_context(|| "Capture source container disappeared before shutdown")?;

            capture_source.shutdown().await?;
        }

        Ok(())
    }
}
