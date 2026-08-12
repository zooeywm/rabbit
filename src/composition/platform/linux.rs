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
        network::{
            NetworkContainer,
            outbound_port::{TransporterClientSide, TransporterHostSide},
        },
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

impl<CvtMgrSt, EcdMgrSt> AsRef<LinuxCapturerManagerState>
    for HostContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_ref(&self) -> &LinuxCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<LinuxCapturerManagerState>
    for HostContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    fn as_mut(&mut self) -> &mut LinuxCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for HostContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
{
    type State = LinuxCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt> {
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

impl<CapMgrSt, EcdMgrSt> AsRef<LinuxConverterManagerState>
    for HostContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<LinuxConverterManagerState>
    for HostContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for HostContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
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
    + use<CapMgrSt, EcdMgrSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            LinuxConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<LinuxEncoderFrameConverterState>
    for HostStreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &LinuxEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<LinuxEncoderFrameConverterState>
    for HostStreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut LinuxEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for HostStreamPipelineContainer<LinuxEncoderFrameConverterState, EcdSt>
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

impl<CapMgrSt, CvtMgrSt> AsRef<LinuxEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<LinuxEncoderManagerState>
    for HostContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for HostContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt> {
        EncoderManager::compose_video_encoder_state(LinuxEncoderManagerImpl::inj_ref_mut(self))
    }
}

impl<CvtSt> AsRef<LinuxVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, LinuxVideoEncoderState>
{
    fn as_ref(&self) -> &LinuxVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<LinuxVideoEncoderState>
    for HostStreamPipelineContainer<CvtSt, LinuxVideoEncoderState>
{
    fn as_mut(&mut self) -> &mut LinuxVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for HostStreamPipelineContainer<CvtSt, LinuxVideoEncoderState> {
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

impl<Host, Client> AsRef<LinuxTransporterConstructorState>
    for AppContainer<Host, Client, LinuxTransporterConstructorState>
{
    fn as_ref(&self) -> &LinuxTransporterConstructorState {
        self.network_constructor_state()
    }
}

impl<Host, Client> TransporterConstructor
    for AppContainer<Host, Client, LinuxTransporterConstructorState>
{
    type State = LinuxTransporterConstructorState;

    fn compose_transporter_state(
        &self,
    ) -> eros::Result<
        impl FnOnce()
            -> eros::Result<<Self::State as TransporterConstructorStateSpec>::TransporterState>
        + Send
        + 'static
        + use<Host, Client>,
    > {
        TransporterConstructor::compose_transporter_state(LinuxTransporterConstructorImpl::inj_ref(
            self,
        ))
    }
}

pub(super) type PlatformHost =
    HostContainer<LinuxCapturerManagerState, LinuxConverterManagerState, LinuxEncoderManagerState>;
pub(super) type PlatformClient = ClientContainer<()>;
pub(super) type PlatformNetworkConstructorState = LinuxTransporterConstructorState;
pub(super) type PlatformApp =
    AppContainer<PlatformHost, PlatformClient, PlatformNetworkConstructorState>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            PlatformHost::new(
                LinuxCapturerManagerState::new()?,
                LinuxConverterManagerState::new()?,
                LinuxEncoderManagerState::new()?,
            ),
            PlatformClient::new(()),
            LinuxTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<LinuxTransporterState> for NetworkContainer<LinuxTransporterState> {
    fn as_mut(&mut self) -> &mut LinuxTransporterState {
        self.state_mut()
    }
}

impl TransporterHostSide for NetworkContainer<LinuxTransporterState> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;
    type Sender = Infallible;

    fn take_sender(&mut self) -> eros::Result<Self::Sender> {
        TransporterHostSide::take_sender(LinuxTransporterImpl::inj_ref_mut(self))
    }

    fn packetize(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        TransporterHostSide::packetize(LinuxTransporterImpl::inj_ref_mut(self), stream_id, unit)
    }

    async fn send(_sender: &mut Self::Sender, packetized: Self::Packetized) -> eros::Result<()> {
        match packetized {}
    }
}

impl TransporterClientSide for NetworkContainer<LinuxTransporterState> {
    type Receiver = Infallible;
    type Received = Infallible;
    type Depacketized = Infallible;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        TransporterClientSide::take_receiver(LinuxTransporterImpl::inj_ref_mut(self))
    }

    async fn receive(_receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        eros::bail!("Linux transporter is not implemented")
    }

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(
        crate::domain::stream::models::vo::StreamId,
        Self::Depacketized,
    )> {
        match received {}
    }
}
