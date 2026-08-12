pub(crate) mod inbound_port;
pub(crate) mod outbound_port;

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    app::container::client_stream_pipeline::{
        ClientStreamPipelineContainer,
        inbound::{ClientStreamPipelineWorker, ClientStreamPipelineWorkerHandle, DecodeUnitSender},
        outbound_port::{DecodedVideoFrame, VideoDecodeUnit, VideoDecoder},
    },
    domain::stream::models::vo::StreamId,
};

use self::outbound_port::{ClientEventReporter, DecoderManager};

type DecoderInputFor<DcdSt> = <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecoderInput;
type DecodedFrameFor<DcdSt> =
    DecodedVideoFrame<<ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecodedBuffer>;
type ClientStreamPipelineWorkerHandleFor<DcdSt> =
    ClientStreamPipelineWorkerHandle<DecoderInputFor<DcdSt>, DecodedFrameFor<DcdSt>>;

pub(crate) struct ClientContainer<DcdMgrSt, DcdSt = ()>
where
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    decoder_manager_state: DcdMgrSt,
    stream_pipelines: HashMap<StreamId, ClientStreamPipelineWorkerHandleFor<DcdSt>>,
}

impl<DcdMgrSt, DcdSt> ClientContainer<DcdMgrSt, DcdSt>
where
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    pub(crate) fn new(decoder_manager_state: DcdMgrSt) -> Self {
        Self {
            decoder_manager_state,
            stream_pipelines: HashMap::new(),
        }
    }

    pub(crate) fn decoder_manager_state(&self) -> &DcdMgrSt {
        &self.decoder_manager_state
    }

    pub(crate) fn decoder_manager_state_mut(&mut self) -> &mut DcdMgrSt {
        &mut self.decoder_manager_state
    }

    pub(crate) async fn start_stream<EventReporter>(
        &mut self,
        stream_id: StreamId,
        event_reporter: EventReporter,
    ) -> eros::Result<DecodeUnitSender<DecoderInputFor<DcdSt>>>
    where
        Self: DecoderManager,
        DecoderInputFor<DcdSt>: VideoDecodeUnit + Send + 'static,
        <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecodedBuffer: Send + 'static,
        ClientStreamPipelineContainer<DcdSt>: 'static,
        <Self as DecoderManager>::State:
            outbound_port::DecoderManagerStateSpec<VideoDecoderState = DcdSt>,
        EventReporter: ClientEventReporter,
    {
        if self.stream_pipelines.contains_key(&stream_id) {
            eros::bail!("Client stream already exists");
        }

        let decoder_constructor = self.compose_video_decoder_state();
        let handle =
            ClientStreamPipelineWorker::spawn(stream_id, decoder_constructor, event_reporter)
                .await?;
        let input_sender = handle.input_sender();
        self.stream_pipelines.insert(stream_id, handle);
        Ok(input_sender)
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let handle = self
            .stream_pipelines
            .remove(&stream_id)
            .ok_or_else(|| eros::error!("Client stream does not exist"))?;
        handle.shutdown().await
    }

    pub(crate) fn decoded_frame_slot(
        &self,
        stream_id: StreamId,
    ) -> Option<
        Arc<
            crate::app::container::client_stream_pipeline::inbound::LatestDecodedFrameSlot<
                DecodedFrameFor<DcdSt>,
            >,
        >,
    > {
        self.stream_pipelines
            .get(&stream_id)
            .map(ClientStreamPipelineWorkerHandle::decoded_frame_slot)
    }

    pub(crate) async fn handle_worker_exit(
        &mut self,
        stream_id: StreamId,
    ) -> Option<eros::ErrorUnion> {
        let handle = self.stream_pipelines.remove(&stream_id)?;
        match handle.shutdown().await {
            Ok(()) => Some(eros::error!(
                "Client stream pipeline worker exited unexpectedly"
            )),
            Err(error) => Some(error),
        }
    }

    pub(crate) async fn shutdown(mut self) -> eros::Result<()> {
        let handles = self
            .stream_pipelines
            .drain()
            .map(|(_, handle)| handle)
            .collect::<Vec<_>>();
        let mut first_error = None;
        for handle in handles {
            if let Err(error) = handle.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}
