use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxVideoEncoderImpl)]
pub(crate) struct LinuxVideoEncoderState {
    never: Infallible,
}
