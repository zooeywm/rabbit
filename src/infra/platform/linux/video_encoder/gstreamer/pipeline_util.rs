//! Shared GStreamer pipeline helpers for the H.264 encoder.

use eros::Context as _;
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{ElementExt as _, GObjectExtManualGst as _, GstObjectExt as _};

pub(crate) fn terminal_messages(
    pipeline: &gstreamer::Pipeline,
) -> eros::Result<flume::Receiver<gstreamer::Message>> {
    let Some(bus) = pipeline.bus() else {
        eros::bail!("GStreamer H.264 encoding pipeline has no Bus");
    };
    let (sender, receiver) = flume::bounded(1);

    bus.set_sync_handler(move |_, message| {
        if matches!(
            message.view(),
            gstreamer::MessageView::Error(_) | gstreamer::MessageView::Eos(_)
        ) {
            let _ = sender.try_send(message.to_owned());
        }

        gstreamer::BusSyncReply::Drop
    });

    Ok(receiver)
}

pub(crate) fn terminal_message_result(message: &gstreamer::MessageRef) -> eros::Result<()> {
    match message.view() {
        gstreamer::MessageView::Eos(_) => Ok(()),
        gstreamer::MessageView::Error(error) => {
            let source = match error.src() {
                Some(source) => source.path_string().to_string(),
                None => String::from("unknown source"),
            };
            let message = error.error();

            match error.debug() {
                Some(debug) => eros::bail!(
                    "GStreamer H.264 pipeline failed at {}: {}; debug: {}",
                    source,
                    message,
                    debug
                ),
                None => eros::bail!("GStreamer H.264 pipeline failed at {}: {}", source, message),
            }
        }
        _ => eros::bail!("GStreamer terminal channel received a non-terminal message"),
    }
}

pub(crate) fn rtp_mtu(max_rtp_packet_size: usize) -> eros::Result<u32> {
    let Ok(rtp_mtu) = u32::try_from(max_rtp_packet_size) else {
        eros::bail!(
            "GStreamer RTP packet size exceeds u32: {}",
            max_rtp_packet_size
        );
    };

    if rtp_mtu < 28 {
        eros::bail!(
            "GStreamer RTP packet size must be at least 28 bytes, got {}",
            max_rtp_packet_size
        );
    }

    Ok(rtp_mtu)
}

pub(crate) fn create_required_element(
    factory: &str,
    name: &str,
) -> eros::Result<gstreamer::Element> {
    Ok(gstreamer::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("Failed to create required GStreamer element {factory}"))?)
}

pub(crate) fn create_pipeline_stage_queue(
    name: &str,
    max_size_buffers: u32,
) -> eros::Result<gstreamer::Element> {
    let queue = create_required_element("queue", name)?;
    queue.set_property("max-size-buffers", max_size_buffers);
    queue.set_property("max-size-bytes", 0_u32);
    queue.set_property("max-size-time", 0_u64);

    Ok(queue)
}

pub(crate) fn va_vpp_output_caps(input: &gstreamer::CapsRef) -> eros::Result<gstreamer::Caps> {
    let structure = input
        .structure(0)
        .with_context(|| "VAAPI VPP input caps are empty")?;
    let width = structure
        .get::<i32>("width")
        .with_context(|| "VAAPI VPP input caps do not contain a fixed width")?;
    let height = structure
        .get::<i32>("height")
        .with_context(|| "VAAPI VPP input caps do not contain a fixed height")?;
    let framerate = structure
        .get::<gstreamer::Fraction>("framerate")
        .with_context(|| "VAAPI VPP input caps do not contain a fixed framerate")?;

    Ok(gstreamer::Caps::builder("video/x-raw")
        .features(["memory:VAMemory"])
        .field("format", "NV12")
        .field("width", width)
        .field("height", height)
        .field("framerate", framerate)
        .field("colorimetry", "bt709")
        .build())
}

pub(crate) const H264_BITRATE_KBPS: u32 = 50_000;
pub(crate) const H264_CPB_SIZE_KBITS: u32 = 5_000;
pub(crate) const H264_KEY_INT_MAX: u32 = 1_024;

pub(crate) fn configure_low_latency_encoder(encoder: &gstreamer::Element) {
    let is_vaapi = encoder
        .factory()
        .is_some_and(|factory| factory.name().starts_with("va"));
    if !is_vaapi {
        return;
    }

    encoder.set_property("b-frames", 0_u32);
    encoder.set_property("ref-frames", 1_u32);
    encoder.set_property("target-usage", 7_u32);
    if encoder.find_property("mbbrc").is_some() {
        encoder.set_property_from_str("mbbrc", "disabled");
    }
    encoder.set_property_from_str("rate-control", "cbr");
    encoder.set_property("bitrate", H264_BITRATE_KBPS);
    encoder.set_property("cpb-size", H264_CPB_SIZE_KBITS);
    encoder.set_property("key-int-max", H264_KEY_INT_MAX);
}

pub(crate) fn is_hardware_video_encoder(factory: &gstreamer::ElementFactory) -> bool {
    let Some(class) = factory.metadata("klass") else {
        return false;
    };

    ["Encoder", "Video", "Hardware"]
        .into_iter()
        .all(|required| class.split('/').any(|component| component == required))
}

pub(crate) fn h264_rtp_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", "H264")
        .field("clock-rate", 90_000_i32)
        .build()
}
