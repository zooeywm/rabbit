use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxEncoderFrameConverterImpl)]
pub(crate) struct LinuxEncoderFrameConverterState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
