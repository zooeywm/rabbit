use eros::Context as _;
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{Cast as _, ElementExt as _, GstBinExtManual as _, GstObjectExt as _};

#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    element: gstreamer::Element,
    sink: gstreamer_app::AppSink,
}

impl GStreamerVideoEncoder {
    pub(crate) fn new(
        input_caps: &gstreamer::Caps,
        max_rtp_packet_size: usize,
    ) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let rtp_mtu = rtp_mtu(max_rtp_packet_size)?;
        let factory = Self::select_hardware_h264_encoder(input_caps)?;
        let factory_name = factory.name();
        let element = factory
            .create()
            .name("h264-encoder")
            .build()
            .with_context(|| {
                format!(
                    "Failed to create GStreamer hardware H.264 encoder element from {}",
                    factory_name
                )
            })?;
        let source = create_required_element("appsrc", "video-input")?;
        let Ok(source) = source.downcast::<gstreamer_app::AppSrc>() else {
            eros::bail!("GStreamer appsrc factory returned an unexpected element type");
        };
        source.set_caps(Some(input_caps));
        source.set_format(gstreamer::Format::Time);
        source.set_is_live(true);

        let parser = create_required_element("h264parse", "h264-parser")?;
        let payloader = create_required_element("rtph264pay", "rtp-payloader")?;
        payloader.set_property("mtu", rtp_mtu);
        let sink = create_required_element("appsink", "rtp-output")?;
        let Ok(sink) = sink.downcast::<gstreamer_app::AppSink>() else {
            eros::bail!("GStreamer appsink factory returned an unexpected element type");
        };
        sink.set_caps(Some(&h264_rtp_caps()));
        sink.set_async(false);
        sink.set_sync(false);

        let pipeline = gstreamer::Pipeline::new();
        let elements = [
            source.upcast_ref(),
            &element,
            &parser,
            &payloader,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(elements)
            .with_context(|| "Failed to add H.264 encoding elements to GStreamer pipeline")?;
        gstreamer::Element::link_many(elements)
            .with_context(|| "Failed to link GStreamer H.264 RTP encoding pipeline")?;

        Ok(Self {
            pipeline,
            source,
            element,
            sink,
        })
    }

    pub(crate) fn start(&self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Playing)
            .with_context(|| "Failed to start GStreamer H.264 encoding pipeline")?;

        Ok(())
    }

    pub(crate) fn stop(&self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Null)
            .with_context(|| "Failed to stop GStreamer H.264 encoding pipeline")?;

        Ok(())
    }

    fn find_hardware_h264_encoders() -> eros::Result<Vec<gstreamer::ElementFactory>> {
        let h264_caps = gstreamer::Caps::builder("video/x-h264").build();

        let factories = gstreamer::ElementFactory::factories_with_type(
            gstreamer::ElementFactoryType::ENCODER | gstreamer::ElementFactoryType::HARDWARE,
            gstreamer::Rank::NONE,
        )
        .into_iter()
        .filter(|factory| {
            is_hardware_video_encoder(factory) && factory.can_src_any_caps(&h264_caps)
        })
        .collect::<Vec<_>>();

        if factories.is_empty() {
            eros::bail!("No GStreamer hardware H.264 encoder is available");
        }

        Ok(factories)
    }

    fn select_hardware_h264_encoder(
        input_caps: &gstreamer::CapsRef,
    ) -> eros::Result<gstreamer::ElementFactory> {
        if !Self::is_nv12_dmabuf_input_caps(input_caps) {
            eros::bail!(
                "First-version H.264 encoding requires NV12 DMA-BUF input caps, got {}",
                input_caps
            );
        }

        let factory = Self::find_hardware_h264_encoders()?
            .into_iter()
            .find(|factory| factory.can_sink_all_caps(input_caps));

        let Some(factory) = factory else {
            eros::bail!(
                "No GStreamer hardware H.264 encoder accepts input caps {}",
                input_caps
            );
        };

        Ok(factory)
    }

    fn is_nv12_dmabuf_input_caps(caps: &gstreamer::CapsRef) -> bool {
        if caps.size() != 1 || !caps.is_fixed() {
            return false;
        }

        let Some((structure, features)) = caps.iter_with_features().next() else {
            return false;
        };

        features.contains("memory:DMABuf")
            && structure
                .get::<&str>("format")
                .is_ok_and(|format| format == "DMA_DRM")
            && structure
                .get::<&str>("drm-format")
                .is_ok_and(|format| format == "NV12" || format.starts_with("NV12:"))
    }
}

fn rtp_mtu(max_rtp_packet_size: usize) -> eros::Result<u32> {
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

fn create_required_element(factory: &str, name: &str) -> eros::Result<gstreamer::Element> {
    Ok(gstreamer::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("Failed to create required GStreamer element {factory}"))?)
}

fn h264_rtp_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("application/x-rtp")
        .field("media", "video")
        .field("encoding-name", "H264")
        .field("clock-rate", 90_000_i32)
        .build()
}

fn is_hardware_video_encoder(factory: &gstreamer::ElementFactory) -> bool {
    let Some(class) = factory.metadata("klass") else {
        return false;
    };

    ["Encoder", "Video", "Hardware"]
        .into_iter()
        .all(|required| class.split('/').any(|component| component == required))
}

#[cfg(test)]
mod tests {
    use gstreamer::glib::prelude::ObjectExt as _;
    use gstreamer::prelude::{ElementExt as _, GstBinExt as _};

    use crate::infra::platform::{GStreamerVideoEncoder, video_encoder::gstreamer::h264_rtp_caps};

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn finds_a_registered_hardware_h264_encoder() {
        gstreamer::init().expect("GStreamer should initialize before inspecting encoders");
        let factories = GStreamerVideoEncoder::find_hardware_h264_encoders()
            .expect("At least one hardware H.264 encoder should be registered");

        for factory in factories {
            let class = factory
                .metadata("klass")
                .expect("Hardware encoder factory should expose klass metadata");
            assert!(class.split('/').any(|component| component == "Encoder"));
            assert!(class.split('/').any(|component| component == "Video"));
            assert!(class.split('/').any(|component| component == "Hardware"));
            assert!(factory.can_src_any_caps(&h264_caps()));
        }
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn creates_a_hardware_h264_rtp_pipeline_for_nv12_dmabuf_input() {
        const MAX_RTP_PACKET_SIZE: usize = 1_200;

        gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let encoder = GStreamerVideoEncoder::new(&input_caps, MAX_RTP_PACKET_SIZE)
            .expect("A hardware H.264 encoder element should be created for NV12 DMA-BUF input");
        let factory = encoder
            .element
            .factory()
            .expect("The created encoder element should retain its factory");

        assert!(factory.can_sink_all_caps(&input_caps));
        assert!(factory.can_src_any_caps(&h264_caps()));
        assert_eq!(
            encoder
                .source
                .caps()
                .expect("The pipeline appsrc should retain its input caps"),
            input_caps
        );
        assert_eq!(
            encoder
                .sink
                .caps()
                .expect("The pipeline appsink should retain its output caps"),
            h264_rtp_caps()
        );
        assert_eq!(
            encoder
                .pipeline
                .by_name("rtp-payloader")
                .expect("The encoding pipeline should contain its RTP payloader")
                .property::<u32>("mtu"),
            1_200
        );

        for name in [
            "video-input",
            "h264-encoder",
            "h264-parser",
            "rtp-payloader",
            "rtp-output",
        ] {
            encoder
                .pipeline
                .by_name(name)
                .expect("The encoding pipeline should contain every required element");
        }
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn rejects_p010_dmabuf_input() {
        gstreamer::init().expect("GStreamer should initialize before constructing input caps");
        let input_caps = gstreamer::Caps::builder("video/x-raw")
            .features(["memory:DMABuf"])
            .field("format", "DMA_DRM")
            .field("drm-format", "P010")
            .build();

        GStreamerVideoEncoder::new(&input_caps, 1_200)
            .expect_err("The first-version encoder should reject P010 input");
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn rejects_rtp_packet_size_below_payloader_minimum() {
        gstreamer::init().expect("GStreamer should initialize before constructing input caps");
        let input_caps = gstreamer::Caps::builder("video/x-raw")
            .features(["memory:DMABuf"])
            .field("format", "DMA_DRM")
            .field("drm-format", "NV12")
            .build();

        let error = GStreamerVideoEncoder::new(&input_caps, 27)
            .expect_err("The RTP payloader should reject packet sizes below 28 bytes");

        assert!(error.to_string().contains("at least 28 bytes"));
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn starts_and_stops_hardware_h264_pipeline() {
        gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let encoder = GStreamerVideoEncoder::new(&input_caps, 1_200)
            .expect("The hardware H.264 pipeline should be created");

        encoder
            .start()
            .expect("The hardware H.264 pipeline should start");
        let (started, current, _) = encoder
            .pipeline
            .state(gstreamer::ClockTime::from_seconds(5));
        started.expect("The hardware H.264 pipeline should finish starting");
        assert_eq!(current, gstreamer::State::Playing);

        encoder
            .stop()
            .expect("The hardware H.264 pipeline should stop");
        let (stopped, current, _) = encoder
            .pipeline
            .state(gstreamer::ClockTime::from_seconds(5));
        stopped.expect("The hardware H.264 pipeline should finish stopping");
        assert_eq!(current, gstreamer::State::Null);
    }

    fn registered_nv12_dmabuf_input_caps() -> gstreamer::Caps {
        GStreamerVideoEncoder::find_hardware_h264_encoders()
            .expect("At least one hardware H.264 encoder should be registered")
            .into_iter()
            .flat_map(|factory| factory.static_pad_templates())
            .filter(|template| template.direction() == gstreamer::PadDirection::Sink)
            .find_map(|template| {
                let caps = template.caps();

                caps.iter_with_features()
                    .map(|(structure, features)| {
                        gstreamer::Caps::builder_full()
                            .structure_with_features(structure.to_owned(), features.to_owned())
                            .build()
                    })
                    .map(|mut caps| {
                        caps.fixate();
                        caps
                    })
                    .find(|caps| GStreamerVideoEncoder::is_nv12_dmabuf_input_caps(caps))
            })
            .expect("A hardware H.264 encoder should advertise NV12 DMA-BUF input caps")
    }

    fn h264_caps() -> gstreamer::Caps {
        gstreamer::Caps::builder("video/x-h264").build()
    }
}
