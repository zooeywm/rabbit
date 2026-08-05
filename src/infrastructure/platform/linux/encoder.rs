use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(VideoEncoderImpl)]
pub(crate) struct VideoEncoderState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
