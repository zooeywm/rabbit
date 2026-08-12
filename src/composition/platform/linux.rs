use std::convert::Infallible;

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
        LinuxCapturerManagerImpl, LinuxCapturerManagerState, LinuxConverterManagerImpl,
        LinuxConverterManagerState, LinuxEncoderFrameConverterState, LinuxEncoderManagerImpl,
        LinuxEncoderManagerState, LinuxPacketizerImpl, LinuxPacketizerManagerImpl,
        LinuxPacketizerManagerState, LinuxPacketizerState, LinuxScreenCapturerImpl,
        LinuxScreenCapturerState, LinuxVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for LinuxCapturerManagerState {
    type ScreenCapturerState = LinuxScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<LinuxScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsRef<LinuxCapturerManagerState>
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_ref(&self) -> &LinuxCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsMut<LinuxCapturerManagerState>
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_mut(&mut self) -> &mut LinuxCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> CapturerManager
    for AppContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    type State = LinuxCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt, PktMgrSt> {
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

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsRef<LinuxConverterManagerState>
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsMut<LinuxConverterManagerState>
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> ConverterManager
    for AppContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt, PktMgrSt>
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
    + use<CapMgrSt, EcdMgrSt, PktMgrSt> {
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

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsRef<LinuxEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsMut<LinuxEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, PktMgrSt> {
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
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        match input {}
    }
}

impl PacketizerManagerStateSpec for LinuxPacketizerManagerState {
    type PacketizerState = LinuxPacketizerState;
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsRef<LinuxPacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, LinuxPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxPacketizerManagerState {
        self.packetizer_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsMut<LinuxPacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, LinuxPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxPacketizerManagerState {
        self.packetizer_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> PacketizerManager
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, LinuxPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = LinuxPacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, EcdMgrSt> {
        PacketizerManager::compose_packetizer_state(LinuxPacketizerManagerImpl::inj_ref_mut(self))
    }
}

impl AsMut<LinuxPacketizerState> for PacketizerContainer<LinuxPacketizerState> {
    fn as_mut(&mut self) -> &mut LinuxPacketizerState {
        self.state_mut()
    }
}

impl Packetizer for PacketizerContainer<LinuxPacketizerState> {
    type EncodedBuffer = Infallible;

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        Packetizer::packetize(LinuxPacketizerImpl::inj_ref_mut(self), frame)
    }
}

pub(super) type PlatformApp = AppContainer<
    LinuxCapturerManagerState,
    LinuxConverterManagerState,
    LinuxEncoderManagerState,
    LinuxPacketizerManagerState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            LinuxCapturerManagerState::new()?,
            LinuxConverterManagerState::new()?,
            LinuxEncoderManagerState::new()?,
            LinuxPacketizerManagerState::new()?,
        ))
    }
}
