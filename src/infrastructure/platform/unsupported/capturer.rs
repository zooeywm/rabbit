use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(UnsupportedScreenCapturerImpl)]
pub(crate) struct UnsupportedScreenCapturerState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
