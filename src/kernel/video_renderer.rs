/// A video viewport in physical window pixels with a top-left origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoViewport {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Presents owned decoded frames into a viewport during the GUI render phase.
pub trait VideoRenderer {
    type Frame;

    fn set_viewport(&mut self, viewport: VideoViewport);
    fn present(&mut self, frame: Self::Frame);
    fn render(&mut self) -> eros::Result<()>;
    fn clear(&mut self) -> eros::Result<()>;
}

// Focused test: cargo test kernel::video_renderer::tests --lib
#[cfg(test)]
mod tests {
    use crate::kernel::video_renderer::{VideoRenderer, VideoViewport};

    #[derive(Debug, PartialEq, Eq)]
    struct NonCloneFrame(u8);

    #[derive(Default)]
    struct EmptyVideoRenderer {
        viewport: Option<VideoViewport>,
        frame: Option<NonCloneFrame>,
        rendered: bool,
    }

    impl VideoRenderer for EmptyVideoRenderer {
        type Frame = NonCloneFrame;

        fn set_viewport(&mut self, viewport: VideoViewport) {
            self.viewport = Some(viewport);
        }

        fn present(&mut self, frame: Self::Frame) {
            self.frame = Some(frame);
        }

        fn render(&mut self) -> eros::Result<()> {
            self.rendered = self.viewport.is_some() && self.frame.is_some();
            Ok(())
        }

        fn clear(&mut self) -> eros::Result<()> {
            self.frame = None;
            Ok(())
        }
    }

    #[test]
    fn renderer_keeps_layout_and_owned_frame_inputs_separate() {
        let mut renderer = EmptyVideoRenderer::default();
        renderer.set_viewport(VideoViewport {
            x: 10,
            y: 20,
            width: 1920,
            height: 1080,
        });
        renderer.present(NonCloneFrame(7));

        renderer
            .render()
            .expect("Renderer should render a configured owned frame");

        assert!(renderer.rendered);
        renderer.clear().expect("Renderer should clear its frame");
        assert!(renderer.frame.is_none());
    }
}
