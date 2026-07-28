//! Linux GStreamer H.264 encode stack.
//!
//! Submodules own the pipeline stages; this root re-exports the public surface
//! and hosts focused integration tests under `tests`.

use drm::buffer::DrmFourcc;
use eros::Context as _;
use gstreamer::prelude::GstObjectExt as _;

use crate::infra::platform::dma_buf::DmaBufFrame;

mod discovery;
mod encoder;
mod frame;
mod pipeline_util;
mod probe;
mod recorder;
mod rtp;
mod va_surface;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use discovery::va_vpp_input_modifier;
pub(crate) use discovery::{va_vpp_input_modifiers, va_vpp_input_profiles};
pub(crate) use encoder::GStreamerVideoEncoder;
#[cfg(test)]
pub(crate) use frame::GStreamerVideoFrame;
pub(crate) use frame::dmabuf_caps;
#[cfg(test)]
pub(crate) use frame::validate_dmabuf_buffer;
pub(crate) use pipeline_util::va_vpp_output_caps;
#[cfg(test)]
pub(crate) use pipeline_util::{
    H264_KEY_INT_MAX, configure_low_latency_encoder, create_required_element, h264_rtp_caps,
};
pub(crate) use recorder::record_frames_to_mp4;
pub(crate) use rtp::GStreamerRtpPacket;
pub(crate) use va_surface::VaDmaBufAllocator;

/// Probes a hardware H.264 encoder factory name for a DMA-BUF frame layout.
pub(crate) fn hardware_h264_encoder_for(
    frame: &DmaBufFrame,
) -> eros::Result<gstreamer::glib::GString> {
    gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
    let modifier = frame
        .planes
        .first()
        .with_context(|| "NV12 DMA-BUF probe frame has no planes")?
        .modifier;
    let caps = dmabuf_caps(frame, modifier, None)?;
    let encoder_caps = match frame.format {
        DrmFourcc::Nv12 => caps,
        DrmFourcc::Xrgb8888 => {
            let vpp = gstreamer::ElementFactory::find("vapostproc")
                .with_context(|| "GStreamer VAAPI video postprocessor is unavailable")?;
            if !vpp.can_sink_all_caps(&caps) {
                eros::bail!("VAAPI video postprocessor rejects input caps {}", caps);
            }
            va_vpp_output_caps(&caps)?
        }
        format => eros::bail!("Unsupported H.264 encoder probe format: {:?}", format),
    };

    Ok(GStreamerVideoEncoder::select_hardware_h264_encoder(&encoder_caps)?.name())
}
