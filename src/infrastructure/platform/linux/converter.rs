use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(EncoderFrameConverterImpl)]
pub(crate) struct EncoderFrameConverterState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
