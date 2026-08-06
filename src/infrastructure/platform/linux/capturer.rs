use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(LinuxScreenCapturerImpl)]
pub(crate) struct LinuxScreenCapturerState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
