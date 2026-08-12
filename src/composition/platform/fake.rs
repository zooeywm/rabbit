use crate::{
    app::container::{
        client::{
            ClientContainer,
            inbound_port::ClientApplication,
            outbound_port::{DecoderManager, DecoderManagerStateSpec},
        },
        client_stream_pipeline::{ClientStreamPipelineContainer, outbound_port::VideoDecoder},
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
        FakeCapturedFrame, FakeCapturerManagerImpl, FakeCapturerManagerState,
        FakeConverterManagerImpl, FakeConverterManagerState, FakeDecoderInput,
        FakeDecoderManagerImpl, FakeDecoderManagerState, FakeEncoderFrameConverterImpl,
        FakeEncoderFrameConverterState, FakeEncoderInput, FakeEncoderManagerImpl,
        FakeEncoderManagerState, FakePacketized, FakeScreenCapturerControl, FakeScreenCapturerImpl,
        FakeScreenCapturerState, FakeTransporterConstructorImpl, FakeTransporterConstructorState,
        FakeTransporterImpl, FakeTransporterReceiver, FakeTransporterSender, FakeTransporterState,
        FakeVideoDecoderImpl, FakeVideoDecoderState, FakeVideoEncoderImpl, FakeVideoEncoderState,
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

impl DecoderManagerStateSpec for FakeDecoderManagerState {
    type VideoDecoderState = FakeVideoDecoderState;
}

impl<DcdSt> AsRef<FakeDecoderManagerState> for ClientContainer<FakeDecoderManagerState, DcdSt> {
    fn as_ref(&self) -> &FakeDecoderManagerState {
        self.decoder_manager_state()
    }
}

impl<DcdSt> AsMut<FakeDecoderManagerState> for ClientContainer<FakeDecoderManagerState, DcdSt> {
    fn as_mut(&mut self) -> &mut FakeDecoderManagerState {
        self.decoder_manager_state_mut()
    }
}

impl<DcdSt> DecoderManager for ClientContainer<FakeDecoderManagerState, DcdSt> {
    type State = FakeDecoderManagerState;

    fn compose_video_decoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as DecoderManagerStateSpec>::VideoDecoderState>
    + Send
    + 'static
    + use<DcdSt> {
        DecoderManager::compose_video_decoder_state(FakeDecoderManagerImpl::inj_ref_mut(self))
    }
}

impl AsRef<FakeVideoDecoderState> for ClientStreamPipelineContainer<FakeVideoDecoderState> {
    fn as_ref(&self) -> &FakeVideoDecoderState {
        self.video_decoder_state()
    }
}

impl AsMut<FakeVideoDecoderState> for ClientStreamPipelineContainer<FakeVideoDecoderState> {
    fn as_mut(&mut self) -> &mut FakeVideoDecoderState {
        self.video_decoder_state_mut()
    }
}

impl VideoDecoder for ClientStreamPipelineContainer<FakeVideoDecoderState> {
    type DecoderInput = FakeDecoderInput;
    type DecodedBuffer = [u8; 8];

    fn decode(
        &mut self,
        input: Self::DecoderInput,
    ) -> eros::Result<
        crate::app::container::client_stream_pipeline::outbound_port::DecodedVideoFrame<
            Self::DecodedBuffer,
        >,
    > {
        VideoDecoder::decode(FakeVideoDecoderImpl::inj_ref_mut(self), input)
    }
}

impl ClientApplication for ClientContainer<FakeDecoderManagerState, FakeVideoDecoderState> {
    type NetworkInput = FakeDecoderInput;

    fn handle_network_input(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        input: Self::NetworkInput,
    ) -> eros::Result<()> {
        self.decode_network_input(stream_id, input)
    }

    async fn shutdown(self) -> eros::Result<()> {
        Ok(())
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
pub(super) type PlatformClient = ClientContainer<FakeDecoderManagerState, FakeVideoDecoderState>;
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
            PlatformClient::new(FakeDecoderManagerState::new()?),
            FakeTransporterConstructorState::new()?,
        ))
    }
}

impl AsMut<FakeTransporterState> for NetworkContainer<FakeTransporterState> {
    fn as_mut(&mut self) -> &mut FakeTransporterState {
        self.state_mut()
    }
}

impl TransporterHostSide for NetworkContainer<FakeTransporterState> {
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;
    type Sender = FakeTransporterSender;

    fn take_sender(&mut self) -> eros::Result<Self::Sender> {
        TransporterHostSide::take_sender(FakeTransporterImpl::inj_ref_mut(self))
    }

    fn packetize(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        TransporterHostSide::packetize(FakeTransporterImpl::inj_ref_mut(self), stream_id, unit)
    }

    async fn send(sender: &mut Self::Sender, packetized: Self::Packetized) -> eros::Result<()> {
        sender.send(packetized).await
    }
}

impl TransporterClientSide for NetworkContainer<FakeTransporterState> {
    type Receiver = FakeTransporterReceiver;
    type Received = crate::infrastructure::platform::FakeReceived;
    type Depacketized = FakeDecoderInput;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        TransporterClientSide::take_receiver(FakeTransporterImpl::inj_ref_mut(self))
    }

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        receiver.receive().await
    }

    fn depacketize(
        &mut self,
        received: Self::Received,
    ) -> eros::Result<(
        crate::domain::stream::models::vo::StreamId,
        Self::Depacketized,
    )> {
        TransporterClientSide::depacketize(FakeTransporterImpl::inj_ref_mut(self), received)
    }
}

#[cfg(test)]
mod tests {
    use eros::Context;

    use super::*;
    use crate::{
        app::container::{
            host_stream_pipeline::outbound_port::UnitNumber, network::inbound::NetworkWorker,
        },
        domain::stream::models::vo::{FrameId, StreamId},
    };

    #[test]
    fn fake_client_pipeline_decodes_input() -> eros::Result<()> {
        let mut client = PlatformClient::new(FakeDecoderManagerState::new()?);
        let compose_decoder = client.compose_video_decoder_state();
        let mut pipeline = ClientStreamPipelineContainer::new(compose_decoder()?);
        let frame_id = FrameId::new(CaptureSourceId::new(7), 11);
        let buffer = 42_u64.to_le_bytes();

        let decoded = pipeline.decode(FakeDecoderInput::new(frame_id, buffer))?;

        assert!(decoded.frame_id == frame_id);
        assert!(decoded.buffer == buffer);
        Ok(())
    }

    #[test]
    fn fake_network_closes_the_host_to_client_loop() -> eros::Result<()> {
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let mut worker = NetworkWorker::spawn(FakeTransporterState::new, app_message_sender)?;
        let encoded_sender = worker.sender();
        let client_event_receiver = worker.take_client_event_receiver()?;
        let frame_id = FrameId::new(CaptureSourceId::new(3), 9);
        let stream_id = StreamId::new(4);
        encoded_sender.send(
            stream_id,
            EncodedVideoUnit::new(frame_id, UnitNumber::new(2), false, 77_u64.to_le_bytes()),
        )?;
        drop(encoded_sender);

        let runtime = compio::runtime::Runtime::new()?;
        runtime.block_on(async move {
            let event = client_event_receiver
                .receive()
                .await
                .with_context(|| "Fake network did not produce a client event")?;
            let mut client = PlatformClient::new(FakeDecoderManagerState::new()?);
            client.handle_network_input(event.stream_id, event.input)?;
            worker.shutdown().await
        })
    }
}
