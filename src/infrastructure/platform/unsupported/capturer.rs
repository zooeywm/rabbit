use std::convert::Infallible;

#[derive(kudi::DepInj)]
#[target(ScreenCapturerImpl)]
pub(crate) struct ScreenCapturerState {
    /// Prevents this state from ever being constructed.
    never: Infallible,
}
