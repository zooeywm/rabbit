use std::convert::Infallible;

use crate::{
    app::container::{
        client::ClientContainer,
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
        LinuxEncoderFrameConverterState, LinuxScreenCapturerImpl, LinuxScreenCapturerState,
        LinuxTransporterImpl, LinuxTransporterState, LinuxVideoEncoderState,
    },
};

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

pub(super) type PlatformHost = HostContainer<
    LinuxScreenCapturerState,
    LinuxEncoderFrameConverterState,
    LinuxVideoEncoderState,
>;
pub(super) type PlatformClient = ClientContainer<()>;
pub(super) type PlatformNetwork = ConfiguredNetworkContainer<LinuxTransporterState>;
pub(super) type PlatformContainers = (PlatformHost, PlatformClient, Vec<PlatformNetwork>);

pub(super) fn compose_containers(
    app_message_sender: flume::Sender<crate::app::AppMessage>,
) -> eros::Result<PlatformContainers> {
    let (encoded_unit_sender, encoded_unit_receiver) = EncodedUnitSender::channel();
    let (client_stream_control_sender, client_stream_control_receiver) =
        ClientStreamControlSender::channel();
    let (network_request_sender, network_request_receiver) = NetworkRequestSender::channel();

    Ok((
        PlatformHost::new(encoded_unit_sender),
        PlatformClient::new(client_stream_control_sender, network_request_sender),
        vec![PlatformNetwork::new(
            (),
            encoded_unit_receiver,
            client_stream_control_receiver,
            network_request_receiver,
            app_message_sender,
        )],
    ))
}

impl AsMut<LinuxTransporterState> for NetworkContainer<LinuxTransporterState> {
    fn as_mut(&mut self) -> &mut LinuxTransporterState {
        self.state_mut()
    }
}

impl TransporterHostSide for NetworkContainer<LinuxTransporterState> {
    type EncodedBuffer = Infallible;
    type Packetized = Infallible;
    type Host = Infallible;
    type RequestReceiver = Infallible;

    fn take_host(&mut self) -> eros::Result<Self::Host> {
        TransporterHostSide::take_host(LinuxTransporterImpl::inj_ref_mut(self))
    }

    fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver> {
        TransporterHostSide::take_request_receiver(LinuxTransporterImpl::inj_ref_mut(self))
    }

    fn packetize(
        host: &mut Self::Host,
        _stream_id: crate::domain::stream::models::vo::StreamId,
        _unit: EncodedVideoUnit<Self::EncodedBuffer>,
    ) -> eros::Result<Self::Packetized> {
        match *host {}
    }

    async fn send(_host: &mut Self::Host, packetized: Self::Packetized) -> eros::Result<SentBytes> {
        match packetized {}
    }

    async fn receive_request(
        receiver: &mut Self::RequestReceiver,
    ) -> eros::Result<Option<StreamRequest>> {
        match *receiver {}
    }
}

impl TransporterClientSide for NetworkContainer<LinuxTransporterState> {
    type Receiver = Infallible;
    type RequestSender = Infallible;
    type Received = Infallible;
    type Depacketized = Infallible;

    fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
        TransporterClientSide::take_receiver(LinuxTransporterImpl::inj_ref_mut(self))
    }

    fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender> {
        TransporterClientSide::take_request_sender(LinuxTransporterImpl::inj_ref_mut(self))
    }

    async fn receive(_receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
        eros::bail!("Linux transporter is not implemented")
    }

    async fn send_request(
        sender: &mut Self::RequestSender,
        _request: StreamRequest,
    ) -> eros::Result<()> {
        match *sender {}
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

    fn request_video_refresh(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
    ) -> eros::Result<()> {
        TransporterClientSide::request_video_refresh(
            LinuxTransporterImpl::inj_ref_mut(self),
            stream_id,
        )
    }
}
