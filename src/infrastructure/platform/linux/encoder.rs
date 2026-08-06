use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxVideoEncoderImpl)]
pub(crate) struct LinuxVideoEncoderState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
