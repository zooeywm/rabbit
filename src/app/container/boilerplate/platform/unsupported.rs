use crate::infrastructure::platform::{
    UnsupportedCapturerManagerImpl, UnsupportedCapturerManagerState,
    UnsupportedConverterManagerImpl, UnsupportedConverterManagerState,
    UnsupportedEncoderFrameConverterState, UnsupportedEncoderManagerImpl,
    UnsupportedEncoderManagerState, UnsupportedScreenCapturerState, UnsupportedVideoEncoderState,
};

impl_capturer_boilerplate!(
    UnsupportedCapturerManagerState,
    UnsupportedCapturerManagerImpl,
    UnsupportedScreenCapturerState
);
impl_converter_boilerplate!(
    UnsupportedConverterManagerState,
    UnsupportedConverterManagerImpl,
    UnsupportedEncoderFrameConverterState
);
impl_encoder_boilerplate!(
    UnsupportedEncoderManagerState,
    UnsupportedEncoderManagerImpl,
    UnsupportedVideoEncoderState
);
impl_root_container!(
    UnsupportedCapturerManagerState,
    UnsupportedConverterManagerState,
    UnsupportedEncoderManagerState
);
