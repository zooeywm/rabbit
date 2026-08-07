use std::convert::Infallible;

use crate::{
    app::container::{
        AppContainer, ScreenCapturerContainer, StreamPipelineContainer,
        app_container::outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec,
        },
        capture_source::outbound_port::{CaptureLoopAction, ScreenCapturer},
    },
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{
        UnsupportedCapturerManagerImpl, UnsupportedCapturerManagerState,
        UnsupportedConverterManagerImpl, UnsupportedConverterManagerState,
        UnsupportedEncoderFrameConverterState, UnsupportedEncoderManagerImpl,
        UnsupportedEncoderManagerState, UnsupportedScreenCapturerImpl,
        UnsupportedScreenCapturerState, UnsupportedVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for UnsupportedCapturerManagerState {
    type ScreenCapturerState = UnsupportedScreenCapturerState;
    type ScreenCapturer = ScreenCapturerContainer<UnsupportedScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt> AsRef<UnsupportedCapturerManagerState>
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<UnsupportedCapturerManagerState>
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for AppContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
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

impl ScreenCapturer for ScreenCapturerContainer<UnsupportedScreenCapturerState> {
    type CapturedFrame = Infallible;

    fn run<OnStarted, OnFrame>(
        &mut self,
        initial_consumer_count: usize,
        on_started: OnStarted,
        on_frame: OnFrame,
    ) -> eros::Result<()>
    where
        OnStarted: FnOnce() -> eros::Result<()>,
        OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
    {
        ScreenCapturer::run(
            UnsupportedScreenCapturerImpl::inj_ref_mut(self),
            initial_consumer_count,
            on_started,
            on_frame,
        )
    }
}

impl ConverterManagerStateSpec for UnsupportedConverterManagerState {
    type EncoderFrameConverterState = UnsupportedEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt> AsRef<UnsupportedConverterManagerState>
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<UnsupportedConverterManagerState>
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for AppContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
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

impl EncoderManagerStateSpec for UnsupportedEncoderManagerState {
    type VideoEncoderState = UnsupportedVideoEncoderState;
}

impl<CapMgrSt, CvtMgrSt> AsRef<UnsupportedEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<UnsupportedEncoderManagerState>
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for AppContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
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

pub(super) type PlatformApp = AppContainer<
    UnsupportedCapturerManagerState,
    UnsupportedConverterManagerState,
    UnsupportedEncoderManagerState,
>;

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<PlatformApp> + Send + 'static {
    || {
        Ok(AppContainer::new(
            UnsupportedCapturerManagerState::new()?,
            UnsupportedConverterManagerState::new()?,
            UnsupportedEncoderManagerState::new()?,
        ))
    }
}
