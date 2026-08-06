use crate::{
    app::container::{
        CaptureSourceContainer, RootContainer, StreamPipelineContainer,
        capture_source::outbound_port::{CaptureFramePoolCapacityController, ScreenCapturer},
        root::outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec,
        },
        stream_pipeline::{
            EncodedVideoFrame,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
    },
    infrastructure::platform::{
        FakeCapturedFrame, FakeCapturerManagerImpl, FakeCapturerManagerState,
        FakeConverterManagerImpl, FakeConverterManagerState, FakeEncoderFrameConverterImpl,
        FakeEncoderFrameConverterState, FakeEncoderInput, FakeEncoderManagerImpl,
        FakeEncoderManagerState, FakeScreenCapturerImpl, FakeScreenCapturerState,
        FakeVideoEncoderImpl, FakeVideoEncoderState,
    },
    infrastructure::support::media::FrameLease,
};

impl CapturerManagerStateSpec for FakeCapturerManagerState {
    type ScreenCapturerState = FakeScreenCapturerState;
}

impl<CvtMgrSt, EcdMgrSt> AsRef<FakeCapturerManagerState>
    for RootContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &FakeCapturerManagerState {
        self.capturer_manager_state()
    }
}

impl<CvtMgrSt, EcdMgrSt> AsMut<FakeCapturerManagerState>
    for RootContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeCapturerManagerState {
        self.capturer_manager_state_mut()
    }
}

impl<CvtMgrSt, EcdMgrSt> CapturerManager
    for RootContainer<FakeCapturerManagerState, CvtMgrSt, EcdMgrSt>
where
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type State = FakeCapturerManagerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: crate::domain::stream::models::vo::CaptureSourceId,
    ) -> eros::Result<<Self::State as CapturerManagerStateSpec>::ScreenCapturerState> {
        CapturerManager::create_screen_capturer(
            FakeCapturerManagerImpl::inj_ref_mut(self),
            capture_source_id,
        )
    }
}

impl<CvtSt, EcdSt> AsRef<FakeScreenCapturerState>
    for CaptureSourceContainer<FakeScreenCapturerState, CvtSt, EcdSt>
{
    fn as_ref(&self) -> &FakeScreenCapturerState {
        self.screen_capturer_state()
    }
}

impl<CvtSt, EcdSt> AsMut<FakeScreenCapturerState>
    for CaptureSourceContainer<FakeScreenCapturerState, CvtSt, EcdSt>
{
    fn as_mut(&mut self) -> &mut FakeScreenCapturerState {
        self.screen_capturer_state_mut()
    }
}

impl<CvtSt, EcdSt> CaptureFramePoolCapacityController
    for CaptureSourceContainer<FakeScreenCapturerState, CvtSt, EcdSt>
{
    fn set_pool_size(&mut self, pool_size: usize) -> eros::Result<()> {
        CaptureFramePoolCapacityController::set_pool_size(
            FakeScreenCapturerImpl::inj_ref_mut(self),
            pool_size,
        )
    }
}

impl<CvtSt, EcdSt> ScreenCapturer
    for CaptureSourceContainer<FakeScreenCapturerState, CvtSt, EcdSt>
{
    type CapturedFrame = FrameLease<FakeCapturedFrame>;

    fn capture_next(&mut self) -> eros::Result<Self::CapturedFrame> {
        ScreenCapturer::capture_next(FakeScreenCapturerImpl::inj_ref_mut(self))
    }
}

impl ConverterManagerStateSpec for FakeConverterManagerState {
    type EncoderFrameConverterState = FakeEncoderFrameConverterState;
}

impl<CapMgrSt, EcdMgrSt> AsRef<FakeConverterManagerState>
    for RootContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_ref(&self) -> &FakeConverterManagerState {
        self.converter_manager_state()
    }
}

impl<CapMgrSt, EcdMgrSt> AsMut<FakeConverterManagerState>
    for RootContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeConverterManagerState {
        self.converter_manager_state_mut()
    }
}

impl<CapMgrSt, EcdMgrSt> ConverterManager
    for RootContainer<CapMgrSt, FakeConverterManagerState, EcdMgrSt>
where
    CapMgrSt: CapturerManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
{
    type State = FakeConverterManagerState;

    fn create_encoder_frame_converter(
        &mut self,
    ) -> eros::Result<<Self::State as ConverterManagerStateSpec>::EncoderFrameConverterState> {
        ConverterManager::create_encoder_frame_converter(FakeConverterManagerImpl::inj_ref_mut(
            self,
        ))
    }
}

impl<EcdSt> AsRef<FakeEncoderFrameConverterState>
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_ref(&self) -> &FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state()
    }
}

impl<EcdSt> AsMut<FakeEncoderFrameConverterState>
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
{
    fn as_mut(&mut self) -> &mut FakeEncoderFrameConverterState {
        self.encoder_frame_converter_state_mut()
    }
}

impl<EcdSt> EncoderFrameConverter
    for StreamPipelineContainer<FakeEncoderFrameConverterState, EcdSt>
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
    for RootContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_ref(&self) -> &FakeEncoderManagerState {
        self.encoder_manager_state()
    }
}

impl<CapMgrSt, CvtMgrSt> AsMut<FakeEncoderManagerState>
    for RootContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    fn as_mut(&mut self) -> &mut FakeEncoderManagerState {
        self.encoder_manager_state_mut()
    }
}

impl<CapMgrSt, CvtMgrSt> EncoderManager
    for RootContainer<CapMgrSt, CvtMgrSt, FakeEncoderManagerState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
{
    type State = FakeEncoderManagerState;

    fn create_video_encoder(
        &mut self,
    ) -> eros::Result<<Self::State as EncoderManagerStateSpec>::VideoEncoderState> {
        EncoderManager::create_video_encoder(FakeEncoderManagerImpl::inj_ref_mut(self))
    }
}

impl<CvtSt> AsRef<FakeVideoEncoderState> for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    fn as_ref(&self) -> &FakeVideoEncoderState {
        self.video_encoder_state()
    }
}

impl<CvtSt> AsMut<FakeVideoEncoderState> for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    fn as_mut(&mut self) -> &mut FakeVideoEncoderState {
        self.video_encoder_state_mut()
    }
}

impl<CvtSt> VideoEncoder for StreamPipelineContainer<CvtSt, FakeVideoEncoderState> {
    type EncoderInput = FakeEncoderInput;
    type EncodedBuffer = [u8; 8];

    fn encode(
        &mut self,
        input: Self::EncoderInput,
    ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
        VideoEncoder::encode(FakeVideoEncoderImpl::inj_ref_mut(self), input)
    }
}

pub(super) fn run() -> eros::Result<()> {
    crate::app::run(|| {
        Ok(RootContainer::new(
            FakeCapturerManagerState::new()?,
            FakeConverterManagerState::new()?,
            FakeEncoderManagerState::new()?,
        ))
    })
}
