pub(crate) mod outbound_port;

use std::collections::HashMap;
use std::sync::Arc;

use crate::{
    app::container::client_stream_pipeline::{
        ClientStreamPipelineContainer,
        inbound::{ClientStreamPipelineWorker, ClientStreamPipelineWorkerHandle},
        outbound_port::{DecodedVideoFrame, VideoDecodeUnit, VideoDecoder},
    },
    app::container::network::inbound::{ClientStreamControlSender, NetworkRequestSender},
    domain::stream::models::{
        StreamRequest,
        vo::{CaptureSourceId, StreamId},
    },
};

use self::outbound_port::VideoDecoderState;

pub(crate) type DecoderInputFor<DcdSt> =
    <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecoderInput;
type DecodedFrameFor<DcdSt> =
    DecodedVideoFrame<<ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecodedBuffer>;
type ClientStreamPipelineWorkerHandleFor<DcdSt> =
    ClientStreamPipelineWorkerHandle<DecoderInputFor<DcdSt>, DecodedFrameFor<DcdSt>>;

pub(crate) struct ClientContainer<DcdSt>
where
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    stream_pipelines: HashMap<StreamId, ClientStreamPipelineWorkerHandleFor<DcdSt>>,
    stream_control_sender: ClientStreamControlSender<DecoderInputFor<DcdSt>>,
    network_request_sender: NetworkRequestSender,
    next_stream_id: u16,
}

impl<DcdSt> ClientContainer<DcdSt>
where
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    pub(crate) fn new(
        stream_control_sender: ClientStreamControlSender<DecoderInputFor<DcdSt>>,
        network_request_sender: NetworkRequestSender,
    ) -> Self {
        Self {
            stream_pipelines: HashMap::new(),
            stream_control_sender,
            network_request_sender,
            next_stream_id: 0,
        }
    }

    pub(crate) async fn start_stream(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<StreamId>
    where
        DcdSt: VideoDecoderState,
        DecoderInputFor<DcdSt>: VideoDecodeUnit + Send + 'static,
        <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecodedBuffer: Send + 'static,
        ClientStreamPipelineContainer<DcdSt>: 'static,
    {
        let stream_id = StreamId::new(self.next_stream_id);
        let next_stream_id = self
            .next_stream_id
            .checked_add(1)
            .ok_or_else(|| eros::error!("Stream ID space is exhausted"))?;

        if self.stream_pipelines.contains_key(&stream_id) {
            eros::bail!("Client stream already exists");
        }

        let handle = ClientStreamPipelineWorker::spawn(stream_id).await?;
        let input_sender = handle.input_sender();
        if let Err(error) = self
            .stream_control_sender
            .register(stream_id, input_sender)
            .await
        {
            let _ = handle.shutdown().await;
            return Err(error);
        }
        self.stream_pipelines.insert(stream_id, handle);
        if let Err(error) = self
            .network_request_sender
            .send(StreamRequest::Start {
                capture_source_id,
                stream_id,
            })
            .await
        {
            let _ = self.remove_local_stream(stream_id).await;
            return Err(error);
        }

        self.next_stream_id = next_stream_id;
        Ok(stream_id)
    }

    pub(crate) async fn remove_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let network_result = self
            .network_request_sender
            .send(StreamRequest::Remove { stream_id })
            .await;
        let local_result = self.remove_local_stream(stream_id).await;

        local_result?;
        network_result
    }

    async fn remove_local_stream(&mut self, stream_id: StreamId) -> eros::Result<()> {
        let handle = self
            .stream_pipelines
            .remove(&stream_id)
            .ok_or_else(|| eros::error!("Client stream does not exist"))?;
        let unregister_result = self.stream_control_sender.unregister(stream_id).await;
        let shutdown_result = handle.shutdown().await;

        shutdown_result?;
        unregister_result
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

    pub(crate) async fn shutdown(mut self) -> eros::Result<()> {
        let handles = self.stream_pipelines.drain().collect::<Vec<_>>();
        let mut first_error = None;
        for (stream_id, handle) in handles {
            if let Err(error) = self.stream_control_sender.unregister(stream_id).await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
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

impl outbound_port::VideoDecoderState for () {
    fn new() -> eros::Result<Self> {
        eros::bail!("Client video pipeline is not implemented")
    }
}
