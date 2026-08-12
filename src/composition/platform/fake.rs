use crate::{
    app::container::{
        client::ClientContainer,
        host::{
            HostContainer,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
            },
        },
        host_stream_pipeline::{
            HostStreamPipelineContainer,
            outbound_port::{EncodedVideoUnit, EncoderFrameConverter, VideoEncoder},
        },
        network::{NetworkContainer, outbound_port::Transporter},
        root::{
            AppContainer,
            outbound_port::{TransporterConstructor, TransporterConstructorStateSpec},
        },
        screen_capture::{
            ScreenCaptureContainer,
            outbound_port::{CaptureLoopAction, ScreenCapturer},
        },
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

impl<CvtMgrSt, EcdMgrSt> AsRef<FakeCapturerManagerState>
    for HostContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_ref(&self) -> &FakeCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<FakeCapturerManagerState>
    for HostContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_mut(&mut self) -> &mut FakeCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for HostContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    type State = FakeCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt> {
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

impl<CapMgrSt, EcdMgrSt> AsRef<FakeConverterManagerState>
    for HostContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<FakeConverterManagerState>
    for HostContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for HostContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
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
    + use<CapMgrSt, EcdMgrSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            FakeConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<FakeEncoderFrameConverterState>
    for HostStreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<FakeEncoderFrameConverterState>
    for HostStreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for HostStreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
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

impl<CapMgrSt, CvtMgrSt> AsRef<FakeEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &FakeEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<FakeEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for HostContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = FakeEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt> {
        EncoderManager::compose_video_encoder_state(FakeEncoderManagerImpl::inj_ref_mut(self))
    }
}

impl<CvtSt> AsRef<FakeVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, FakeVideoEncoderState>
{
    fn as_ref(&self) -> &FakeVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<FakeVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, FakeVideoEncoderState>
{
    fn as_mut(&mut self) -> &mut FakeVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for HostStreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
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

impl<Host, Client> AsRef<FakeTransporterConstructorState>
    for AppContainer<Host, Client, FakeTransporterConstructorState>
{
    fn as_ref(&self) -> &FakeTransporterConstructorState {
        self.network_constructor_state()
    }
}

impl<Host, Client> TransporterConstructor
    for AppContainer<Host, Client, FakeTransporterConstructorState>
{
    type State = FakeTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<Host, Client>,
    > {
        TransporterConstructor::compose_transporter_state(FakeTransporterConstructorImpl::inj_ref(
            self,
        ))
    }
}

pub(super) type PlatformHost =
    HostContainer<FakeCapturerManagerState, FakeConverterManagerState, FakeEncoderManagerState>;
pub(super) type PlatformClient = ClientContainer;
pub(super) type PlatformNetworkConstructorState = FakeTransporterConstructorState;
pub(super) type PlatformApp =
    AppContainer<PlatformHost, PlatformClient, PlatformNetworkConstructorState>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            PlatformHost::new(
                FakeCapturerManagerState::new()?,
                FakeConverterManagerState::new()?,
                FakeEncoderManagerState::new()?,
            ),
            PlatformClient::new(),
            FakeTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<FakeTransporterState> for NetworkContainer<FakeTransporterState> {
    fn as_mut(&mut self) -> &mut FakeTransporterState {
        self.state_mut()
    }
}

impl Transporter for NetworkContainer<FakeTransporterState> {
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
