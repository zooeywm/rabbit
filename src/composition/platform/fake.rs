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
        FakeCapturedFrame, FakeCapturerManagerImpl, FakeCapturerManagerState,
        FakeConverterManagerImpl, FakeConverterManagerState, FakeEncoderFrameConverterImpl,
        FakeEncoderFrameConverterState, FakeEncoderInput, FakeEncoderManagerImpl,
        FakeEncoderManagerState, FakePacketized, FakeScreenCapturerControl, FakeScreenCapturerImpl,
        FakeScreenCapturerState, FakeTransporterConstructorImpl, FakeTransporterConstructorState,
        FakeTransporterImpl, FakeTransporterState, FakeVideoEncoderImpl, FakeVideoEncoderState,
    },
    infrastructure::support::media::FrameLease,
};

impl CapturerManagerStateSpec for FakeCapturerManagerState {
    type ScreenCapturerState = FakeScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<FakeScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> AsRef<FakeCapturerManagerState>
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    fn as_ref(&self) -> &FakeCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> AsMut<FakeCapturerManagerState>
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    fn as_mut(&mut self) -> &mut FakeCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt, TprCstSt> CapturerManager
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, TprCstSt>
{
    type State = FakeCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt, TprCstSt> {
        CapturerManager::compose_screen_capturer_state(
            FakeCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl AsMut<FakeScreenCapturerState> for ScreenCaptureContainer<FakeScreenCapturerState> {
    fn as_mut(&mut self) -> &mut FakeScreenCapturerState {
        self.state_mut()
    }
}

impl AsRef<FakeScreenCapturerState> for ScreenCaptureContainer<FakeScreenCapturerState> {
    fn as_ref(&self) -> &FakeScreenCapturerState {
        self.state()
    }
}

impl ScreenCapturer for ScreenCaptureContainer<FakeScreenCapturerState> {
    type CapturedFrame = FrameLease<FakeCapturedFrame>;
    type Control = FakeScreenCapturerControl;

    fn control(&self) -> eros::Result<Self::Control> {
        ScreenCapturer::control(FakeScreenCapturerImpl::inj_ref(self))
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
            FakeScreenCapturerImpl::inj_ref_mut(self),
            initial_consumer_count,
            on_started,
            on_control,
            on_frame,
        )
    }
}

impl ConverterManagerStateSpec for FakeConverterManagerState {
    type EncoderFrameConverterState = FakeEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> AsRef<FakeConverterManagerState>
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> AsMut<FakeConverterManagerState>
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt, TprCstSt> ConverterManager
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakeConverterManagerState;

    fn compose_encoder_frame_converter_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<CapMgrSt, EcdMgrSt, TprCstSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            FakeConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<FakeEncoderFrameConverterState>
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<FakeEncoderFrameConverterState>
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;
    type EncoderInput = FakeEncoderInput;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput> {
        EncoderFrameConverter::convert(FakeEncoderFrameConverterImpl::inj_ref_mut(self), frame)
    }
}

impl EncoderManagerStateSpec for FakeEncoderManagerState {
    type VideoEncoderState = FakeVideoEncoderState;
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> AsRef<FakeEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> AsMut<FakeEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, TprCstSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, TprCstSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakeEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, TprCstSt> {
        EncoderManager::compose_video_encoder_state(FakeEncoderManagerImpl::inj_ref_mut(self))
    }
}

impl<CvtSt> AsRef<FakeVideoEncoderState> for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    fn as_ref(&self) -> &FakeVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<FakeVideoEncoderState> for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    fn as_mut(&mut self) -> &mut FakeVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    type EncoderInput = FakeEncoderInput;
    type EncodedBuffer = [u8; 8];

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoUnit<Self::EncodedBuffer>> {
        VideoEncoder::encode(FakeVideoEncoderImpl::inj_ref_mut(self), input)
    }
}

impl TransporterConstructorStateSpec for FakeTransporterConstructorState {
    type TransporterState = FakeTransporterState;
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsRef<FakeTransporterConstructorState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, FakeTransporterConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeTransporterConstructorState {
        self.transporter_constructor_state()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> TransporterConstructor
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, FakeTransporterConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakeTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<CapMgrSt, CvtMgrSt, EcdMgrSt>,
    > {
        TransporterConstructor::compose_transporter_state(FakeTransporterConstructorImpl::inj_ref(
            self,
        ))
    }
}

pub(super) type PlatformApp = AppContainer<
    FakeCapturerManagerState,
    FakeConverterManagerState,
    FakeEncoderManagerState,
    FakeTransporterConstructorState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            FakeCapturerManagerState::new()?,
            FakeConverterManagerState::new()?,
            FakeEncoderManagerState::new()?,
            FakeTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<FakeTransporterState> for TransporterContainer<FakeTransporterState> {
    fn as_mut(&mut self) -> &mut FakeTransporterState {
        self.state_mut()
    }
}

impl Transporter for TransporterContainer<FakeTransporterState> {
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;

    fn packetize(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        Transporter::packetize(FakeTransporterImpl::inj_ref_mut(self), stream_id, unit)
    }

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()> {
        Transporter::send(FakeTransporterImpl::inj_ref_mut(self), packetized).await
    }
}
