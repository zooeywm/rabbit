use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedScreenCapturerImpl)]
pub(crate) struct UnsupportedScreenCapturerState {
    never: Infallible,
}
