use crate::{
    app::{
        self,
        container::{
            CaptureSourceContainer, RootContainer, StreamPipelineContainer,
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
        LinuxEncoderManagerState, LinuxScreenCapturerState, LinuxVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for LinuxCapturerManagerState {
    type ScreenCapturerState = LinuxScreenCapturerState;
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

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState> {
        CapturerManager::create_screen_capturer(
            LinuxCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl<CvtSt, EcdSt> AsRef<LinuxScreenCapturerState>
    for CaptureSourceContainer<LinuxScreenCapturerState, CvtSt, EcdSt>
{
    fn as_ref(&self) -> &LinuxScreenCapturerState {
        self.screen_capturer_state()
    }
}

impl<CvtSt, EcdSt> AsMut<LinuxScreenCapturerState>
    for CaptureSourceContainer<LinuxScreenCapturerState, CvtSt, EcdSt>
{
    fn as_mut(&mut self) -> &mut LinuxScreenCapturerState {
        self.screen_capturer_state_mut()
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

    fn create_encoder_frame_converter(
        &mut self,
    ) -> eros::Result<<Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState> {
        ConverterManager::create_encoder_frame_converter(LinuxConverterManagerImpl::inj_ref_mut(
            self,
        ))
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

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState> {
        EncoderManager::create_video_encoder(LinuxEncoderManagerImpl::inj_ref_mut(self))
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
