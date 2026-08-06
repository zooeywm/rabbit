use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedVideoEncoderImpl)]
pub(crate) struct UnsupportedVideoEncoderState {
    never: Infallible,
}
