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
        UnsupportedCapturerManagerImpl, UnsupportedCapturerManagerState,
        UnsupportedConverterManagerImpl, UnsupportedConverterManagerState,
        UnsupportedEncoderFrameConverterState, UnsupportedEncoderManagerImpl,
        UnsupportedEncoderManagerState, UnsupportedPacketizerImpl,
        UnsupportedPacketizerManagerImpl, UnsupportedPacketizerManagerState,
        UnsupportedPacketizerState, UnsupportedScreenCapturerImpl, UnsupportedScreenCapturerState,
        UnsupportedVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for UnsupportedCapturerManagerState {
    type ScreenCapturerState = UnsupportedScreenCapturerState;
    type ScreenCapturer = ScreenCaptureContainer<UnsupportedScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsRef<UnsupportedCapturerManagerState>
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_ref(&self) -> &UnsupportedCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> AsMut<UnsupportedCapturerManagerState>
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    fn as_mut(&mut self) -> &mut UnsupportedCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt, PktMgrSt> CapturerManager
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt, PktMgrSt>
{
    type State = UnsupportedCapturerManagerState;

    fn compose_screen_capturer_state(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt, PktMgrSt> {
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

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsRef<UnsupportedConverterManagerState>
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> AsMut<UnsupportedConverterManagerState>
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt, PktMgrSt> ConverterManager
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt, PktMgrSt>
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
    + use<CapMgrSt, EcdMgrSt, PktMgrSt> {
        ConverterManager::compose_encoder_frame_converter_state(
            UnsupportedConverterManagerImpl::inj_ref_mut(self),
        )
    }
}

impl<EcdSt> AsRef<UnsupportedEncoderFrameConverterState>
    for StreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &UnsupportedEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<UnsupportedEncoderFrameConverterState>
    for StreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for StreamPipelineContainer<UnsupportedEncoderFrameConverterState, EcdSt>
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

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsRef<UnsupportedEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> AsMut<UnsupportedEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, PktMgrSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState, PktMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = UnsupportedEncoderManagerState;

    fn compose_video_encoder_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, PktMgrSt> {
        EncoderManager::compose_video_encoder_state(UnsupportedEncoderManagerImpl::inj_ref_mut(
            self,
        ))
    }
}

impl<CvtSt> AsRef<UnsupportedVideoEncoderState>
    for StreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState>
{
    fn as_ref(&self) -> &UnsupportedVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<UnsupportedVideoEncoderState>
    for StreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState>
{
    fn as_mut(&mut self) -> &mut UnsupportedVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for StreamPipelineContainer<CvtSt, UnsupportedVideoEncoderState> {
    type EncoderInput = Infallible;
    type EncodedBuffer = Infallible;

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        match input {}
    }
}

impl PacketizerManagerStateSpec for UnsupportedPacketizerManagerState {
    type PacketizerState = UnsupportedPacketizerState;
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsRef<UnsupportedPacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, UnsupportedPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedPacketizerManagerState {
        self.packetizer_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> AsMut<UnsupportedPacketizerManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, UnsupportedPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedPacketizerManagerState {
        self.packetizer_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt, EcdMgrSt> PacketizerManager
    for AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, UnsupportedPacketizerManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
{
    type State = UnsupportedPacketizerManagerState;

    fn compose_packetizer_state(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as PacketizerManagerStateSpec>::PacketizerState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt, EcdMgrSt> {
        PacketizerManager::compose_packetizer_state(UnsupportedPacketizerManagerImpl::inj_ref_mut(
            self,
        ))
    }
}

impl AsMut<UnsupportedPacketizerState> for PacketizerContainer<UnsupportedPacketizerState> {
    fn as_mut(&mut self) -> &mut UnsupportedPacketizerState {
        self.state_mut()
    }
}

impl Packetizer for PacketizerContainer<UnsupportedPacketizerState> {
    type EncodedBuffer = Infallible;

    fn packetize(&mut self, frame: EncodedVideoFrame<Self::EncodedBuffer>) -> eros::Result<()> {
        Packetizer::packetize(UnsupportedPacketizerImpl::inj_ref_mut(self), frame)
    }
}

pub(super) type PlatformApp = AppContainer<
    UnsupportedCapturerManagerState,
    UnsupportedConverterManagerState,
    UnsupportedEncoderManagerState,
    UnsupportedPacketizerManagerState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            UnsupportedCapturerManagerState::new()?,
            UnsupportedConverterManagerState::new()?,
            UnsupportedEncoderManagerState::new()?,
            UnsupportedPacketizerManagerState::new()?,
        ))
    }
}
