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
        UnsupportedCapturerManagerImpl, UnsupportedCapturerManagerState,
        UnsupportedConverterManagerImpl, UnsupportedConverterManagerState,
        UnsupportedEncoderFrameConverterState, UnsupportedEncoderManagerImpl,
        UnsupportedEncoderManagerState, UnsupportedScreenCapturerState,
        UnsupportedVideoEncoderState,
    },
};

impl CapturerManagerStateSpec for UnsupportedCapturerManagerState {
    type ScreenCapturerState = UnsupportedScreenCapturerState;
}

impl<CvtMgrSt, EcdMgrSt> AsRef<UnsupportedCapturerManagerState>
    for RootContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<UnsupportedCapturerManagerState>
    for RootContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for RootContainer<UnsupportedCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type ScreenCapturerState = UnsupportedScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        CapturerManager::create_screen_capturer(
            UnsupportedCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl<CvtSt, EcdSt> AsRef<UnsupportedScreenCapturerState>
    for CaptureSourceContainer<UnsupportedScreenCapturerState, CvtSt, EcdSt>
{
    fn as_ref(&self) -> &UnsupportedScreenCapturerState {
        self.screen_capturer_state()
    }
}

impl<CvtSt, EcdSt> AsMut<UnsupportedScreenCapturerState>
    for CaptureSourceContainer<UnsupportedScreenCapturerState, CvtSt, EcdSt>
{
    fn as_mut(&mut self) -> &mut UnsupportedScreenCapturerState {
        self.screen_capturer_state_mut()
    }
}

impl ConverterManagerStateSpec for UnsupportedConverterManagerState {
    type EncoderFrameConverterState = UnsupportedEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt> AsRef<UnsupportedConverterManagerState>
    for RootContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<UnsupportedConverterManagerState>
    for RootContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for RootContainer<CapMgrSt, UnsupportedConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type EncoderFrameConverterState = UnsupportedEncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState> {
        ConverterManager::create_encoder_frame_converter(
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
    for RootContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_ref(&self) -> &UnsupportedEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<UnsupportedEncoderManagerState>
    for RootContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut UnsupportedEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for RootContainer<CapMgrSt, CvtMgrSt, UnsupportedEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    type VideoEncoderState = UnsupportedVideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        EncoderManager::create_video_encoder(UnsupportedEncoderManagerImpl::inj_ref_mut(self))
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

pub(super) fn run() -> eros::Result<()> {
    app::run(|| {
        Ok(RootContainer::new(
            UnsupportedCapturerManagerState::new()?,
            UnsupportedConverterManagerState::new()?,
            UnsupportedEncoderManagerState::new()?,
        ))
    })
}
