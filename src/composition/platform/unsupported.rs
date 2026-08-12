use std::convert::Infallible;

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
        UnsupportedCapturerManagerImpl, UnsupportedCapturerManagerState,
        UnsupportedConverterManagerImpl, UnsupportedConverterManagerState,
        UnsupportedEncoderFrameConverterState, UnsupportedEncoderManagerImpl,
        UnsupportedEncoderManagerState, UnsupportedScreenCapturerImpl,
        UnsupportedScreenCapturerState, UnsupportedTransporterConstructorImpl,
        UnsupportedTransporterConstructorState, UnsupportedTransporterImpl,
        UnsupportedTransporterState, UnsupportedVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for UnsupportedCapturerManagerState {
    type ScreenCapturerState = UnsupportedScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<UnsupportedScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt> AsRef<UnsupportedCapturerManagerState>
    for HostContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_ref(&self) -> &UnsupportedCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<UnsupportedCapturerManagerState>
    for HostContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_mut(&mut self) -> &mut UnsupportedCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for HostContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    type State = UnsupportedCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt> {
        CapturerManager::compose_screen_capturer_state(
            UnsupportedCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl ScreenCapturer for ScreenCaptureContainer<UnsupportedScreenCapturerState> {
    type CapturedFrame = Infallible;
    type Control = Infallible;

    fn control(&self) -> eros::Result<Self::Control> {
        ScreenCapturer::control(UnsupportedScreenCapturerImpl::inj_ref(self))
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
            UnsupportedScreenCapturerImpl::inj_ref_mut(self),
            initial_consumer_count,
            on_started,
            on_control,
            on_frame,
        )
    }
}

impl ConverterManagerStateSpec for UnsupportedConverterManagerState {
    type EncoderFrameConverterState = UnsupportedEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt> AsRef<UnsupportedConverterManagerState>
    for HostContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<UnsupportedConverterManagerState>
    for HostContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for HostContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = UnsupportedConverterManagerState;

    fn compose_encoder_frame_converter_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<CapMgrSt, EcdMgrSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            UnsupportedConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<UnsupportedEncoderFrameConverterState>
    for HostStreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &UnsupportedEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<UnsupportedEncoderFrameConverterState>
    for HostStreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for HostStreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
{
    type CapturedFrame = Infallible;
    type EncoderInput = Infallible;

    fn convert(&mut self, frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput> {
        match frame {}
    }
}

impl EncoderManagerStateSpec for UnsupportedEncoderManagerState {
    type VideoEncoderState = UnsupportedVideoEncoderState;
}

impl<CapMgrSt, CvtMgrSt> AsRef<UnsupportedEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<UnsupportedEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for HostContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = UnsupportedEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt> {
        EncoderManager::compose_video_encoder_state(UnsupportedEncoderManagerImpl::inj_ref_mut(
            self,
        ))
    }
}

impl<CvtSt> AsRef<UnsupportedVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState>
{
    fn as_ref(&self) -> &UnsupportedVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<UnsupportedVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState>
{
    fn as_mut(&mut self) -> &mut UnsupportedVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for HostStreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState> {
    type EncoderInput = Infallible;
    type EncodedBuffer = Infallible;

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoUnit<Self::EncodedBuffer>> {
        match input {}
    }
}

impl TransporterConstructorStateSpec for UnsupportedTransporterConstructorState {
    type TransporterState = UnsupportedTransporterState;
}

impl<Host, Client> AsRef<UnsupportedTransporterConstructorState>
    for AppContainer<Host, Client, UnsupportedTransporterConstructorState>
{
    fn as_ref(&self) -> &UnsupportedTransporterConstructorState {
        self.network_constructor_state()
    }
}

impl<Host, Client> TransporterConstructor
    for AppContainer<Host, Client, UnsupportedTransporterConstructorState>
{
    type State = UnsupportedTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<Host, Client>,
    > {
        TransporterConstructor::compose_transporter_state(
            UnsupportedTransporterConstructorImpl::inj_ref(self),
        )
    }
}

pub(super) type PlatformHost = HostContainer<
    UnsupportedCapturerManagerState,
    UnsupportedConverterManagerState,
    UnsupportedEncoderManagerState,
>;
pub(super) type PlatformClient = ClientContainer;
pub(super) type PlatformNetworkConstructorState = UnsupportedTransporterConstructorState;
pub(super) type PlatformApp =
    AppContainer<PlatformHost, PlatformClient, PlatformNetworkConstructorState>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            PlatformHost::new(
                UnsupportedCapturerManagerState::new()?,
                UnsupportedConverterManagerState::new()?,
                UnsupportedEncoderManagerState::new()?,
            ),
            PlatformClient::new(),
            UnsupportedTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<UnsupportedTransporterState> for NetworkContainer<UnsupportedTransporterState> {
    fn as_mut(&mut self) -> &mut UnsupportedTransporterState {
        self.state_mut()
    }
}

impl Transporter for NetworkContainer<UnsupportedTransporterState> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;

    fn packetize(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        Transporter::packetize(
            UnsupportedTransporterImpl::inj_ref_mut(self),
            stream_id,
            unit,
        )
    }

    async fn send(&mut self, packetized: Self::Packetized) -> eros::Result<()> {
        Transporter::send(UnsupportedTransporterImpl::inj_ref_mut(self), packetized).await
    }
}
