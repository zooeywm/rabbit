use eros::Context as _;
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{Cast as _, ElementExt as _, GstBinExtManual as _, GstObjectExt as _};

#[derive(Debug)]
pub(crate) struct GStreamerVideoFrame {
    buffer: gstreamer::Buffer,
}

impl TryFrom<gstreamer::Buffer> for GStreamerVideoFrame {
    type Error = eros::ErrorUnion;

    fn try_from(buffer: gstreamer::Buffer) -> Result<Self, Self::Error> {
        if buffer.n_memory() == 0 {
            eros::bail!("GStreamer video frame does not contain DMA-BUF memory");
        }

        for (index, memory) in buffer.iter_memories().enumerate() {
            if !memory.is_memory_type::<gstreamer_allocators::DmaBufMemory>() {
                eros::bail!("GStreamer video frame memory {} is not DMA-BUF", index);
            }
        }

        let Some(video) = buffer.meta::<gstreamer_video::VideoMeta>() else {
            eros::bail!("GStreamer DMA-BUF video frame is missing VideoMeta");
        };

        if video.format() != gstreamer_video::VideoFormat::DmaDrm {
            eros::bail!(
                "GStreamer DMA-BUF video frame has non-DRM format {}",
                video.format()
            );
        }

        Ok(Self { buffer })
    }
}

#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    element: gstreamer::Element,
    sink: gstreamer_app::AppSink,
    terminal_messages: flume::Receiver<gstreamer::Message>,
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
        let terminal_messages = terminal_messages(&pipeline)?;

        Ok(Self {
            pipeline,
            source,
            element,
            sink,
            terminal_messages,
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

    pub(crate) fn finish(&self) -> eros::Result<()> {
        self.source
            .end_of_stream()
            .with_context(|| "Failed to finish GStreamer H.264 encoding input")?;

        Ok(())
    }

    pub(crate) async fn wait_terminal(&self) -> eros::Result<()> {
        let message = self
            .terminal_messages
            .recv_async()
            .await
            .with_context(|| "GStreamer H.264 terminal message channel disconnected")?;

        terminal_message_result(&message)
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

fn terminal_messages(
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

fn terminal_message_result(message: &gstreamer::MessageRef) -> eros::Result<()> {
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
    use std::fs::File;

    use gstreamer::glib::prelude::ObjectExt as _;
    use gstreamer::prelude::{ElementExt as _, GstBinExt as _};
    use gstreamer_allocators::prelude::DmaBufAllocatorExtManual as _;

    use crate::infra::platform::{
        GStreamerVideoEncoder, GStreamerVideoFrame, video_encoder::gstreamer::h264_rtp_caps,
    };

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

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn receives_gstreamer_eos_and_error_messages_asynchronously() {
        gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let encoder = GStreamerVideoEncoder::new(&input_caps, 1_200)
            .expect("The hardware H.264 pipeline should be created");
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        encoder
            .pipeline
            .post_message(
                gstreamer::message::Eos::builder()
                    .src(&encoder.pipeline)
                    .build(),
            )
            .expect("The test EOS message should be posted");
        runtime
            .block_on(encoder.wait_terminal())
            .expect("EOS should complete the pipeline normally");

        encoder
            .pipeline
            .post_message(
                gstreamer::message::Error::builder(
                    gstreamer::CoreError::Failed,
                    "test pipeline failure",
                )
                .src(&encoder.pipeline)
                .debug("test debug details")
                .build(),
            )
            .expect("The test error message should be posted");
        let error = runtime
            .block_on(encoder.wait_terminal())
            .expect_err("A GStreamer error message should fail the pipeline");
        assert!(error.to_string().contains("test pipeline failure"));
        assert!(error.to_string().contains("test debug details"));
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn finishes_hardware_h264_pipeline_through_appsrc() {
        gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let encoder = GStreamerVideoEncoder::new(&input_caps, 1_200)
            .expect("The hardware H.264 pipeline should be created");
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        encoder
            .start()
            .expect("The hardware H.264 pipeline should start");
        encoder
            .finish()
            .expect("The hardware H.264 input should accept EOS");
        runtime
            .block_on(encoder.wait_terminal())
            .expect("EOS should finish the hardware H.264 pipeline normally");
        encoder
            .stop()
            .expect("The hardware H.264 pipeline should stop");
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn accepts_dmabuf_video_frames() {
        gstreamer::init().expect("GStreamer should initialize before constructing a frame");

        let _frame = dmabuf_video_frame();
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn rejects_system_memory_video_frames() {
        gstreamer::init().expect("GStreamer should initialize before constructing a frame");
        let buffer = gstreamer::Buffer::from_slice([0_u8; 16]);

        GStreamerVideoFrame::try_from(buffer)
            .expect_err("The hardware encoder input should reject system memory");
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

    fn dmabuf_video_frame() -> GStreamerVideoFrame {
        const WIDTH: u32 = 16;
        const HEIGHT: u32 = 16;
        const Y_SIZE: usize = WIDTH as usize * HEIGHT as usize;
        const BUFFER_SIZE: usize = Y_SIZE + Y_SIZE / 2;

        let allocator = gstreamer_allocators::DmaBufAllocator::new();
        let file = File::open("/dev/zero").expect("The test DMA-BUF fd should open");
        let memory = unsafe {
            allocator
                .alloc_dmabuf(file, BUFFER_SIZE)
                .expect("The test fd should be wrapped as GStreamer DMA-BUF memory")
        };
        let mut buffer = gstreamer::Buffer::new();
        let buffer = buffer
            .get_mut()
            .expect("A newly allocated GStreamer buffer should be writable");
        buffer.append_memory(memory);
        gstreamer_video::VideoMeta::add_full(
            buffer,
            gstreamer_video::VideoFrameFlags::empty(),
            gstreamer_video::VideoFormat::DmaDrm,
            WIDTH,
            HEIGHT,
            &[0, Y_SIZE],
            &[WIDTH as i32, WIDTH as i32],
        )
        .expect("The test DMA-BUF should accept NV12 plane metadata");

        GStreamerVideoFrame::try_from(buffer.to_owned())
            .expect("The test buffer should satisfy the encoder input boundary")
    }
}
