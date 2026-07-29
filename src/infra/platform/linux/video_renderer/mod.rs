use eros::Context as _;

use crate::kernel::{geometry::PixelSize, video_renderer::VideoViewport};

mod wayland;

pub(crate) use wayland::{WaylandVideoRenderer, WaylandVideoViewport};

fn fit_viewport(viewport: VideoViewport, frame_size: PixelSize) -> eros::Result<VideoViewport> {
    if frame_size.width == 0 || frame_size.height == 0 {
        eros::bail!("Cannot render a zero-sized video frame");
    }
    let width_limited = u64::from(viewport.width) * u64::from(frame_size.height)
        <= u64::from(viewport.height) * u64::from(frame_size.width);
    let (width, height) = if width_limited {
        (
            viewport.width,
            u32::try_from(
                u64::from(viewport.width) * u64::from(frame_size.height)
                    / u64::from(frame_size.width),
            )
            .with_context(|| "Fitted video height exceeds u32")?,
        )
    } else {
        (
            u32::try_from(
                u64::from(viewport.height) * u64::from(frame_size.width)
                    / u64::from(frame_size.height),
            )
            .with_context(|| "Fitted video width exceeds u32")?,
            viewport.height,
        )
    };
    Ok(VideoViewport {
        x: viewport.x + (viewport.width - width) / 2,
        y: viewport.y + (viewport.height - height) / 2,
        width,
        height,
    })
}

// Focused test: cargo test infra::platform::video_renderer::tests:: --lib
#[cfg(test)]
mod tests {
    use crate::{
        infra::platform::video_renderer::fit_viewport,
        kernel::{geometry::PixelSize, video_renderer::VideoViewport},
    };

    #[test]
    fn fits_video_inside_viewport_without_changing_aspect_ratio() {
        let fitted = fit_viewport(
            VideoViewport {
                x: 10,
                y: 20,
                width: 1920,
                height: 1200,
            },
            PixelSize {
                width: 1920,
                height: 1080,
            },
        )
        .expect("Video viewport should fit");

        assert_eq!(
            fitted,
            VideoViewport {
                x: 10,
                y: 80,
                width: 1920,
                height: 1080,
            }
        );
    }
}
