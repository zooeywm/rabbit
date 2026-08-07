use std::convert::Infallible;

use crate::{
    app::{
        self,
        container::{
            RootContainer, ScreenCapturerContainer, StreamPipelineContainer,
            capture_source::outbound_port::{CaptureLoopAction, ScreenCapturer},
            root::outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
            },
        },
    },
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{
        LinuxCapturerManagerImpl, LinuxCapturerManagerState, LinuxConverterManagerImpl,
        LinuxConverterManagerState, LinuxEncoderFrameConverterState, LinuxEncoderManagerImpl,
        LinuxEncoderManagerState, LinuxScreenCapturerImpl, LinuxScreenCapturerState,
        LinuxVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for LinuxCapturerManagerState {
    type ScreenCapturerState = LinuxScreenCapturerState;
    type ScreenCapturer = ScreenCapturerContainer<LinuxScreenCapturerState>;
}

impl<CvtMgrSt, EcdMgrSt> AsRef<LinuxCapturerManagerState>
    for RootContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<LinuxCapturerManagerState>
    for RootContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for RootContainer<LinuxCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type State = LinuxCapturerManagerState;

    fn screen_capturer_state_factory(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> impl FnOnce()
        -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState>
    + Send
    + 'static
    + use<CvtMgrSt, EcdMgrSt> {
        CapturerManager::screen_capturer_state_factory(
            LinuxCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl ScreenCapturer for ScreenCapturerContainer<LinuxScreenCapturerState> {
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
            LinuxScreenCapturerImpl::inj_ref_mut(self),
            initial_consumer_count,
            on_started,
            on_frame,
        )
    }
}

impl ConverterManagerStateSpec for LinuxConverterManagerState {
    type EncoderFrameConverterState = LinuxEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt> AsRef<LinuxConverterManagerState>
    for RootContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<LinuxConverterManagerState>
    for RootContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for RootContainer<CapMgrSt, LinuxConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type State = LinuxConverterManagerState;

    fn encoder_frame_converter_state_factory(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<
        <Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState,
    >
    + Send
    + 'static
    + use<CapMgrSt, EcdMgrSt> {
        ConverterManager::encoder_frame_converter_state_factory(
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

impl EncoderManagerStateSpec for LinuxEncoderManagerState {
    type VideoEncoderState = LinuxVideoEncoderState;
}

impl<CapMgrSt, CvtMgrSt> AsRef<LinuxEncoderManagerState>
    for RootContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_ref(&self) -> &LinuxEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<LinuxEncoderManagerState>
    for RootContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut LinuxEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for RootContainer<CapMgrSt, CvtMgrSt, LinuxEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    type State = LinuxEncoderManagerState;

    fn video_encoder_state_factory(
        &mut self,
    ) -> impl FnOnce() -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState>
    + Send
    + 'static
    + use<CapMgrSt, CvtMgrSt> {
        EncoderManager::video_encoder_state_factory(LinuxEncoderManagerImpl::inj_ref_mut(self))
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

pub(super) fn run() -> eros::Result<()> {
    app::run(|| {
        Ok(RootContainer::new(
            LinuxCapturerManagerState::new()?,
            LinuxConverterManagerState::new()?,
            LinuxEncoderManagerState::new()?,
        ))
    })
}
