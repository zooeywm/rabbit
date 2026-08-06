use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxEncoderFrameConverterImpl)]
pub(crate) struct LinuxEncoderFrameConverterState {
    never: Infallible,
}
