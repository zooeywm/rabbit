use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxScreenCapturerImpl)]
pub(crate) struct LinuxScreenCapturerState {
    never: Infallible,
}
