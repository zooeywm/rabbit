use crate::{
    app::container::{
        packetization::{PacketizerContainer, outbound_port::Packetizer},
        root::{
            AppContainer,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                PacketizerManager, PacketizerManagerStateSpec,
            },
        },
        screen_capture::{
            ScreenCaptureContainer,
            outbound_port::{CaptureLoopAction, ScreenCapturer},
        },
        stream_pipeline::{
            StreamPipelineContainer,
            outbound_port::{EncodedVideoFrame, EncoderFrameConverter, VideoEncoder},
        },
    },
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{
        FakeCapturedFrame, FakeCapturerManagerImpl, FakeCapturerManagerState,
        FakeConverterManagerImpl, FakeConverterManagerState, FakeEncoderFrameConverterImpl,
        FakeEncoderFrameConverterState, FakeEncoderInput, FakeEncoderManagerImpl,
        FakeEncoderManagerState, FakePacketizerImpl, FakePacketizerManagerImpl,
        FakePacketizerManagerState, FakePacketizerState, FakeScreenCapturerControl,
        FakeScreenCapturerImpl, FakeScreenCapturerState, FakeVideoEncoderImpl,
        FakeVideoEncoderState,
    },
    infrastructure::support::media::FrameLease,
};

impl CapturerManagerStateSpec for FakeCapturerManagerState {
    type ScreenCapturerState = FakeScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<FakeScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsRef<FakeCapturerManagerState>
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_ref(&self) -> &FakeCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsMut<FakeCapturerManagerState>
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_mut(&mut self) -> &mut FakeCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> CapturerManager
    for AppContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    type State = FakeCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt, PktMgrSt> {
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

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsRef<FakeConverterManagerState>
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsMut<FakeConverterManagerState>
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> ConverterManager
    for AppContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt, PktMgrSt>
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
    + use<CapMgrSt, EcdMgrSt, PktMgrSt> {
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

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsRef<FakeEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsMut<FakeEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakeEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, PktMgrSt> {
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
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        VideoEncoder::encode(FakeVideoEncoderImpl::inj_ref_mut(self), input)
    }
}

impl PacketizerManagerStateSpec for FakePacketizerManagerState {
    type PacketizerState = FakePacketizerState;
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsRef<FakePacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, FakePacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakePacketizerManagerState {
        self.packetizer_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsMut<FakePacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, FakePacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakePacketizerManagerState {
        self.packetizer_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> PacketizerManager
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, FakePacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakePacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, EcdMgrSt> {
        PacketizerManager::compose_packetizer_state(FakePacketizerManagerImpl::inj_ref_mut(self))
    }
}

impl AsMut<FakePacketizerState> for PacketizerContainer<FakePacketizerState> {
    fn as_mut(&mut self) -> &mut FakePacketizerState {
        self.state_mut()
    }
}

impl Packetizer for PacketizerContainer<FakePacketizerState> {
    type EncodedBuffer = [u8; 8];

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        Packetizer::packetize(FakePacketizerImpl::inj_ref_mut(self), frame)
    }
}

pub(super) type PlatformApp = AppContainer<
    FakeCapturerManagerState,
    FakeConverterManagerState,
    FakeEncoderManagerState,
    FakePacketizerManagerState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            FakeCapturerManagerState::new()?,
            FakeConverterManagerState::new()?,
            FakeEncoderManagerState::new()?,
            FakePacketizerManagerState::new()?,
        ))
    }
}
