use std::convert::Infallible;

use crate::{
    app::container::{
        root::{
            AppContainer,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                TransporterConstructor, TransporterConstructorStateSpec,
            },
        },
        screen_capture::{
            ScreenCaptureContainer,
            outbound_port::{CaptureLoopAction, ScreenCapturer},
        },
        stream_pipeline::{
            StreamPipelineContainer,
            outbound_port::{EncodedVideoUnit, EncoderFrameConverter, VideoEncoder},
        },
        transporter::{TransporterContainer, outbound_port::Transporter},
    },
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{
        LinuxCapturerManagerImpl, LinuxCapturerManagerState, LinuxConverterManagerImpl,
        LinuxConverterManagerState, LinuxEncoderFrameConverterState, LinuxEncoderManagerImpl,
        LinuxEncoderManagerState, LinuxScreenCapturerImpl, LinuxScreenCapturerState,
        LinuxTransporterConstructorImpl, LinuxTransporterConstructorState, LinuxTransporterImpl,
        LinuxTransporterState, LinuxVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for LinuxCapturerManagerState {
    type ScreenCapturerState = LinuxScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<LinuxScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> AsRef<LinuxCapturerManagerState>
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    fn as_ref(&self) -> &LinuxCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> AsMut<LinuxCapturerManagerState>
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    fn as_mut(&mut self) -> &mut LinuxCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> CapturerManager
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    type State = LinuxCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt, TprCstSt> {
        CapturerManager::compose_screen_capturer_state(
            LinuxCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl ScreenCapturer for ScreenCaptureContainer<LinuxScreenCapturerState> {
    type CapturedFrame = Infallible;
    type Control = Infallible;

    fn control(&self) -> eros::Result<Self::Control> {
        ScreenCapturer::control(LinuxScreenCapturerImpl::inj_ref(self))
    }

    fn run<OnStarted, OnControl, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        on_control: OnControl,
        on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        ScreenCapturer::run(
            LinuxScreenCapturerImpl::inj_ref_mut(self),
            initial_consumer_count,
            on_started,
            on_control,
            on_frame,
        )
    }
}

impl ConverterManagerStateSpec for LinuxConverterManagerState {
    type EncoderFrameConverterState = LinuxEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> AsRef<LinuxConverterManagerState>
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> AsMut<LinuxConverterManagerState>
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> ConverterManager
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxConverterManagerState;

    fn compose_encoder_frame_converter_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<CapMgrSt, EcdMgrSt, TprCstSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            LinuxConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<LinuxEncoderFrameConverterState>
    for StreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &LinuxEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<LinuxEncoderFrameConverterState>
    for StreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut LinuxEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for StreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
{
    type CapturedFrame = Infallible;
    type EncoderInput = Infallible;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput> {
        match frame {}
    }
}

impl EncoderManagerStateSpec for LinuxEncoderManagerState {
    type VideoEncoderState = LinuxVideoEncoderState;
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> AsRef<LinuxEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> AsMut<LinuxEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, TprCstSt> {
        EncoderManager::compose_video_encoder_state(LinuxEncoderManagerImpl::inj_ref_mut(self))
    }
}

impl<CvtSt> AsRef<LinuxVideoEncoderState>
    for StreamPipelineContainer<CvtSt, LinuxVideoEncoderState>
{
    fn as_ref(&self) -> &LinuxVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<LinuxVideoEncoderState>
    for StreamPipelineContainer<CvtSt, LinuxVideoEncoderState>
{
    fn as_mut(&mut self) -> &mut LinuxVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for StreamPipelineContainer<CvtSt, LinuxVideoEncoderState> {
    type EncoderInput = Infallible;
    type EncodedBuffer = Infallible;

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoUnit<Self::EncodedBuffer>> {
        match input {}
    }
}

impl TransporterConstructorStateSpec for LinuxTransporterConstructorState {
    type TransporterState = LinuxTransporterState;
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsRef<LinuxTransporterConstructorState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, LinuxTransporterConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxTransporterConstructorState {
        self.transporter_constructor_state()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> TransporterConstructor
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, LinuxTransporterConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<CapMgrSt, CvtMgrSt, EcdMgrSt>,
    > {
        TransporterConstructor::compose_transporter_state(LinuxTransporterConstructorImpl::inj_ref(
            self,
        ))
    }
}

pub(super) type PlatformApp = AppContainer<
    LinuxCapturerManagerState,
    LinuxConverterManagerState,
    LinuxEncoderManagerState,
    LinuxTransporterConstructorState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            LinuxCapturerManagerState::new()?,
            LinuxConverterManagerState::new()?,
            LinuxEncoderManagerState::new()?,
            LinuxTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<LinuxTransporterState> for TransporterContainer<LinuxTransporterState> {
    fn as_mut(&mut self) -> &mut LinuxTransporterState {
        self.state_mut()
    }
}

impl Transporter for TransporterContainer<LinuxTransporterState> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;

    fn packetize(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        Transporter::packetize(LinuxTransporterImpl::inj_ref_mut(self), stream_id, unit)
    }

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()> {
        Transporter::send(LinuxTransporterImpl::inj_ref_mut(self), packetized).await
    }
}
