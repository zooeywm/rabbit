mod capturer;
mod capturer_manager;

mod converter;
mod converter_manager;

mod encoder;
mod encoder_manager;

pub(crate) use capturer::FakeScreenCapturerState as ScreenCapturerState;
pub(crate) use capturer_manager::{
    FakeCapturerManagerImpl as CapturerManagerImpl,
    FakeCapturerManagerState as CapturerManagerState,
};
pub(crate) use converter::{
    FakeEncoderFrameConverterImpl as EncoderFrameConverterImpl,
    FakeEncoderFrameConverterState as EncoderFrameConverterState,
};
pub(crate) use converter_manager::{
    FakeConverterManagerImpl as ConverterManagerImpl,
    FakeConverterManagerState as ConverterManagerState,
};
pub(crate) use encoder::{
    FakeVideoEncoderImpl as VideoEncoderImpl, FakeVideoEncoderState as VideoEncoderState,
};
pub(crate) use encoder_manager::{
    FakeEncoderManagerImpl as EncoderManagerImpl, FakeEncoderManagerState as EncoderManagerState,
};
