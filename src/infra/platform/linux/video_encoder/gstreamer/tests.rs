use std::{
    cell::Cell,
    fs::File,
    future::{Future, ready},
    os::fd::OwnedFd,
    path::PathBuf,
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use compio::runtime::fd::PollFd;
use drm::buffer::{DrmFourcc, DrmModifier};
use gstreamer::glib::prelude::{Cast as _, ObjectExt as _};
use gstreamer::prelude::{
    ElementExt as _, GstBinExt as _, GstBinExtManual as _, PadExtManual as _,
};
use gstreamer_allocators::prelude::DmaBufAllocatorExtManual as _;
use tracing_subscriber::{
    filter::{LevelFilter, Targets},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

use crate::{
    infra::{
        WorkerReaper,
        platform::{
            dma_buf::{DmaBufFrame, DmaBufLease, DmaBufObject, DmaBufPlane},
            frame_pipeline::{
                GbmFramePipelineFrame, GbmFramePipelineManager, GbmFramePipelineManagerState,
            },
            gpu::{GpuContext, GpuDevice},
            screen_capture::{
                KmsCaptureLease, KmsFrameReceiver, KmsScreenCaptureManager,
                KmsScreenCaptureManagerState,
            },
            video_encoder::gstreamer::{
                GStreamerRtpPacket, GStreamerVideoEncoder, GStreamerVideoFrame, H264_KEY_INT_MAX,
                configure_low_latency_encoder, create_required_element, dmabuf_caps, h264_rtp_caps,
                va_vpp_input_modifier, va_vpp_output_caps, validate_dmabuf_buffer,
            },
        },
    },
    kernel::{
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        geometry::{FrameRate, PixelSize},
        screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
        screen_manager::{
            Screen, ScreenId, ScreenLayout, ScreenLayoutManager, ScreenRect, ScreenTransform,
        },
    },
};

struct HostVideoTestDeps {
    capture: KmsScreenCaptureManagerState,
    pipeline: GbmFramePipelineManagerState,
    screens: Vec<Screen>,
}

#[derive(Debug, Default)]
struct VaVppProbe {
    submitted: Option<Instant>,
    vpp_entered: Option<Instant>,
    vpp_completed: Option<Instant>,
}

struct TimedFrames<Frames> {
    frames: Frames,
    duration: Duration,
    deadline: Option<Pin<Box<dyn Future<Output = ()>>>>,
}

impl<Frames> TimedFrames<Frames> {
    fn new(frames: Frames, duration: Duration) -> Self {
        Self {
            frames,
            duration,
            deadline: None,
        }
    }
}

impl<Frames> futures_core::Stream for TimedFrames<Frames>
where
    Frames: futures_core::Stream + Unpin,
{
    type Item = Frames::Item;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(deadline) = &mut self.deadline
            && deadline.as_mut().poll(context).is_ready()
        {
            return Poll::Ready(None);
        }

        let frame = Pin::new(&mut self.frames).poll_next(context);
        if self.deadline.is_none() && matches!(frame, Poll::Ready(Some(_))) {
            self.deadline = Some(Box::pin(compio::time::sleep(self.duration)));
        }

        frame
    }
}

impl AsRef<KmsScreenCaptureManagerState> for HostVideoTestDeps {
    fn as_ref(&self) -> &KmsScreenCaptureManagerState {
        &self.capture
    }
}

impl AsMut<KmsScreenCaptureManagerState> for HostVideoTestDeps {
    fn as_mut(&mut self) -> &mut KmsScreenCaptureManagerState {
        &mut self.capture
    }
}

impl AsRef<GbmFramePipelineManagerState> for HostVideoTestDeps {
    fn as_ref(&self) -> &GbmFramePipelineManagerState {
        &self.pipeline
    }
}

impl AsMut<GbmFramePipelineManagerState> for HostVideoTestDeps {
    fn as_mut(&mut self) -> &mut GbmFramePipelineManagerState {
        &mut self.pipeline
    }
}

impl ScreenLayoutManager for HostVideoTestDeps {
    fn refresh(&mut self) -> eros::Result<()> {
        Ok(())
    }

    fn screens(&self) -> &[Screen] {
        &self.screens
    }

    fn screen(&self, id: &ScreenId) -> Option<&Screen> {
        self.screens.iter().find(|screen| screen.id == *id)
    }

    fn primary_screen(&self) -> eros::Result<&Screen> {
        Ok(self
            .screens
            .first()
            .expect("Host video smoke test should contain one screen"))
    }
}

impl ScreenCaptureManager for HostVideoTestDeps {
    type Lease = KmsCaptureLease;
    type Receiver = KmsFrameReceiver;

    fn acquire(
        &mut self,
        screen_id: &ScreenId,
    ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>> {
        KmsScreenCaptureManager::inj_ref_mut(self).acquire(screen_id)
    }
}

#[test]
fn dma_buf_caps_distinguish_full_range_rgb_from_limited_range_nv12() {
    gstreamer::init().expect("GStreamer should initialize before constructing DMA-BUF caps");

    let rgb = dmabuf_caps(
        &colorimetry_test_frame(DrmFourcc::Xrgb8888),
        DrmModifier::Invalid,
        Some(FrameRate::new(120, 1).expect("Test frame rate should be valid")),
    )
    .expect("XRGB DMA-BUF caps should be constructed");
    let nv12 = dmabuf_caps(
        &colorimetry_test_frame(DrmFourcc::Nv12),
        DrmModifier::Invalid,
        Some(FrameRate::new(120, 1).expect("Test frame rate should be valid")),
    )
    .expect("NV12 DMA-BUF caps should be constructed");

    let rgb_colorimetry = caps_colorimetry(&rgb);
    assert_eq!(
        rgb_colorimetry.range(),
        gstreamer_video::VideoColorRange::Range0_255
    );
    assert_eq!(
        rgb_colorimetry.matrix(),
        gstreamer_video::VideoColorMatrix::Rgb
    );

    let nv12_colorimetry = caps_colorimetry(&nv12);
    assert_eq!(
        nv12_colorimetry.range(),
        gstreamer_video::VideoColorRange::Range16_235
    );
    assert_eq!(
        nv12_colorimetry.matrix(),
        gstreamer_video::VideoColorMatrix::Bt709
    );
}

#[test]
fn va_vpp_output_is_bt709_limited_range() {
    gstreamer::init().expect("GStreamer should initialize before constructing VPP caps");
    let input = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", "XR24")
        .field("width", 1920_i32)
        .field("height", 1080_i32)
        .field("framerate", gstreamer::Fraction::new(120, 1))
        .build();

    let output = va_vpp_output_caps(&input)
        .expect("Fixed XRGB input caps should produce fixed VA VPP output caps");
    let structure = output
        .structure(0)
        .expect("VA VPP output caps should contain one structure");
    let colorimetry = structure
        .get::<&str>("colorimetry")
        .expect("VA VPP output caps should declare colorimetry")
        .parse::<gstreamer_video::VideoColorimetry>()
        .expect("BT.709 colorimetry should parse");

    assert_eq!(
        colorimetry.range(),
        gstreamer_video::VideoColorRange::Range16_235
    );
    assert_eq!(
        colorimetry.matrix(),
        gstreamer_video::VideoColorMatrix::Bt709
    );
}

fn colorimetry_test_frame(format: DrmFourcc) -> DmaBufFrame {
    DmaBufFrame {
        size: PixelSize {
            width: 1920,
            height: 1080,
        },
        format,
        objects: Vec::new(),
        planes: Vec::new(),
        readiness_fence: None,
        lease: None,
        va_backing: None,
    }
}

fn caps_colorimetry(caps: &gstreamer::CapsRef) -> gstreamer_video::VideoColorimetry {
    caps.structure(0)
        .expect("Video caps should contain one structure")
        .get::<&str>("colorimetry")
        .expect("Video caps should declare colorimetry")
        .parse()
        .expect("Video colorimetry should parse")
}

#[test]
#[ignore = "run through scripts/test-host-video"]
fn streams_several_host_video_frames() {
    const REQUIRED_ENCODED_FRAMES: u64 = 3;
    const MAX_RTP_PACKET_SIZE: usize = 1_200;
    const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

    init_host_video_tracing();
    let screen_name = std::env::var("RABBIT_KMS_SCREEN")
        .expect("RABBIT_KMS_SCREEN should name the DRM connector to capture");
    let run_seconds = std::env::var("RABBIT_HOST_VIDEO_TEST_SECONDS")
        .expect("RABBIT_HOST_VIDEO_TEST_SECONDS should specify the run duration")
        .parse::<u64>()
        .expect("RABBIT_HOST_VIDEO_TEST_SECONDS should be a positive integer");
    assert!(
        run_seconds > 0,
        "Host video test duration should be positive"
    );
    let run_duration = Duration::from_secs(run_seconds);
    let test_timeout = run_duration
        .checked_add(SHUTDOWN_TIMEOUT)
        .expect("Host video test timeout should fit Duration");
    let (source_size, source_frame_rate) = host_video_test_source_geometry(&screen_name);
    let target_size = host_video_test_target_size(source_size);
    let target_frame_rate = host_video_test_target_frame_rate(source_frame_rate);
    eprintln!(
        "Host video test source: {}x{} @ {}/{} fps, target: {}x{} @ {}/{} fps",
        source_size.width,
        source_size.height,
        source_frame_rate.numerator(),
        source_frame_rate.denominator(),
        target_size.width,
        target_size.height,
        target_frame_rate.numerator(),
        target_frame_rate.denominator(),
    );
    let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");
    let encoded_frames = Rc::new(Cell::new(0_u64));
    let encoded_frames_for_callback = Rc::clone(&encoded_frames);
    let rtp_packets = Rc::new(Cell::new(0_u64));
    let rtp_packets_for_callback = Rc::clone(&rtp_packets);

    runtime.block_on(async {
        let (_reaper, reaper_handle) =
            WorkerReaper::new().expect("Test worker reaper should start");
        let mut deps = HostVideoTestDeps {
            capture: KmsScreenCaptureManagerState::new(
                true,
                Duration::from_secs(2),
                reaper_handle.clone(),
                crate::infra::platform::video_encoder::va_vpp_input_profiles,
            ),
            pipeline: GbmFramePipelineManagerState::new(reaper_handle),
            screens: vec![host_video_test_screen(
                screen_name,
                source_size,
                source_frame_rate,
            )],
        };
        let frames = GbmFramePipelineManager::inj_ref_mut(&mut deps)
            .subscribe(
                &ScreenId(0),
                FramePipelineParameters {
                    frame_size: target_size,
                },
                target_frame_rate,
                crate::kernel::frame_pipeline::FrameDelivery::Latest,
            )
            .expect("Host video frame pipeline should start");
        let frames = TimedFrames::new(frames, run_duration);
        let encoding = GStreamerVideoEncoder::run_inner(
            frames,
            futures_util::stream::pending(),
            target_frame_rate,
            MAX_RTP_PACKET_SIZE,
            move |packet| {
                assert!(
                    packet.payload_len() <= MAX_RTP_PACKET_SIZE,
                    "Encoded RTP packet should respect the transport packet size"
                );
                rtp_packets_for_callback.set(rtp_packets_for_callback.get() + 1);
                if packet.is_frame_end() {
                    encoded_frames_for_callback.set(encoded_frames_for_callback.get() + 1);
                }

                ready(Ok::<(), eros::ErrorUnion>(()))
            },
        );

        let result = compio::time::timeout(test_timeout, encoding).await;
        result
            .expect("Host video smoke test should finish within its shutdown timeout")
            .expect("Host video chain should encode H.264 RTP frames");
    });

    let min_frames = host_video_test_min_encoded_frames(target_frame_rate, run_seconds)
        .max(REQUIRED_ENCODED_FRAMES);
    assert!(
        encoded_frames.get() >= min_frames,
        "Host video chain should encode at least {min_frames} frames for {}/{} fps over {run_seconds}s, got {}",
        target_frame_rate.numerator(),
        target_frame_rate.denominator(),
        encoded_frames.get()
    );
    assert!(
        rtp_packets.get() > 0,
        "Host video chain should produce RTP packets"
    );
}

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
    let encoder = GStreamerVideoEncoder::create(&input_caps, MAX_RTP_PACKET_SIZE, None)
        .expect("A hardware H.264 encoder element should be created for NV12 DMA-BUF input");
    let factory = encoder
        .pipeline
        .by_name("h264-encoder")
        .expect("The pipeline should retain its encoder element")
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
            .pipeline
            .by_name("rtp-output")
            .expect("The encoding pipeline should retain its RTP appsink")
            .downcast::<gstreamer_app::AppSink>()
            .expect("The RTP output element should remain an appsink")
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
        "encoded-output-queue",
        "h264-parser",
        "rtp-payloader",
        "rtp-output",
    ] {
        encoder
            .pipeline
            .by_name(name)
            .expect("The encoding pipeline should contain every required element");
    }
    let encoded_output_queue = encoder
        .pipeline
        .by_name("encoded-output-queue")
        .expect("The encoding pipeline should contain its encoded-output queue");
    assert_eq!(encoded_output_queue.property::<u32>("max-size-buffers"), 2);
    assert_eq!(encoded_output_queue.property::<u32>("max-size-bytes"), 0);
    assert_eq!(encoded_output_queue.property::<u64>("max-size-time"), 0);
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

    GStreamerVideoEncoder::create(&input_caps, 1_200, None)
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

    let error = GStreamerVideoEncoder::create(&input_caps, 27, None)
        .expect_err("The RTP payloader should reject packet sizes below 28 bytes");

    assert!(error.to_string().contains("at least 28 bytes"));
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn starts_and_stops_hardware_h264_pipeline() {
    gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
    let input_caps = registered_nv12_dmabuf_input_caps();
    let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, None)
        .expect("The hardware H.264 pipeline should be created");
    assert_eq!(
        encoder
            .pipeline
            .by_name("h264-encoder")
            .expect("The pipeline should retain its hardware encoder")
            .property::<u32>("key-int-max"),
        H264_KEY_INT_MAX
    );

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
fn requests_a_key_frame_from_the_running_hardware_encoder() {
    gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
    let input_caps = registered_nv12_dmabuf_input_caps();
    let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, None)
        .expect("The hardware H.264 pipeline should be created");

    encoder
        .start()
        .expect("The hardware H.264 pipeline should start");
    encoder
        .request_key_frame()
        .expect("The running hardware encoder should accept a force-key-unit request");
    encoder
        .stop()
        .expect("The hardware H.264 pipeline should stop");
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn receives_gstreamer_eos_and_error_messages_asynchronously() {
    gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
    let input_caps = registered_nv12_dmabuf_input_caps();
    let encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, None)
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
    let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, None)
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
fn closes_rtp_output_when_hardware_pipeline_reaches_eos() {
    gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
    let input_caps = registered_nv12_dmabuf_input_caps();
    let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, None)
        .expect("The hardware H.264 pipeline should be created");
    let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

    encoder
        .start()
        .expect("The hardware H.264 pipeline should start");
    encoder
        .finish()
        .expect("The hardware H.264 input should accept EOS");
    assert!(
        runtime
            .block_on(encoder.receive_packet())
            .expect("The H.264 RTP output should close normally")
            .is_none()
    );
    runtime
        .block_on(encoder.wait_terminal())
        .expect("EOS should finish the hardware H.264 pipeline normally");
    encoder
        .stop()
        .expect("The hardware H.264 pipeline should stop");
}

#[test]
fn gstreamer_buffer_retains_the_dma_buf_pool_lease() {
    gstreamer::init().expect("GStreamer should initialize before constructing a frame");
    let (release, released) = flume::unbounded();
    let frame =
        GStreamerVideoFrame::try_from(dmabuf_pipeline_frame(Some(DmaBufLease::new(7, release))))
            .expect("GStreamer should wrap the leased DMA-BUF frame");

    assert!(matches!(
        released.try_recv(),
        Err(flume::TryRecvError::Empty)
    ));
    drop(frame);
    let release = released
        .try_recv()
        .expect("Dropping the GStreamer buffer should release its DMA-BUF lease");
    assert_eq!(release.slot, 7);
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn accepts_dmabuf_video_frames() {
    gstreamer::init().expect("GStreamer should initialize before constructing a frame");

    let frame = dmabuf_video_frame();

    assert!(GStreamerVideoEncoder::is_nv12_dmabuf_input_caps(
        frame.input_caps()
    ));
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn creates_and_starts_encoder_from_first_frame() {
    gstreamer::init().expect("GStreamer should initialize before constructing a frame");
    let frame = dmabuf_video_frame();
    let input_caps = frame.input_caps().to_owned();
    let source_frame_rate = frame.input_signature.frame_rate;
    let mut encoder = GStreamerVideoEncoder::new(frame, source_frame_rate, 1_200)
        .expect("The first frame should create and start its hardware encoder");
    let (started, current, _) = encoder
        .pipeline
        .state(gstreamer::ClockTime::from_seconds(5));

    started.expect("The first frame should finish starting its hardware encoder");
    assert_eq!(current, gstreamer::State::Playing);
    assert_eq!(encoder.input_caps, input_caps);
    encoder.stop().expect("The first-frame encoder should stop");
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn submits_a_dmabuf_video_frame_to_appsrc() {
    gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
    let frame = dmabuf_video_frame();
    let mut encoder = GStreamerVideoEncoder::create(frame.input_caps(), 1_200, None)
        .expect("The hardware H.264 pipeline should be created");

    encoder
        .submit_frame(frame)
        .expect("The appsrc should accept one DMA-BUF video frame");

    assert_eq!(encoder.source.current_level_buffers(), 1);
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn rejects_system_memory_video_frames() {
    gstreamer::init().expect("GStreamer should initialize before constructing a frame");
    let buffer = gstreamer::Buffer::from_slice([0_u8; 16]);

    validate_dmabuf_buffer(&buffer)
        .expect_err("The hardware encoder input should reject system memory");
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn accepts_h264_rtp_packet_samples() {
    gstreamer::init().expect("GStreamer should initialize before constructing a packet");
    let buffer = gstreamer::Buffer::from_slice([0x80_u8, 0xe0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]);
    let sample = gstreamer::Sample::builder()
        .buffer(&buffer)
        .caps(&h264_rtp_caps())
        .build();

    let packet = GStreamerRtpPacket::try_from(sample)
        .expect("An H.264 RTP sample should satisfy the encoded packet boundary");

    assert_eq!(packet.payload_len(), 12);
    assert!(packet.is_frame_end());
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn rejects_non_h264_rtp_packet_samples() {
    gstreamer::init().expect("GStreamer should initialize before constructing a packet");
    let buffer = gstreamer::Buffer::from_slice([0_u8; 4]);
    let caps = gstreamer::Caps::builder("application/x-rtp")
        .field("media", "audio")
        .field("encoding-name", "OPUS")
        .field("clock-rate", 48_000_i32)
        .build();
    let sample = gstreamer::Sample::builder()
        .buffer(&buffer)
        .caps(&caps)
        .build();

    GStreamerRtpPacket::try_from(sample).expect_err("A non-H.264 RTP sample should be rejected");
}

#[test]
#[ignore = "run through scripts/test-gstreamer"]
fn vaapi_vpp_encodes_an_xrgb_dmabuf_with_latency_probe() {
    const FRAME_COUNT: u64 = 8;
    const FRAME_INTERVAL_NS: u64 = 16_666_667;
    const OUTPUT_TIMEOUT: gstreamer::ClockTime = gstreamer::ClockTime::from_seconds(5);

    gstreamer::init().expect("GStreamer should initialize");
    let render_node = std::env::var_os("RABBIT_GPU_RENDER_NODE")
        .expect("RABBIT_GPU_RENDER_NODE should name the render node under test");
    let context = GpuContext::new(&GpuDevice::from(PathBuf::from(render_node)))
        .expect("GPU context should initialize");
    let size = PixelSize {
        width: 1280,
        height: 720,
    };
    let modifier = va_vpp_input_modifier(DrmFourcc::Xrgb8888)
        .expect("VAAPI VPP should advertise an XRGB DMA-BUF modifier");
    let frame = context
        .allocate_dma_buf_with_modifier(
            size,
            DrmFourcc::Xrgb8888,
            modifier,
            gbm::BufferObjectFlags::RENDERING,
        )
        .expect("GBM should allocate a VAAPI-compatible XRGB DMA-BUF");
    let image = context
        .egl()
        .import_composition_target(&frame)
        .expect("EGL should import the XRGB DMA-BUF");
    let target = context
        .egl()
        .create_composition_target(&image)
        .expect("OpenGL should bind the XRGB DMA-BUF");
    context
        .egl()
        .clear_composition_target(&target)
        .expect("OpenGL should render the test frame");
    let fence = context
        .egl()
        .finish_composition()
        .expect("OpenGL should export a readiness fence");
    let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");
    runtime.block_on(async {
        let fence = PollFd::new(fence).expect("Readiness fence should register");
        fence
            .read_ready()
            .await
            .expect("Test frame should become ready");
    });

    let (pipeline, source, vpp, sink) = vaapi_vpp_test_pipeline(&frame);
    let probe = Arc::new(Mutex::new(VaVppProbe::default()));
    install_vpp_probe(&vpp, Arc::clone(&probe));
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("VAAPI VPP test pipeline should start");
    let mut warm_vpp = Duration::ZERO;
    let mut warm_encode = Duration::ZERO;
    let mut warm_total = Duration::ZERO;

    for frame_index in 0..FRAME_COUNT {
        let pts = gstreamer::ClockTime::from_nseconds(frame_index * FRAME_INTERVAL_NS);
        let mut buffer = xrgb_dmabuf_buffer(&frame);
        buffer
            .get_mut()
            .expect("New XRGB test buffer should be writable")
            .set_pts(pts);
        {
            let mut probe = probe.lock().expect("VPP probe lock should remain usable");
            probe.submitted = Some(Instant::now());
            probe.vpp_entered = None;
            probe.vpp_completed = None;
        }
        source
            .push_buffer(buffer)
            .expect("VAAPI VPP appsrc should accept the XRGB DMA-BUF");
        let sample = sink
            .try_pull_sample(OUTPUT_TIMEOUT)
            .expect("VAAPI VPP and H.264 encoder should produce output");
        let encoded = Instant::now();
        sample
            .buffer()
            .expect("Encoded sample should contain a buffer")
            .pts()
            .expect("Encoded output should carry a PTS");
        let probe = probe.lock().expect("VPP probe lock should remain usable");
        let submitted = probe
            .submitted
            .expect("VPP submission should be timestamped");
        let entered = probe
            .vpp_entered
            .expect("VPP input pad should observe the frame");
        let completed = probe
            .vpp_completed
            .expect("VPP output pad should observe the converted frame");
        let submit_to_vpp = elapsed(submitted, entered);
        let vpp = elapsed(entered, completed);
        let encode = elapsed(completed, encoded);
        let total = elapsed(submitted, encoded);
        println!(
            "VAAPI frame {frame_index}: cold={}, submit_to_vpp_ms={:.2}, vpp_ms={:.2}, encode_ms={:.2}, total_ms={:.2}",
            frame_index == 0,
            duration_ms(submit_to_vpp),
            duration_ms(vpp),
            duration_ms(encode),
            duration_ms(total),
        );
        if frame_index > 0 {
            warm_vpp += vpp;
            warm_encode += encode;
            warm_total += total;
        }
    }

    pipeline
        .set_state(gstreamer::State::Null)
        .expect("VAAPI VPP test pipeline should stop");
    let warm_frames = FRAME_COUNT - 1;
    println!(
        "VAAPI warm average: frames={warm_frames}, vpp_ms={:.2}, encode_ms={:.2}, total_ms={:.2}",
        average_ms(warm_vpp, warm_frames),
        average_ms(warm_encode, warm_frames),
        average_ms(warm_total, warm_frames),
    );
}

fn vaapi_vpp_test_pipeline(
    frame: &DmaBufFrame,
) -> (
    gstreamer::Pipeline,
    gstreamer_app::AppSrc,
    gstreamer::Element,
    gstreamer_app::AppSink,
) {
    let source = create_required_element("appsrc", "vaapi-test-input")
        .expect("GStreamer appsrc should be available")
        .downcast::<gstreamer_app::AppSrc>()
        .expect("appsrc factory should return AppSrc");
    source.set_caps(Some(&xrgb_dmabuf_caps(frame)));
    source.set_format(gstreamer::Format::Time);
    source.set_is_live(true);
    source.set_max_buffers(1);
    source.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);

    let vpp = create_required_element("vapostproc", "vaapi-test-vpp")
        .expect("GStreamer VAAPI VPP should be available");
    let output_caps = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:VAMemory"])
        .field("format", "NV12")
        .field(
            "width",
            i32::try_from(frame.size.width).expect("Test width should fit i32"),
        )
        .field(
            "height",
            i32::try_from(frame.size.height).expect("Test height should fit i32"),
        )
        .field("framerate", gstreamer::Fraction::new(120, 1))
        .build();
    let filter = create_required_element("capsfilter", "vaapi-test-output-caps")
        .expect("GStreamer capsfilter should be available");
    filter.set_property("caps", &output_caps);
    let encoder = create_required_element("vah264enc", "vaapi-test-encoder")
        .expect("GStreamer VAAPI H.264 encoder should be available");
    configure_low_latency_encoder(&encoder);
    let sink = create_required_element("appsink", "vaapi-test-output")
        .expect("GStreamer appsink should be available")
        .downcast::<gstreamer_app::AppSink>()
        .expect("appsink factory should return AppSink");
    sink.set_caps(Some(&gstreamer::Caps::builder("video/x-h264").build()));
    sink.set_sync(false);
    sink.set_async(false);

    let pipeline = gstreamer::Pipeline::new();
    let elements = [
        source.upcast_ref(),
        &vpp,
        &filter,
        &encoder,
        sink.upcast_ref(),
    ];
    pipeline
        .add_many(elements)
        .expect("VAAPI VPP test elements should join one pipeline");
    gstreamer::Element::link_many(elements).expect("VAAPI VPP test pipeline should negotiate");

    (pipeline, source, vpp, sink)
}

fn install_vpp_probe(vpp: &gstreamer::Element, probe: Arc<Mutex<VaVppProbe>>) {
    let input_probe = Arc::clone(&probe);
    vpp.static_pad("sink")
        .expect("VAAPI VPP should expose a sink pad")
        .add_probe(gstreamer::PadProbeType::BUFFER, move |_, _| {
            let mut probe = input_probe
                .lock()
                .expect("VPP input probe lock should remain usable");
            probe.vpp_entered.get_or_insert_with(Instant::now);
            gstreamer::PadProbeReturn::Ok
        });
    vpp.static_pad("src")
        .expect("VAAPI VPP should expose a source pad")
        .add_probe(gstreamer::PadProbeType::BUFFER, move |_, _| {
            let mut probe = probe
                .lock()
                .expect("VPP output probe lock should remain usable");
            probe.vpp_completed.get_or_insert_with(Instant::now);
            gstreamer::PadProbeReturn::Ok
        });
}

fn xrgb_dmabuf_caps(frame: &DmaBufFrame) -> gstreamer::Caps {
    let modifier: u64 = frame.planes[0].modifier.into();
    let drm_format = gstreamer_video::dma_drm_fourcc_to_string(frame.format as u32, modifier);

    gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", drm_format)
        .field(
            "width",
            i32::try_from(frame.size.width).expect("Test width should fit i32"),
        )
        .field(
            "height",
            i32::try_from(frame.size.height).expect("Test height should fit i32"),
        )
        .field("framerate", gstreamer::Fraction::new(120, 1))
        .build()
}

fn xrgb_dmabuf_buffer(frame: &DmaBufFrame) -> gstreamer::Buffer {
    assert_eq!(frame.objects.len(), 1);
    assert_eq!(frame.planes.len(), 1);
    let object = &frame.objects[0];
    let plane = frame.planes[0];
    let allocator = gstreamer_allocators::DmaBufAllocator::new();
    let memory = unsafe {
        allocator.alloc_dmabuf(
            object
                .fd
                .try_clone()
                .expect("Test DMA-BUF fd should duplicate"),
            object.size,
        )
    }
    .expect("GStreamer should wrap the XRGB DMA-BUF");
    let mut buffer = gstreamer::Buffer::new();
    let buffer_mut = buffer
        .get_mut()
        .expect("New XRGB test buffer should be writable");
    buffer_mut.append_memory(memory);
    gstreamer_video::VideoMeta::add_full(
        buffer_mut,
        gstreamer_video::VideoFrameFlags::empty(),
        gstreamer_video::VideoFormat::DmaDrm,
        frame.size.width,
        frame.size.height,
        &[usize::try_from(plane.offset).expect("Test offset should fit usize")],
        &[i32::try_from(plane.stride).expect("Test stride should fit i32")],
    )
    .expect("GStreamer should attach XRGB DMA-BUF VideoMeta");

    buffer
}

fn registered_nv12_dmabuf_input_caps() -> gstreamer::Caps {
    let mut caps = GStreamerVideoEncoder::find_hardware_h264_encoders()
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
        .expect("A hardware H.264 encoder should advertise NV12 DMA-BUF input caps");
    caps.make_mut()
        .structure_mut(0)
        .expect("Test encoder caps should contain one structure")
        .set("framerate", gstreamer::Fraction::new(120, 1));
    caps
}

fn init_host_video_tracing() {
    let targets = Targets::new()
        .with_default(LevelFilter::WARN)
        .with_target("rabbit::video_probe", LevelFilter::INFO)
        .with_target("rabbit::frame_pipeline", LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(targets)
        .with(tracing_subscriber::fmt::layer().with_test_writer())
        .try_init()
        .expect("Host video smoke test should install its tracing subscriber");
}

fn elapsed(start: Instant, end: Instant) -> Duration {
    end.checked_duration_since(start)
        .expect("Host video probe timestamps should be monotonic")
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn average_ms(total: Duration, count: u64) -> f64 {
    if count == 0 {
        return 0.0;
    }

    duration_ms(total) / count as f64
}

/// Probe native size and refresh from one KMS frame (DRM mode, not a hardcoded fps).
fn host_video_test_source_geometry(screen_name: &str) -> (PixelSize, FrameRate) {
    let (_reaper, reaper_handle) = WorkerReaper::new().expect("Test worker reaper should start");
    let ScreenCaptureSource { lease, receiver } = KmsCaptureLease::new(
        screen_name.to_owned(),
        false,
        Duration::from_secs(2),
        reaper_handle,
        Vec::new(),
        crate::infra::platform::screen_capture::KmsFrameQueuePolicy::Latest,
    )
    .expect("KMS capture source should start");
    let (device, frames, _fallback) = receiver.into_parts();
    device
        .recv()
        .expect("KMS capture worker should report its GPU")
        .expect("KMS capture GPU discovery should succeed");
    let frame = frames
        .recv()
        .expect("KMS capture worker should remain connected")
        .expect("KMS capture worker should publish one frame");
    let size = match frame.source {
        crate::infra::platform::screen_capture::KmsCapturedSource::PlaneSet {
            output_size, ..
        } => output_size,
        crate::infra::platform::screen_capture::KmsCapturedSource::Composed(buffer) => buffer.size,
    };
    let frame_rate = frame.frame_rate;
    drop(lease);

    (size, frame_rate)
}

fn host_video_test_target_size(source_size: PixelSize) -> PixelSize {
    let Ok(resolution) = std::env::var("RABBIT_HOST_VIDEO_TEST_RESOLUTION") else {
        return source_size;
    };
    let (width, height) = resolution
        .split_once('x')
        .expect("RABBIT_HOST_VIDEO_TEST_RESOLUTION should use WIDTHxHEIGHT");
    let width = width
        .parse::<u32>()
        .expect("Host video target width should be a positive integer");
    let height = height
        .parse::<u32>()
        .expect("Host video target height should be a positive integer");
    assert!(width > 0, "Host video target width should be positive");
    assert!(height > 0, "Host video target height should be positive");

    PixelSize { width, height }
}

/// Optional override: `RABBIT_HOST_VIDEO_TEST_FRAME_RATE=144` or `144/1`.
/// Default: use the KMS-reported source refresh (e.g. 144 on HDMI-A-1).
fn host_video_test_target_frame_rate(source: FrameRate) -> FrameRate {
    let Ok(raw) = std::env::var("RABBIT_HOST_VIDEO_TEST_FRAME_RATE") else {
        return source;
    };
    let (numerator, denominator) = if let Some((num, den)) = raw.split_once('/') {
        (
            num.parse::<u32>()
                .expect("RABBIT_HOST_VIDEO_TEST_FRAME_RATE numerator should be a positive integer"),
            den.parse::<u32>().expect(
                "RABBIT_HOST_VIDEO_TEST_FRAME_RATE denominator should be a positive integer",
            ),
        )
    } else {
        (
            raw.parse::<u32>()
                .expect("RABBIT_HOST_VIDEO_TEST_FRAME_RATE should be FPS or NUM/DEN"),
            1,
        )
    };
    FrameRate::new(numerator, denominator)
        .expect("RABBIT_HOST_VIDEO_TEST_FRAME_RATE should be a positive frame rate")
}

/// Expect at least ~80% of the target refresh over the run (allows short warmup).
fn host_video_test_min_encoded_frames(frame_rate: FrameRate, run_seconds: u64) -> u64 {
    let frames_per_second = f64::from(frame_rate.numerator()) / f64::from(frame_rate.denominator());
    let expected = frames_per_second * run_seconds as f64 * 0.8;
    expected.floor().max(1.0) as u64
}

fn host_video_test_screen(name: String, resolution: PixelSize, frame_rate: FrameRate) -> Screen {
    Screen {
        id: ScreenId(0),
        name,
        resolution,
        frame_rate,
        layout: ScreenLayout {
            rect: ScreenRect {
                x: 0,
                y: 0,
                width: resolution.width,
                height: resolution.height,
            },
            scale: 1.0,
            transform: ScreenTransform::Normal,
        },
    }
}

fn h264_caps() -> gstreamer::Caps {
    gstreamer::Caps::builder("video/x-h264").build()
}

fn dmabuf_video_frame() -> GStreamerVideoFrame {
    GStreamerVideoFrame::try_from(dmabuf_pipeline_frame(None))
        .expect("The test buffer should satisfy the encoder input boundary")
}

fn dmabuf_pipeline_frame(lease: Option<DmaBufLease>) -> Rc<GbmFramePipelineFrame> {
    const WIDTH: u32 = 16;
    const HEIGHT: u32 = 16;
    const Y_SIZE: usize = WIDTH as usize * HEIGHT as usize;
    const BUFFER_SIZE: usize = Y_SIZE + Y_SIZE / 2;

    let file = File::open("/dev/zero").expect("The test DMA-BUF fd should open");
    Rc::new(GbmFramePipelineFrame {
        buffer: DmaBufFrame {
            size: PixelSize {
                width: WIDTH,
                height: HEIGHT,
            },
            format: DrmFourcc::Nv12,
            objects: vec![DmaBufObject {
                fd: OwnedFd::from(file),
                size: BUFFER_SIZE,
            }],
            planes: vec![
                DmaBufPlane {
                    object_index: 0,
                    offset: 0,
                    stride: WIDTH,
                    modifier: DrmModifier::Invalid,
                },
                DmaBufPlane {
                    object_index: 0,
                    offset: u32::try_from(Y_SIZE).expect("The test Y plane size should fit u32"),
                    stride: WIDTH,
                    modifier: DrmModifier::Invalid,
                },
            ],
            readiness_fence: None,
            lease,
            va_backing: None,
        },
        source_frame_rate: FrameRate::new(60, 1).expect("Test frame rate should be valid"),
        frame_rate: FrameRate::new(60, 1).expect("Test frame rate should be valid"),
        probe: None,
    })
}
