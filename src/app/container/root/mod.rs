mod inbound;

pub(crate) mod outbound_port;

use std::collections::HashMap;

use crate::{
    app::container::{
        root::outbound_port::{
            CapturerManagerStateSpec, ConverterManagerStateSpec, EncoderManagerStateSpec,
            TransporterConstructor, TransporterConstructorStateSpec,
        },
        screen_capture::outbound_port::ScreenCapturer,
        stream_pipeline::{
            StreamPipelineContainer,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
    },
    domain::stream::models::vo::CaptureSourceId,
};
use inbound::CaptureSourceRuntime;

pub(crate) type CapturedFrameFor<CapMgrSt> =
    <<CapMgrSt as CapturerManagerStateSpec>::ScreenCapturer as ScreenCapturer>::CapturedFrame;

pub(crate) type StreamPipelineFor<CvtMgrSt, EcdMgrSt> = StreamPipelineContainer<
    <CvtMgrSt as ConverterManagerStateSpec>::EncoderFrameConverterState,
    <EcdMgrSt as EncoderManagerStateSpec>::VideoEncoderState,
>;

pub(crate) type EncoderInputFor<CvtMgrSt, EcdMgrSt> =
    <StreamPipelineFor<CvtMgrSt, EcdMgrSt> as EncoderFrameConverter>::EncoderInput;

pub(crate) type EncodedBufferFor<CvtMgrSt, EcdMgrSt> =
    <StreamPipelineFor<CvtMgrSt, EcdMgrSt> as VideoEncoder>::EncodedBuffer;

pub(crate) type TransporterStateFor<TprCstSt> =
    <TprCstSt as TransporterConstructorStateSpec>::TransporterState;

type CaptureSourceRuntimeFor<CapMgrSt> =
    CaptureSourceRuntime<<CapMgrSt as CapturerManagerStateSpec>::ScreenCapturer>;

pub(crate) struct AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    capturer_manager_state: CapMgrSt,
    converter_manager_state: CvtMgrSt,
    encoder_manager_state: EcdMgrSt,
    transporter_constructor_state: TprCstSt,
    capture_source_runtimes: HashMap<CaptureSourceId, CaptureSourceRuntimeFor<CapMgrSt>>,
    next_stream_id: u16,
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt> AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    pub(crate) fn new(
        capturer_manager_state: CapMgrSt,
        converter_manager_state: CvtMgrSt,
        encoder_manager_state: EcdMgrSt,
        transporter_constructor: TprCstSt,
    ) -> Self {
        Self {
            capturer_manager_state,
            converter_manager_state,
            encoder_manager_state,
            transporter_constructor_state: transporter_constructor,
            capture_source_runtimes: HashMap::new(),
            next_stream_id: 0,
        }
    }

    pub(crate) fn capturer_manager_state(&self) -> &CapMgrSt {
        &self.capturer_manager_state
    }

    pub(crate) fn capturer_manager_state_mut(&mut self) -> &mut CapMgrSt {
        &mut self.capturer_manager_state
    }

    pub(crate) fn converter_manager_state(&self) -> &CvtMgrSt {
        &self.converter_manager_state
    }

    pub(crate) fn converter_manager_state_mut(&mut self) -> &mut CvtMgrSt {
        &mut self.converter_manager_state
    }

    pub(crate) fn encoder_manager_state(&self) -> &EcdMgrSt {
        &self.encoder_manager_state
    }

    pub(crate) fn encoder_manager_state_mut(&mut self) -> &mut EcdMgrSt {
        &mut self.encoder_manager_state
    }

    pub(crate) fn transporter_constructor_state(&self) -> &TprCstSt {
        &self.transporter_constructor_state
    }

    pub(in crate::app) fn compose_transporter(
        &self,
    ) -> eros::Result<
        impl FnOnce() -> eros::Result<TransporterStateFor<TprCstSt>>
        + Send
        + 'static
        + use<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>,
    >
    where
        TprCstSt: TransporterConstructorStateSpec,
        Self: TransporterConstructor<State = TprCstSt>,
    {
        TransporterConstructor::compose_transporter_state(self)
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
