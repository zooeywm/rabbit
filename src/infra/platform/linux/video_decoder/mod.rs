use std::future::Future;

use eros::Context as _;
use futures_util::StreamExt as _;
use gstreamer::glib::prelude::Cast as _;
use gstreamer::prelude::{GstBinExtManual as _, GstObjectExt as _};

use crate::{
    infra::platform::dma_buf::DmaBufFrame,
    kernel::{session::ReceivedVideoFrame, video_decoder::VideoDecoder},
};

#[derive(Debug)]
pub(crate) struct GStreamerVideoDecoder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    decoder: gstreamer::Element,
    sink: gstreamer_app::AppSink,
}

impl GStreamerVideoDecoder {
    fn create() -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let input_caps = h264_rtp_caps();
        let output_caps = decoded_dma_buf_caps();
        let factory = Self::select_hardware_h264_decoder(&output_caps)?;
        let factory_name = factory.name();
        let decoder = factory
            .create()
            .name("h264-decoder")
            .build()
            .with_context(|| {
                format!(
                    "Failed to create GStreamer hardware H.264 decoder element from {}",
                    factory_name
                )
            })?;

        let source = create_required_element("appsrc", "rtp-input")?;
        let Ok(source) = source.downcast::<gstreamer_app::AppSrc>() else {
            eros::bail!("GStreamer appsrc factory returned an unexpected element type");
        };
        source.set_caps(Some(&input_caps));
        source.set_format(gstreamer::Format::Time);
        source.set_is_live(true);

        let depayloader = create_required_element("rtph264depay", "rtp-depayloader")?;
        let parser = create_required_element("h264parse", "h264-parser")?;
        let sink = create_required_element("appsink", "decoded-output")?;
        let Ok(sink) = sink.downcast::<gstreamer_app::AppSink>() else {
            eros::bail!("GStreamer appsink factory returned an unexpected element type");
        };
        sink.set_caps(Some(&output_caps));
        sink.set_async(false);
        sink.set_sync(false);
        sink.set_max_buffers(1);
        sink.set_drop(true);

        let pipeline = gstreamer::Pipeline::new();
        let elements = [
            source.upcast_ref(),
            &depayloader,
            &parser,
            &decoder,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(elements)
            .with_context(|| "Failed to add H.264 decoding elements to GStreamer pipeline")?;
        gstreamer::Element::link_many(elements)
            .with_context(|| "Failed to link GStreamer H.264 DMA-BUF decoding pipeline")?;

        Ok(Self {
            pipeline,
            source,
            decoder,
            sink,
        })
    }

    fn find_hardware_h264_decoders() -> eros::Result<Vec<gstreamer::ElementFactory>> {
        let input_caps = h264_access_unit_caps();
        let output_caps = decoded_dma_buf_caps();
        let factories = gstreamer::ElementFactory::factories_with_type(
            gstreamer::ElementFactoryType::DECODER | gstreamer::ElementFactoryType::HARDWARE,
            gstreamer::Rank::NONE,
        )
        .into_iter()
        .filter(|factory| {
            is_hardware_video_decoder(factory)
                && factory.can_sink_any_caps(&input_caps)
                && factory.can_src_any_caps(&output_caps)
        })
        .collect::<Vec<_>>();

        if factories.is_empty() {
            eros::bail!("No GStreamer hardware H.264 decoder with DMA-BUF output is available");
        }

        Ok(factories)
    }

    fn select_hardware_h264_decoder(
        output_caps: &gstreamer::CapsRef,
    ) -> eros::Result<gstreamer::ElementFactory> {
        let factory = Self::find_hardware_h264_decoders()?
            .into_iter()
            .find(|factory| factory.can_src_any_caps(output_caps));

        let Some(factory) = factory else {
            eros::bail!(
                "No GStreamer hardware H.264 decoder can produce output caps {}",
                output_caps
            );
        };

        Ok(factory)
    }
}

impl VideoDecoder for GStreamerVideoDecoder {
    type Input = ReceivedVideoFrame;
    type Frame = DmaBufFrame;

    fn run<Inputs, PresentFrame, PresentFuture>(
        mut inputs: Inputs,
        _present_frame: PresentFrame,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        async move {
            while let Some(input) = inputs.next().await {
                input?;
            }

            Ok(())
        }
    }
}

fn h264_rtp_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", "H264")
        .field("clock-rate", 90_000_i32)
        .build()
}

fn h264_access_unit_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("video/x-h264")
        .field("alignment", "au")
        .build()
}

fn decoded_dma_buf_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .build()
}

fn is_hardware_video_decoder(factory: &gstreamer::ElementFactory) -> bool {
    let Some(class) = factory.metadata("klass") else {
        return false;
    };

    ["Decoder", "Video", "Hardware"]
        .into_iter()
        .all(|required| class.split('/').any(|component| component == required))
}

fn create_required_element(factory: &str, name: &str) -> eros::Result<gstreamer::Element> {
    Ok(gstreamer::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("Failed to create required GStreamer element {factory}"))?)
}

#[cfg(test)]
mod tests {
    use gstreamer::prelude::{ElementExt as _, GstBinExt as _};

    use crate::{
        infra::platform::video_decoder::{
            GStreamerVideoDecoder, decoded_dma_buf_caps, h264_rtp_caps,
        },
        kernel::video_decoder::VideoDecoder,
    };

    #[test]
    fn empty_decoder_accepts_its_platform_boundary() {
        let inputs = futures_util::stream::empty();
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(GStreamerVideoDecoder::run(inputs, |_| {
                std::future::ready(Ok(()))
            }))
            .expect("Empty Linux video decoder should finish cleanly");
    }

    #[test]
    fn decoded_output_caps_require_dma_buf_memory() {
        gstreamer::init().expect("GStreamer should initialize before constructing output caps");
        let caps = decoded_dma_buf_caps();
        let (structure, features) = caps
            .iter_with_features()
            .next()
            .expect("Decoded output caps should contain one structure");

        assert!(features.contains("memory:DMABuf"));
        assert_eq!(
            structure
                .get::<&str>("format")
                .expect("Decoded output caps should contain a format"),
            "DMA_DRM"
        );
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn creates_hardware_h264_dma_buf_pipeline() {
        let decoder = GStreamerVideoDecoder::create()
            .expect("A hardware H.264 DMA-BUF decoder should be available");
        let factory = decoder
            .decoder
            .factory()
            .expect("The selected decoder should have a factory");

        assert!(factory.can_src_any_caps(&decoded_dma_buf_caps()));
        assert_eq!(
            decoder
                .source
                .caps()
                .expect("Decoder appsrc should retain its RTP caps"),
            h264_rtp_caps()
        );
        assert_eq!(
            decoder
                .sink
                .caps()
                .expect("Decoder appsink should retain its DMA-BUF caps"),
            decoded_dma_buf_caps()
        );

        for name in [
            "rtp-input",
            "rtp-depayloader",
            "h264-parser",
            "h264-decoder",
            "decoded-output",
        ] {
            decoder
                .pipeline
                .by_name(name)
                .expect("The decoding pipeline should contain every required element");
        }
    }
}
