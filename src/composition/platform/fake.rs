use crate::{
    app::container::{
        client::ClientContainer,
        client_stream_pipeline::{ClientStreamPipelineContainer, outbound_port::VideoDecoder},
        host::HostContainer,
        host_stream_pipeline::{
            HostStreamPipelineContainer,
            outbound_port::{EncodedVideoUnit, EncoderFrameConverter, VideoEncoder},
        },
        network::{
            ConfiguredNetworkContainer, NetworkContainer,
            inbound::{ClientStreamControlSender, EncodedUnitSender, NetworkRequestSender},
            outbound_port::{SentBytes, TransporterClientSide, TransporterHostSide},
        },
        screen_capture::{
            ScreenCaptureContainer,
            outbound_port::{CaptureLoopAction, ScreenCapturer},
        },
    },
    domain::stream::models::StreamRequest,
    infrastructure::platform::{
        FakeCapturedFrame, FakeDecoderInput, FakeEncoderFrameConverterImpl,
        FakeEncoderFrameConverterState, FakeEncoderInput, FakePacketized,
        FakeScreenCapturerControl, FakeScreenCapturerImpl, FakeScreenCapturerState,
        FakeTransporterConfig, FakeTransporterHost, FakeTransporterImpl, FakeTransporterReceiver,
        FakeTransporterRequestReceiver, FakeTransporterRequestSender, FakeTransporterState,
        FakeVideoDecoderImpl, FakeVideoDecoderState, FakeVideoEncoderImpl, FakeVideoEncoderState,
    },
    infrastructure::support::media::FrameLease,
};

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

    fn reset(&mut self) -> eros::Result<()> {
        VideoDecoder::reset(FakeVideoDecoderImpl::inj_ref_mut(self))
    }

    fn submit(&mut self, input: Self::DecoderInput) -> eros::Result<()> {
        VideoDecoder::submit(FakeVideoDecoderImpl::inj_ref_mut(self), input)
    }

    fn try_receive(
        &mut self,
    ) -> eros::Result<
        Option<
            crate::app::container::client_stream_pipeline::outbound_port::DecodedVideoFrame<
                Self::DecodedBuffer,
            >,
        >,
    > {
        VideoDecoder::try_receive(FakeVideoDecoderImpl::inj_ref_mut(self))
    }
}

pub(super) type PlatformHost =
    HostContainer<FakeScreenCapturerState, FakeEncoderFrameConverterState, FakeVideoEncoderState>;
pub(super) type PlatformClient = ClientContainer<FakeVideoDecoderState>;
pub(super) type PlatformNetwork = ConfiguredNetworkContainer<FakeTransporterState>;
pub(super) type PlatformContainers = (PlatformHost, PlatformClient, Vec<PlatformNetwork>);

pub(super) fn compose_containers(
    app_message_sender: flume::Sender<crate::app::AppMessage>,
) -> eros::Result<PlatformContainers> {
    let (client_endpoint_config, host_endpoint_config) = FakeTransporterConfig::pair();

    let (_client_encoded_unit_sender, client_encoded_unit_receiver) = EncodedUnitSender::channel();
    let (client_stream_control_sender, client_stream_control_receiver) =
        ClientStreamControlSender::channel();
    let (client_network_request_sender, client_network_request_receiver) =
        NetworkRequestSender::channel();

    let (host_encoded_unit_sender, host_encoded_unit_receiver) = EncodedUnitSender::channel();
    let (_host_client_stream_control_sender, host_client_stream_control_receiver) =
        ClientStreamControlSender::channel();
    let (_host_network_request_sender, host_network_request_receiver) =
        NetworkRequestSender::channel();

    Ok((
        PlatformHost::new(host_encoded_unit_sender),
        PlatformClient::new(client_stream_control_sender, client_network_request_sender),
        vec![
            PlatformNetwork::new(
                client_endpoint_config,
                client_encoded_unit_receiver,
                client_stream_control_receiver,
                client_network_request_receiver,
                app_message_sender.clone(),
            ),
            PlatformNetwork::new(
                host_endpoint_config,
                host_encoded_unit_receiver,
                host_client_stream_control_receiver,
                host_network_request_receiver,
                app_message_sender,
            ),
        ],
    ))
}

impl AsMut<FakeTransporterState> for NetworkContainer<FakeTransporterState> {
    fn as_mut(&mut self) -> &mut FakeTransporterState {
        self.state_mut()
    }
}

impl TransporterHostSide for NetworkContainer<FakeTransporterState> {
    type EncodedBuffer = [u8; 8];
    type Packetized = FakePacketized;
    type Host = FakeTransporterHost;
    type RequestReceiver = FakeTransporterRequestReceiver;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        TransporterHostSide::take_host(FakeTransporterImpl::inj_ref_mut(self))
    }

    fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver> {
        TransporterHostSide::take_request_receiver(FakeTransporterImpl::inj_ref_mut(self))
    }

    fn packetize(
        host: &mut Self::Host,
        stream_id: crate::domain::stream::models::vo::StreamId,
        unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        Ok(host.packetize(stream_id, unit))
    }

    async fn send(host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes> {
        host.send(packetized).await
    }

    async fn receive_request(
        receiver: &mut Self::RequestReceiver,
    ) -> eros::Result<Option<StreamRequest>> {
        receiver.receive().await
    }
}

impl TransporterClientSide for NetworkContainer<FakeTransporterState> {
    type Receiver = FakeTransporterReceiver;
    type RequestSender = FakeTransporterRequestSender;
    type Received = crate::infrastructure::platform::FakeReceived;
    type Depacketized = FakeDecoderInput;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        TransporterClientSide::take_receiver(FakeTransporterImpl::inj_ref_mut(self))
    }

    fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender> {
        TransporterClientSide::take_request_sender(FakeTransporterImpl::inj_ref_mut(self))
    }

    async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        receiver.receive().await
    }

    async fn send_request(
        sender: &mut Self::RequestSender,
        request: StreamRequest,
    ) -> eros::Result<()> {
        sender.send(request).await
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

    fn request_video_refresh(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
    ) -> eros::Result<()> {
        TransporterClientSide::request_video_refresh(
            FakeTransporterImpl::inj_ref_mut(self),
            stream_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::container::{
            host_stream_pipeline::outbound_port::UnitNumber, network::inbound::NetworkWorker,
        },
        domain::stream::models::vo::{CaptureSourceId, FrameId},
    };

    #[test]
    fn fake_client_pipeline_decodes_input() -> eros::Result<()> {
        let mut pipeline = ClientStreamPipelineContainer::new(FakeVideoDecoderState::new());
        let frame_id = FrameId::new(CaptureSourceId::new(7), 11);
        let buffer = 42_u64.to_le_bytes();

        pipeline.submit(FakeDecoderInput::new(frame_id, buffer, true))?;
        let decoded = pipeline
            .try_receive()?
            .expect("Fake decoder should produce one output");

        assert!(decoded.frame_id == frame_id);
        assert!(decoded.buffer == buffer);
        Ok(())
    }

    #[test]
    fn fake_network_closes_the_host_to_client_loop() -> eros::Result<()> {
        let (a_config, b_config) = FakeTransporterConfig::pair();
        let (a_encoded_sender, a_encoded_receiver) = EncodedUnitSender::channel();
        let (_a_client_stream_control_sender, a_client_stream_control_receiver) =
            ClientStreamControlSender::channel();
        let (_a_network_request_sender, a_network_request_receiver) =
            NetworkRequestSender::channel();
        let (a_app_message_sender, _a_app_message_receiver) = flume::unbounded();
        let a_worker =
            NetworkWorker::spawn(ConfiguredNetworkContainer::<FakeTransporterState>::new(
                a_config,
                a_encoded_receiver,
                a_client_stream_control_receiver,
                a_network_request_receiver,
                a_app_message_sender,
            ))?;

        let (_b_encoded_sender, b_encoded_receiver) = EncodedUnitSender::channel();
        let (b_client_stream_control_sender, b_client_stream_control_receiver) =
            ClientStreamControlSender::channel();
        let (b_network_request_sender, b_network_request_receiver) =
            NetworkRequestSender::channel();
        let (b_app_message_sender, _b_app_message_receiver) = flume::unbounded();
        let b_worker =
            NetworkWorker::spawn(ConfiguredNetworkContainer::<FakeTransporterState>::new(
                b_config,
                b_encoded_receiver,
                b_client_stream_control_receiver,
                b_network_request_receiver,
                b_app_message_sender,
            ))?;
        let frame_id = FrameId::new(CaptureSourceId::new(3), 9);

        let runtime = compio::runtime::Runtime::new()?;
        runtime.block_on(async move {
            let mut client =
                PlatformClient::new(b_client_stream_control_sender, b_network_request_sender);
            let stream_id = client.start_stream(CaptureSourceId::new(3)).await?;
            let decoded_frame_slot = client
                .decoded_frame_slot(stream_id)
                .expect("Client stream should expose its decoded frame slot");
            a_encoded_sender.send(
                stream_id,
                EncodedVideoUnit::new(frame_id, UnitNumber::new(2), false, 77_u64.to_le_bytes()),
            )?;

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
            let decoded = loop {
                if let Some(decoded) = decoded_frame_slot.take_latest() {
                    break decoded;
                }
                if std::time::Instant::now() >= deadline {
                    eros::bail!("Fake Client pipeline did not produce a decoded frame");
                }
                std::thread::yield_now();
            };
            assert!(decoded.frame_id == frame_id);
            assert!(decoded.buffer == 77_u64.to_le_bytes());

            client.remove_stream(stream_id).await?;
            let a_result = a_worker.shutdown().await;
            let b_result = b_worker.shutdown().await;
            a_result?;
            b_result
        })
    }
}
