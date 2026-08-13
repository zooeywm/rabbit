pub(crate) mod inbound;
pub(crate) mod outbound_port;

use std::{collections::HashMap, marker::PhantomData};

use crate::{
    app::container::{
        host::inbound::CaptureSourceRuntime,
        host_stream_pipeline::{
            HostStreamPipelineContainer,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
        network::inbound::EncodedUnitSender,
        screen_capture::{ScreenCaptureContainer, outbound_port::ScreenCapturer},
    },
    domain::stream::models::vo::CaptureSourceId,
};

pub(crate) type CapturedFrameFor<CapSt> =
    <ScreenCaptureContainer<CapSt> as ScreenCapturer>::CapturedFrame;

pub(crate) type HostStreamPipelineFor<CvtSt, EcdSt> = HostStreamPipelineContainer<CvtSt, EcdSt>;

pub(crate) type EncoderInputFor<CvtSt, EcdSt> =
    <HostStreamPipelineFor<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput;

pub(crate) type EncodedBufferFor<CvtSt, EcdSt> =
    <HostStreamPipelineFor<CvtSt, EcdSt> as VideoEncoder>::EncodedBuffer;

type CaptureSourceRuntimeFor<CapSt> = CaptureSourceRuntime<ScreenCaptureContainer<CapSt>>;

pub(crate) struct HostContainer<CapSt, CvtSt, EcdSt>
where
    ScreenCaptureContainer<CapSt>: ScreenCapturer,
    HostStreamPipelineFor<CvtSt, EcdSt>: VideoEncoder,
{
    capture_source_runtimes: HashMap<CaptureSourceId, CaptureSourceRuntimeFor<CapSt>>,
    encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
    _pipeline_state_types: PhantomData<fn() -> (CvtSt, EcdSt)>,
}

impl<CapSt, CvtSt, EcdSt> HostContainer<CapSt, CvtSt, EcdSt>
where
    ScreenCaptureContainer<CapSt>: ScreenCapturer,
    HostStreamPipelineFor<CvtSt, EcdSt>: VideoEncoder,
{
    pub(crate) fn new(
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
    ) -> Self {
        Self {
            capture_source_runtimes: HashMap::new(),
            encoded_unit_sender,
            _pipeline_state_types: PhantomData,
        }
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let mut first_error = None;

        for capture_source_runtime in self.capture_source_runtimes.into_values() {
            if let Err(error) = capture_source_runtime.shutdown().await
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
