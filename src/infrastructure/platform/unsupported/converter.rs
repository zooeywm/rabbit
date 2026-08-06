use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedEncoderFrameConverterImpl)]
pub(crate) struct UnsupportedEncoderFrameConverterState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
