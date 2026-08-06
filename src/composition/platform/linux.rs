use crate::infrastructure::platform::{
    LinuxCapturerManagerImpl, LinuxCapturerManagerState, LinuxConverterManagerImpl,
    LinuxConverterManagerState, LinuxEncoderFrameConverterState, LinuxEncoderManagerImpl,
    LinuxEncoderManagerState, LinuxScreenCapturerState, LinuxVideoEncoderState,
};

impl_capturer_boilerplate!(
    LinuxCapturerManagerState,
    LinuxCapturerManagerImpl,
    LinuxScreenCapturerState
);
impl_converter_boilerplate!(
    LinuxConverterManagerState,
    LinuxConverterManagerImpl,
    LinuxEncoderFrameConverterState
);
impl_encoder_boilerplate!(
    LinuxEncoderManagerState,
    LinuxEncoderManagerImpl,
    LinuxVideoEncoderState
);
impl_root_container!(
    LinuxCapturerManagerState,
    LinuxConverterManagerState,
    LinuxEncoderManagerState
);
