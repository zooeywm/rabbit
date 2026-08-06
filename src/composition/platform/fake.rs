use crate::infrastructure::platform::{
    FakeCapturerManagerImpl, FakeCapturerManagerState, FakeConverterManagerImpl,
    FakeConverterManagerState, FakeEncoderFrameConverterState, FakeEncoderManagerImpl,
    FakeEncoderManagerState, FakeScreenCapturerState, FakeVideoEncoderState,
};

impl_capturer_boilerplate!(
    FakeCapturerManagerState,
    FakeCapturerManagerImpl,
    FakeScreenCapturerState
);
impl_converter_boilerplate!(
    FakeConverterManagerState,
    FakeConverterManagerImpl,
    FakeEncoderFrameConverterState
);
impl_encoder_boilerplate!(
    FakeEncoderManagerState,
    FakeEncoderManagerImpl,
    FakeVideoEncoderState
);
impl_root_container!(
    FakeCapturerManagerState,
    FakeConverterManagerState,
    FakeEncoderManagerState
);
