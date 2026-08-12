pub(crate) mod inbound_port;
pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{
    app::container::client_stream_pipeline::{
        ClientStreamPipelineContainer, outbound_port::VideoDecoder,
    },
    domain::stream::models::vo::StreamId,
};

use self::outbound_port::DecoderManager;

pub(crate) struct ClientContainer<DcdMgrSt, DcdSt = ()> {
    decoder_manager_state: DcdMgrSt,
    stream_pipelines: HashMap<StreamId, ClientStreamPipelineContainer<DcdSt>>,
}

impl<DcdMgrSt, DcdSt> ClientContainer<DcdMgrSt, DcdSt> {
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

    pub(crate) fn decode_network_input<Input>(
        &mut self,
        stream_id: StreamId,
        input: Input,
    ) -> eros::Result<()>
    where
        Self: DecoderManager,
        ClientStreamPipelineContainer<DcdSt>: VideoDecoder<DecoderInput = Input>,
        <Self as DecoderManager>::State:
            outbound_port::DecoderManagerStateSpec<VideoDecoderState = DcdSt>,
    {
        if !self.stream_pipelines.contains_key(&stream_id) {
            let compose_decoder = self.compose_video_decoder_state();
            let decoder_state = compose_decoder()?;
            self.stream_pipelines
                .insert(stream_id, ClientStreamPipelineContainer::new(decoder_state));
        }

        let pipeline = self
            .stream_pipelines
            .get_mut(&stream_id)
            .expect("client stream pipeline should exist after insertion");
        pipeline.decode(input)?;
        Ok(())
    }
}
