mod capturer;
mod capturer_manager;

mod converter;
mod converter_manager;

mod encoder;
mod encoder_manager;

pub(crate) use capturer::{FakeCapturedFrame, FakeScreenCapturerImpl, FakeScreenCapturerState};
pub(crate) use capturer_manager::{FakeCapturerManagerImpl, FakeCapturerManagerState};
pub(crate) use converter::{
    FakeEncoderFrameConverterImpl, FakeEncoderFrameConverterState, FakeEncoderInput,
};
pub(crate) use converter_manager::{FakeConverterManagerImpl, FakeConverterManagerState};
pub(crate) use encoder::{FakeVideoEncoderImpl, FakeVideoEncoderState};
pub(crate) use encoder_manager::{FakeEncoderManagerImpl, FakeEncoderManagerState};
