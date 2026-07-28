//! Local screen recording: DMA-BUF frames → hardware H.264 → MP4 file.
//!
//! Unlike the RTP streaming encoder, this path is **lossless with respect to
//! delivered frames**: no leaky queues, no live appsrc drops, and monotonic PTS
//! at the declared frame rate. Upstream capture must use [`FrameDelivery::Reliable`].

use std::{
    future::{Future, poll_fn},
    path::{Path, PathBuf},
    pin::Pin,
    rc::Rc,
    time::Duration,
};

use eros::Context as _;
use futures_util::future::{Either, select};
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{
    Cast as _, ElementExt as _, GObjectExtManualGst as _, GstBinExtManual as _, GstObjectExt as _,
};
use gstreamer::{ClockTime, Format};

use super::{
    encoder::GStreamerVideoEncoder,
    frame::{DmaBufInputSignature, GStreamerVideoFrame},
    pipeline_util::{
        create_pipeline_stage_queue, create_required_element, terminal_message_result,
        terminal_messages, va_vpp_output_caps,
    },
};
use crate::infra::platform::frame_pipeline::GbmFramePipelineFrame;
use crate::infra::unsync_queue::UnsyncQueue;
use crate::kernel::geometry::FrameRate;

/// Recording-oriented VAAPI settings: quality over ultra-low latency.
const RECORD_BITRATE_KBPS: u32 = 80_000;
const RECORD_CPB_SIZE_KBITS: u32 = 40_000;
/// ~2s GOP at 144 Hz; better compression than per-frame IDR streaming defaults.
const RECORD_KEY_INT_MAX: u32 = 288;
/// Keep appsrc depth within the VA NV12 output pool so leases can recycle.
const RECORD_APPSRC_MAX_BUFFERS: u64 = 32;
const RECORD_STAGE_QUEUE_BUFFERS: u32 = 16;

/// Records a processed frame subscription to an H.264 MP4 file until cancelled
/// or the frame stream ends.
pub(crate) async fn record_frames_to_mp4<Frames>(
    mut frames: Frames,
    _subscribe_frame_rate: FrameRate,
    output_path: impl AsRef<Path>,
    cancellation: UnsyncQueue<()>,
) -> eros::Result<()>
where
    Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
{
    let output_path = output_path.as_ref().to_path_buf();
    let Some(first_frame) = poll_fn(|context| Pin::new(&mut frames).poll_next(context)).await
    else {
        eros::bail!("Recording ended before the first frame arrived");
    };
    let first_frame =
        first_frame.with_context(|| "Failed to receive first recording frame from the pipeline")?;
    let source_frame_rate = first_frame.source_frame_rate;
    // Tag caps with the pipeline output rate (min of source/target), not wall clock.
    let output_rate = first_frame.frame_rate;
    let first_frame = GStreamerVideoFrame::from_pipeline_frame(first_frame, output_rate, None)?;
    let mut recorder =
        GStreamerScreenRecorder::new(first_frame, source_frame_rate, output_rate, output_path)?;
    let result = recorder.drive(&mut frames, &cancellation).await;
    let stop = recorder
        .finalize()
        .with_context(|| "Failed to finalize MP4 recording");

    match (result, stop) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(stop_error)) => eros::bail!(
            "Recording failed: {}; additionally failed to finalize: {}",
            error,
            stop_error
        ),
    }
}

struct GStreamerScreenRecorder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    terminal_messages: flume::Receiver<gstreamer::Message>,
    input_caps: gstreamer::Caps,
    input_signature: DmaBufInputSignature,
    source_frame_rate: FrameRate,
    frame_duration: ClockTime,
    next_frame_index: u64,
    output_path: PathBuf,
}

impl GStreamerScreenRecorder {
    fn new(
        first_frame: GStreamerVideoFrame,
        source_frame_rate: FrameRate,
        output_rate: FrameRate,
        output_path: PathBuf,
    ) -> eros::Result<Self> {
        let mut recorder = Self::create(first_frame.input_caps(), output_rate, &output_path)?;
        recorder.source_frame_rate = source_frame_rate;
        if let Some(context) = &first_frame.va_context {
            recorder.pipeline.set_context(context);
        }
        recorder.submit_frame(first_frame)?;
        recorder
            .pipeline
            .set_state(gstreamer::State::Playing)
            .with_context(|| "Failed to start GStreamer screen recorder")?;
        tracing::info!(
            event = "local_recording_started",
            path = %recorder.output_path.display(),
            width = recorder.input_signature.size.width,
            height = recorder.input_signature.size.height,
            fps_num = output_rate.numerator(),
            fps_den = output_rate.denominator(),
            "Local screen recording started (lossless delivery, fixed PTS)"
        );
        Ok(recorder)
    }

    fn create(
        input_caps: &gstreamer::CapsRef,
        output_rate: FrameRate,
        output_path: &Path,
    ) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let input_signature = DmaBufInputSignature::try_from(input_caps)?;
        let vpp_caps = if GStreamerVideoEncoder::is_xrgb_dmabuf_input_caps(input_caps) {
            Some(va_vpp_output_caps(input_caps)?)
        } else {
            if !GStreamerVideoEncoder::is_nv12_dmabuf_input_caps(input_caps) {
                eros::bail!(
                    "Local recording requires NV12 or XRGB8888 DMA-BUF input caps, got {}",
                    input_caps
                );
            }
            None
        };
        let encoder_caps = vpp_caps.as_deref().unwrap_or(input_caps);
        let factory = GStreamerVideoEncoder::select_hardware_h264_encoder(encoder_caps)?;
        let input_caps = input_caps.to_owned();
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
        configure_recording_encoder(&element, output_rate);
        tracing::info!(
            target: "rabbit::video_encoder",
            event = "video_recorder_encoder_selected",
            factory = %factory_name,
            bitrate_kbps = RECORD_BITRATE_KBPS,
            cpb_size_kbits = RECORD_CPB_SIZE_KBITS,
            key_int_max = RECORD_KEY_INT_MAX,
            "Selected hardware H.264 encoder for lossless local recording"
        );

        let source = create_required_element("appsrc", "video-input")?;
        let Ok(source) = source.downcast::<gstreamer_app::AppSrc>() else {
            eros::bail!("GStreamer appsrc factory returned an unexpected element type");
        };
        source.set_caps(Some(&input_caps));
        source.set_format(Format::Time);
        // Non-live + no leaky: block / queue rather than drop when encode lags.
        source.set_is_live(false);
        source.set_do_timestamp(false);
        source.set_block(true);
        source.set_max_bytes(0);
        source.set_max_buffers(RECORD_APPSRC_MAX_BUFFERS);
        source.set_leaky_type(gstreamer_app::AppLeakyType::None);

        let vpp = if let Some(vpp_caps) = &vpp_caps {
            let vpp = create_required_element("vapostproc", "video-postprocessor")?;
            let filter = create_required_element("capsfilter", "video-postprocessor-output")?;
            filter.set_property("caps", vpp_caps);
            let queue =
                create_pipeline_stage_queue("processed-frame-queue", RECORD_STAGE_QUEUE_BUFFERS)?;
            // Default leaky=no: never discard.
            Some((vpp, filter, queue))
        } else {
            None
        };

        let parser = create_required_element("h264parse", "h264-parser")?;
        parser.set_property("config-interval", -1_i32);
        let encoded_output_queue =
            create_pipeline_stage_queue("encoded-output-queue", RECORD_STAGE_QUEUE_BUFFERS)?;
        let mux = create_required_element("mp4mux", "mp4-mux")?;
        if mux.find_property("fragment-duration").is_some() {
            mux.set_property("fragment-duration", 0_u32);
        }
        if mux.find_property("streamable").is_some() {
            mux.set_property("streamable", false);
        }
        let sink = create_required_element("filesink", "file-output")?;
        sink.set_property(
            "location",
            output_path.to_str().with_context(|| {
                format!(
                    "Recording path is not valid UTF-8: {}",
                    output_path.display()
                )
            })?,
        );
        sink.set_property("sync", false);
        sink.set_property("async", false);

        let pipeline = gstreamer::Pipeline::new();
        let base_elements = [
            source.upcast_ref(),
            &element,
            &encoded_output_queue,
            &parser,
            &mux,
            &sink,
        ];
        pipeline
            .add_many(base_elements)
            .with_context(|| "Failed to add recording elements to GStreamer pipeline")?;
        if let Some((vpp, filter, queue)) = &vpp {
            pipeline
                .add_many([vpp, filter, queue])
                .with_context(|| "Failed to add VAAPI VPP elements to recording pipeline")?;
            gstreamer::Element::link_many([
                source.upcast_ref(),
                vpp,
                filter,
                queue,
                &element,
                &encoded_output_queue,
                &parser,
                &mux,
                &sink,
            ])
            .with_context(|| "Failed to link VAAPI VPP H.264 recording pipeline")?;
        } else {
            gstreamer::Element::link_many(base_elements)
                .with_context(|| "Failed to link GStreamer H.264 recording pipeline")?;
        }

        let frame_duration = frame_duration_ns(output_rate)?;
        let terminal_messages = terminal_messages(&pipeline)?;
        Ok(Self {
            pipeline,
            source,
            terminal_messages,
            input_caps,
            input_signature,
            source_frame_rate: input_signature.frame_rate,
            frame_duration,
            next_frame_index: 0,
            output_path: output_path.to_path_buf(),
        })
    }

    async fn drive<Frames>(
        &mut self,
        frames: &mut Frames,
        cancellation: &UnsyncQueue<()>,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
    {
        loop {
            enum Event {
                Frame(Option<eros::Result<Rc<GbmFramePipelineFrame>>>),
                Cancel,
                Terminal(Result<gstreamer::Message, flume::RecvError>),
            }

            let event = {
                let frame = poll_fn(|context| Pin::new(&mut *frames).poll_next(context));
                let mut cancel = cancellation.pop();
                let cancel = poll_fn(move |context| {
                    Pin::new(&mut cancel).poll(context).map(|_| Event::Cancel)
                });
                let terminal = self.terminal_messages.recv_async();
                futures_util::pin_mut!(frame, cancel, terminal);

                match select(frame, select(cancel, terminal)).await {
                    Either::Left((sample, _)) => Event::Frame(sample),
                    Either::Right((Either::Left((cancel_event, _)), _)) => cancel_event,
                    Either::Right((Either::Right((message, _)), _)) => Event::Terminal(message),
                }
            };

            match event {
                Event::Cancel => {
                    tracing::info!(
                        event = "local_recording_stop_requested",
                        path = %self.output_path.display(),
                        frames = self.next_frame_index,
                        "Local recording stop requested"
                    );
                    return Ok(());
                }
                Event::Frame(None) => return Ok(()),
                Event::Frame(Some(frame)) => {
                    let frame = frame.with_context(|| "Recording frame pipeline failed")?;
                    let prepared = self.prepare_frame(frame)?;
                    self.submit_frame(prepared)?;
                }
                Event::Terminal(Ok(message)) => {
                    terminal_message_result(&message)?;
                    return Ok(());
                }
                Event::Terminal(Err(_)) => {
                    eros::bail!("GStreamer recording terminal message channel disconnected")
                }
            }
        }
    }

    fn prepare_frame(&self, frame: Rc<GbmFramePipelineFrame>) -> eros::Result<GStreamerVideoFrame> {
        if frame.source_frame_rate != self.source_frame_rate {
            eros::bail!(
                "Frame-pipeline source frame rate changed from {:?} to {:?}",
                self.source_frame_rate,
                frame.source_frame_rate
            );
        }
        GStreamerVideoFrame::from_pipeline_frame(
            frame,
            self.input_signature.frame_rate,
            Some((self.input_caps.as_ref(), self.input_signature)),
        )
    }

    fn submit_frame(&mut self, mut frame: GStreamerVideoFrame) -> eros::Result<()> {
        if frame.input_signature != self.input_signature {
            eros::bail!(
                "Recorder input changed from {:?} to {:?}",
                self.input_signature,
                frame.input_signature
            );
        }

        let pts = self
            .frame_duration
            .nseconds()
            .checked_mul(self.next_frame_index)
            .with_context(|| "Recording PTS overflowed")?;
        {
            let buffer = frame.buffer.make_mut();
            buffer.set_pts(Some(ClockTime::from_nseconds(pts)));
            buffer.set_dts(Some(ClockTime::from_nseconds(pts)));
            buffer.set_duration(Some(self.frame_duration));
        }

        // block=true on appsrc: wait for space rather than dropping.
        self.source
            .push_buffer(frame.buffer)
            .with_context(|| "Failed to submit DMA-BUF frame to screen recorder")?;
        self.next_frame_index = self
            .next_frame_index
            .checked_add(1)
            .with_context(|| "Recording frame index overflowed")?;
        Ok(())
    }

    fn finalize(&mut self) -> eros::Result<()> {
        self.source
            .end_of_stream()
            .with_context(|| "Failed to send EOS to screen recorder")?;

        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        while std::time::Instant::now() < deadline {
            match self
                .terminal_messages
                .recv_timeout(Duration::from_millis(100))
            {
                Ok(message) => {
                    terminal_message_result(&message)?;
                    break;
                }
                Err(flume::RecvTimeoutError::Timeout) => continue,
                Err(flume::RecvTimeoutError::Disconnected) => break,
            }
        }

        self.pipeline
            .set_state(gstreamer::State::Null)
            .with_context(|| "Failed to stop GStreamer screen recorder")?;
        tracing::info!(
            event = "local_recording_finished",
            path = %self.output_path.display(),
            frames = self.next_frame_index,
            "Local screen recording finished"
        );
        Ok(())
    }
}

fn frame_duration_ns(frame_rate: FrameRate) -> eros::Result<ClockTime> {
    let numer = u128::from(frame_rate.numerator());
    let denom = u128::from(frame_rate.denominator());
    // duration = 1e9 * denom / numer nanoseconds
    let nanos = 1_000_000_000u128
        .checked_mul(denom)
        .and_then(|value| value.checked_div(numer))
        .with_context(|| "Invalid frame rate for recording duration")?;
    let nanos = u64::try_from(nanos).with_context(|| "Frame duration exceeds u64 nanoseconds")?;
    if nanos == 0 {
        eros::bail!("Frame duration collapsed to zero for {frame_rate:?}");
    }
    Ok(ClockTime::from_nseconds(nanos))
}

fn configure_recording_encoder(encoder: &gstreamer::Element, frame_rate: FrameRate) {
    let is_vaapi = encoder
        .factory()
        .is_some_and(|factory| factory.name().starts_with("va"));
    if !is_vaapi {
        return;
    }

    encoder.set_property("b-frames", 0_u32);
    encoder.set_property("ref-frames", 2_u32);
    // Prefer quality over pure speed (streaming uses 7).
    encoder.set_property("target-usage", 4_u32);
    if encoder.find_property("mbbrc").is_some() {
        encoder.set_property_from_str("mbbrc", "disabled");
    }
    encoder.set_property_from_str("rate-control", "vbr");
    encoder.set_property("bitrate", RECORD_BITRATE_KBPS);
    if encoder.find_property("target-percentage").is_some() {
        encoder.set_property("target-percentage", 95_u32);
    }
    encoder.set_property("cpb-size", RECORD_CPB_SIZE_KBITS);
    let key_int = RECORD_KEY_INT_MAX.min(
        (u64::from(frame_rate.numerator()).saturating_mul(2)
            / u64::from(frame_rate.denominator()).max(1))
        .clamp(30, RECORD_KEY_INT_MAX as u64) as u32,
    );
    encoder.set_property("key-int-max", key_int);
}
