use std::{
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

use eros::Context as _;
use tracing::debug;

use crate::kernel::{
    frame_pipeline::{FramePipelineManager, FramePipelineParameters},
    geometry::{FrameRate, FrameRateGate, PixelSize},
    screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
    screen_manager::ScreenId,
};

use super::screen_capture::{WindowsCaptureLease, WindowsCapturedSurface, WindowsFrameReceiver};

#[derive(Debug, Default, kudi::DepInj)]
#[target(WindowsFramePipelineManager)]
pub(crate) struct WindowsFramePipelineManagerState;

impl WindowsFramePipelineManagerState {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct WindowsFramePipelineFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) size: PixelSize,
    pub(crate) source_frame_rate: FrameRate,
    pub(crate) frame_rate: FrameRate,
    pub(crate) probe: Option<super::host_video_probe::HostVideoFrameProbe>,
    surface: WindowsCapturedSurface,
}

impl WindowsFramePipelineFrame {
    pub(crate) fn texture(&self) -> &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D {
        self.surface.texture()
    }
}

pub(crate) struct WindowsFramePipelineSubscription {
    _lease: WindowsCaptureLease,
    receiver: flume::r#async::RecvStream<
        'static,
        eros::Result<super::screen_capture::WindowsCapturedFrame>,
    >,
    frame_size: PixelSize,
    frame_rate: FrameRate,
    frame_rate_gate: FrameRateGate,
}

impl futures_core::Stream for WindowsFramePipelineSubscription {
    type Item = eros::Result<Rc<WindowsFramePipelineFrame>>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        loop {
            match Pin::new(&mut this.receiver).poll_next(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(frame))) => {
                    debug!(
                        event = "windows_capture_frame_received",
                        screen_id = frame.screen_id.0,
                        width = frame.content_size.width,
                        height = frame.content_size.height,
                        source_frame_rate_numerator = frame.frame_rate.numerator(),
                        source_frame_rate_denominator = frame.frame_rate.denominator(),
                        "Received Windows capture frame"
                    );
                    if !this
                        .frame_rate_gate
                        .should_emit(frame.frame_rate, this.frame_rate)
                    {
                        continue;
                    }
                    let mut probe = frame.probe;
                    if let Some(probe) = &mut probe {
                        probe.mark_pipeline_ready();
                    }
                    let emitted = WindowsFramePipelineFrame {
                        screen_id: frame.screen_id,
                        size: if this.frame_size.width == 0 || this.frame_size.height == 0 {
                            frame.content_size
                        } else {
                            this.frame_size
                        },
                        source_frame_rate: frame.frame_rate,
                        frame_rate: this.frame_rate,
                        probe,
                        surface: frame.surface,
                    };
                    return Poll::Ready(Some(Ok(Rc::new(emitted))));
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }
    }
}

impl<Deps> FramePipelineManager for WindowsFramePipelineManager<Deps>
where
    Deps: AsRef<WindowsFramePipelineManagerState>
        + ScreenCaptureManager<Lease = WindowsCaptureLease, Receiver = WindowsFrameReceiver>,
{
    type Frame = WindowsFramePipelineFrame;
    type Subscription = WindowsFramePipelineSubscription;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: FramePipelineParameters,
        frame_rate: FrameRate,
        _delivery: crate::kernel::frame_pipeline::FrameDelivery,
    ) -> eros::Result<Self::Subscription> {
        let _ = <Deps as AsRef<WindowsFramePipelineManagerState>>::as_ref(self.prj_ref());
        let ScreenCaptureSource { lease, receiver } =
            ScreenCaptureManager::acquire(self.prj_ref_mut(), screen_id).with_context(|| {
                format!(
                    "Failed to acquire Windows capture for screen {}",
                    screen_id.get()
                )
            })?;
        Ok(WindowsFramePipelineSubscription {
            _lease: lease,
            receiver: receiver.into_stream(),
            frame_size: parameters.frame_size,
            frame_rate,
            frame_rate_gate: FrameRateGate::default(),
        })
    }
}
