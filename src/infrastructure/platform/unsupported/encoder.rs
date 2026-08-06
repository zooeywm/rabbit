use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedVideoEncoderImpl)]
pub(crate) struct UnsupportedVideoEncoderState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
