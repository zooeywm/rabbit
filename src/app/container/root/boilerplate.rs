use crate::{
    app::container::root::outbound_port::{CapturerManager, ConverterManager, EncoderManager},
    domain::stream::models::vo::CaptureSourceId,
    infrastructure::platform::{
        CapturerManagerImpl, CapturerManagerState, ConverterManagerImpl, ConverterManagerState,
        EncoderFrameConverterState, EncoderManagerImpl, EncoderManagerState, ScreenCapturerState,
        VideoEncoderState,
    },
};

use super::RootContainer;

impl AsRef<CapturerManagerState> for RootContainer {
    fn as_ref(&self) -> &CapturerManagerState {
        &self.capturer_manager_state
    }
}

impl AsMut<CapturerManagerState> for RootContainer {
    fn as_mut(&mut self) -> &mut CapturerManagerState {
        &mut self.capturer_manager_state
    }
}

impl AsRef<ConverterManagerState> for RootContainer {
    fn as_ref(&self) -> &ConverterManagerState {
        &self.converter_manager_state
    }
}

impl AsMut<ConverterManagerState> for RootContainer {
    fn as_mut(&mut self) -> &mut ConverterManagerState {
        &mut self.converter_manager_state
    }
}

impl AsRef<EncoderManagerState> for RootContainer {
    fn as_ref(&self) -> &EncoderManagerState {
        &self.encoder_manager_state
    }
}

impl AsMut<EncoderManagerState> for RootContainer {
    fn as_mut(&mut self) -> &mut EncoderManagerState {
        &mut self.encoder_manager_state
    }
}

impl CapturerManager for RootContainer {
    type ScreenCapturerState = ScreenCapturerState;

    fn create_screen_capturer(
        &mut self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<Self::ScreenCapturerState> {
        CapturerManagerImpl::inj_ref_mut(self).create_screen_capturer(capture_source_id)
    }
}

impl ConverterManager for RootContainer {
    type EncoderFrameConverterState = EncoderFrameConverterState;

    fn create_encoder_frame_converter(&mut self) -> eros::Result<Self::EncoderFrameConverterState> {
        ConverterManagerImpl::inj_ref_mut(self).create_encoder_frame_converter()
    }
}

impl EncoderManager for RootContainer {
    type VideoEncoderState = VideoEncoderState;

    fn create_video_encoder(&mut self) -> eros::Result<Self::VideoEncoderState> {
        EncoderManagerImpl::inj_ref_mut(self).create_video_encoder()
    }
}
