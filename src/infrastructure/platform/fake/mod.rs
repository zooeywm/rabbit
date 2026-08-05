mod capture;
mod capture_manager;

mod converter;
mod converter_manager;

mod encoder;
mod encoder_manager;

pub(crate) use capture::FakeScreenCapturerState as ScreenCapturerState;
pub(crate) use capture_manager::{
    FakeCaptureManagerImpl as CaptureManagerImpl, FakeCaptureManagerState as CaptureManagerState,
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
