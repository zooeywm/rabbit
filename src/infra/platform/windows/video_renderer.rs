use std::{ffi::CStr, time::Duration};

use crate::{
    infra::platform::video_decoder::WindowsDecodedFrame,
    kernel::video_renderer::{VideoRenderer, VideoViewport},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NativeVideoViewport {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl NativeVideoViewport {
    fn as_video_viewport(self) -> VideoViewport {
        VideoViewport {
            x: self.x.max(0) as u32,
            y: self.y.max(0) as u32,
            width: self.width.max(0) as u32,
            height: self.height.max(0) as u32,
        }
    }
}

impl From<VideoViewport> for NativeVideoViewport {
    fn from(value: VideoViewport) -> Self {
        Self {
            x: value.x.min(i32::MAX as u32) as i32,
            y: value.y.min(i32::MAX as u32) as i32,
            width: value.width.min(i32::MAX as u32) as i32,
            height: value.height.min(i32::MAX as u32) as i32,
        }
    }
}

pub(crate) struct NativeVideoRenderer {
    viewport: NativeVideoViewport,
    pending_frame: Option<WindowsDecodedFrame>,
}

impl NativeVideoRenderer {
    pub(crate) fn new(_window: &slint::Window, _probe_interval: Duration) -> eros::Result<Self> {
        eros::bail!("Native Windows D3D11 video presentation is not implemented yet")
    }

    pub(crate) fn set_viewport(&mut self, viewport: NativeVideoViewport) -> eros::Result<()> {
        if viewport.width < 0 || viewport.height < 0 {
            eros::bail!(
                "Windows native video viewport has negative size {}x{}",
                viewport.width,
                viewport.height
            );
        }
        self.viewport = viewport;
        Ok(())
    }

    pub(crate) fn validate_frame(&self, frame: &WindowsDecodedFrame) -> eros::Result<()> {
        if frame.size.width == 0 || frame.size.height == 0 {
            eros::bail!("Windows decoded frame has an empty size")
        }
        Ok(())
    }

    pub(crate) fn teardown(&mut self) -> eros::Result<()> {
        self.clear()
    }
}

impl VideoRenderer for NativeVideoRenderer {
    type Frame = WindowsDecodedFrame;

    fn set_viewport(&mut self, viewport: VideoViewport) {
        self.viewport = NativeVideoViewport::from(viewport);
    }

    fn present(&mut self, frame: Self::Frame) {
        self.pending_frame = Some(frame);
    }

    fn render(&mut self) -> eros::Result<()> {
        let _ = (self.viewport.as_video_viewport(), &self.pending_frame);
        Ok(())
    }

    fn clear(&mut self) -> eros::Result<()> {
        self.pending_frame = None;
        Ok(())
    }
}

pub(crate) struct OpenGlVideoRenderer {
    viewport: Option<VideoViewport>,
    pending_frame: Option<WindowsDecodedFrame>,
}

impl OpenGlVideoRenderer {
    pub(crate) fn new(
        _get_proc_address: &dyn Fn(&CStr) -> *const std::ffi::c_void,
        _probe_interval: Duration,
    ) -> eros::Result<Self> {
        Ok(Self {
            viewport: None,
            pending_frame: None,
        })
    }

    pub(crate) fn teardown(&mut self) -> eros::Result<()> {
        self.clear()
    }
}

impl VideoRenderer for OpenGlVideoRenderer {
    type Frame = WindowsDecodedFrame;

    fn set_viewport(&mut self, viewport: VideoViewport) {
        self.viewport = Some(viewport);
    }

    fn present(&mut self, frame: Self::Frame) {
        let _ = frame.payload.len();
        self.pending_frame = Some(frame);
    }

    fn render(&mut self) -> eros::Result<()> {
        let _ = (&self.viewport, &self.pending_frame);
        Ok(())
    }

    fn clear(&mut self) -> eros::Result<()> {
        self.pending_frame = None;
        Ok(())
    }
}
