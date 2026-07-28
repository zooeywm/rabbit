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

use super::screen_capture::{WgcCaptureLease, WgcFrameReceiver};

#[derive(Debug, Default, kudi::DepInj)]
#[target(WgcFramePipelineManager)]
pub(crate) struct WgcFramePipelineManagerState;

impl WgcFramePipelineManagerState {
    pub(crate) fn new() -> Self {
        Self
    }
}

#[derive(Debug)]
pub(crate) struct WgcFramePipelineFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) texture: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    pub(crate) size: PixelSize,
    pub(crate) source_frame_rate: FrameRate,
    pub(crate) frame_rate: FrameRate,
    pub(crate) probe: Option<super::host_video_probe::HostVideoFrameProbe>,
}

pub(crate) struct WgcFramePipelineSubscription {
    _lease: WgcCaptureLease,
    receiver:
        flume::r#async::RecvStream<'static, eros::Result<super::screen_capture::WgcCapturedFrame>>,
    frame_size: PixelSize,
    frame_rate: FrameRate,
    frame_rate_gate: FrameRateGate,
}

impl futures_core::Stream for WgcFramePipelineSubscription {
    type Item = eros::Result<Rc<WgcFramePipelineFrame>>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.as_mut().get_mut();
        loop {
            match Pin::new(&mut this.receiver).poll_next(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(frame))) => {
                    debug!(
                        event = "wgc_frame_received",
                        screen_id = frame.screen_id.0,
                        width = frame.content_size.width,
                        height = frame.content_size.height,
                        source_frame_rate_numerator = frame.frame_rate.numerator(),
                        source_frame_rate_denominator = frame.frame_rate.denominator(),
                        "Received WGC frame"
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
                    let emitted = WgcFramePipelineFrame {
                        screen_id: frame.screen_id,
                        texture: frame.texture,
                        size: if this.frame_size.width == 0 || this.frame_size.height == 0 {
                            frame.content_size
                        } else {
                            this.frame_size
                        },
                        source_frame_rate: frame.frame_rate,
                        frame_rate: this.frame_rate,
                        probe,
                    };
                    return Poll::Ready(Some(Ok(Rc::new(emitted))));
                }
                Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                Poll::Ready(None) => return Poll::Ready(None),
            }
        }
    }
}

impl<Deps> FramePipelineManager for WgcFramePipelineManager<Deps>
where
    Deps: AsRef<WgcFramePipelineManagerState>
        + ScreenCaptureManager<Lease = WgcCaptureLease, Receiver = WgcFrameReceiver>,
{
    type Frame = WgcFramePipelineFrame;
    type Subscription = WgcFramePipelineSubscription;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: FramePipelineParameters,
        frame_rate: FrameRate,
        _delivery: crate::kernel::frame_pipeline::FrameDelivery,
    ) -> eros::Result<Self::Subscription> {
        let _ = <Deps as AsRef<WgcFramePipelineManagerState>>::as_ref(self.prj_ref());
        let ScreenCaptureSource { lease, receiver } =
            ScreenCaptureManager::acquire(self.prj_ref_mut(), screen_id).with_context(|| {
                format!(
                    "Failed to acquire WGC capture for screen {}",
                    screen_id.get()
                )
            })?;
        Ok(WgcFramePipelineSubscription {
            _lease: lease,
            receiver: receiver.into_stream(),
            frame_size: parameters.frame_size,
            frame_rate,
            frame_rate_gate: FrameRateGate::default(),
        })
    }
}
