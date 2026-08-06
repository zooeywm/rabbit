use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedEncoderFrameConverterImpl)]
pub(crate) struct UnsupportedEncoderFrameConverterState {
    never: Infallible,
}
