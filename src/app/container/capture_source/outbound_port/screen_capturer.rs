pub trait ScreenCapturer {
    type CapturedFrame;

    fn capture_next(&mut self) -> eros::Result<Self::CapturedFrame>;
}
