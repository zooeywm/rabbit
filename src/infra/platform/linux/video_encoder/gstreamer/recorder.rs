//! Local screen recording: DMA-BUF frames → hardware H.264 → MP4 file.
//!
//! Shares encoder selection and VPP wiring with the RTP streaming encoder, but
//! terminates in `mp4mux` + `filesink` instead of `rtph264pay` + `appsink`.

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

use super::{
    encoder::GStreamerVideoEncoder,
    frame::{DmaBufInputSignature, GStreamerVideoFrame},
    pipeline_util::{
        H264_BITRATE_KBPS, H264_CPB_SIZE_KBITS, H264_KEY_INT_MAX, configure_low_latency_encoder,
        create_pipeline_stage_queue, create_required_element, terminal_message_result,
        terminal_messages, va_vpp_output_caps,
    },
};
use crate::infra::platform::frame_pipeline::GbmFramePipelineFrame;
use crate::infra::unsync_queue::UnsyncQueue;
use crate::kernel::geometry::FrameRate;

/// Records a processed frame subscription to an H.264 MP4 file until cancelled
/// or the frame stream ends.
pub(crate) async fn record_frames_to_mp4<Frames>(
    mut frames: Frames,
    frame_rate: FrameRate,
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
    let first_frame = first_frame.with_context(|| "Failed to receive first recording frame")?;
    let source_frame_rate = first_frame.source_frame_rate;
    let first_frame = GStreamerVideoFrame::from_pipeline_frame(first_frame, frame_rate, None)?;
    let mut recorder = GStreamerScreenRecorder::new(first_frame, source_frame_rate, output_path)?;
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
    output_path: PathBuf,
}

impl GStreamerScreenRecorder {
    fn new(
        first_frame: GStreamerVideoFrame,
        source_frame_rate: FrameRate,
        output_path: PathBuf,
    ) -> eros::Result<Self> {
        let mut recorder = Self::create(first_frame.input_caps(), &output_path)?;
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
            "Local screen recording started"
        );
        Ok(recorder)
    }

    fn create(input_caps: &gstreamer::CapsRef, output_path: &Path) -> eros::Result<Self> {
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
        configure_low_latency_encoder(&element);
        // Prefer slightly longer GOPs for file size when recording (still low-latency friendly).
        if element.find_property("key-int-max").is_some() {
            element.set_property("key-int-max", H264_KEY_INT_MAX.min(120));
        }
        tracing::info!(
            target: "rabbit::video_encoder",
            event = "video_recorder_encoder_selected",
            factory = %factory_name,
            bitrate_kbps = H264_BITRATE_KBPS,
            cpb_size_kbits = H264_CPB_SIZE_KBITS,
            "Selected hardware H.264 encoder for local recording"
        );

        let source = create_required_element("appsrc", "video-input")?;
        let Ok(source) = source.downcast::<gstreamer_app::AppSrc>() else {
            eros::bail!("GStreamer appsrc factory returned an unexpected element type");
        };
        source.set_caps(Some(&input_caps));
        source.set_format(gstreamer::Format::Time);
        source.set_is_live(true);
        source.set_do_timestamp(true);
        source.set_max_buffers(2);
        source.set_leaky_type(gstreamer_app::AppLeakyType::Downstream);

        let vpp = if let Some(vpp_caps) = &vpp_caps {
            let vpp = create_required_element("vapostproc", "video-postprocessor")?;
            let filter = create_required_element("capsfilter", "video-postprocessor-output")?;
            filter.set_property("caps", vpp_caps);
            let queue = create_pipeline_stage_queue("processed-frame-queue", 1)?;
            queue.set_property_from_str("leaky", "downstream");
            Some((vpp, filter, queue))
        } else {
            None
        };

        let parser = create_required_element("h264parse", "h264-parser")?;
        parser.set_property("config-interval", -1_i32);
        let encoded_output_queue = create_pipeline_stage_queue("encoded-output-queue", 2)?;
        let mux = create_required_element("mp4mux", "mp4-mux")?;
        // Live-friendly fragmented-ish settings; still produces a normal mp4 after EOS.
        if mux.find_property("fragment-duration").is_some() {
            mux.set_property("fragment-duration", 0_u32);
        }
        if mux.find_property("streamable").is_some() {
            mux.set_property("streamable", false);
        }
        let sink = create_required_element("filesink", "file-output")?;
        sink.set_property(
            "location",
            output_path
                .to_str()
                .with_context(|| format!("Recording path is not valid UTF-8: {}", output_path.display()))?,
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

        let terminal_messages = terminal_messages(&pipeline)?;
        Ok(Self {
            pipeline,
            source,
            terminal_messages,
            input_caps,
            input_signature,
            source_frame_rate: input_signature.frame_rate,
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

    fn prepare_frame(
        &self,
        frame: Rc<GbmFramePipelineFrame>,
    ) -> eros::Result<GStreamerVideoFrame> {
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

    fn submit_frame(&mut self, frame: GStreamerVideoFrame) -> eros::Result<()> {
        if frame.input_signature != self.input_signature {
            eros::bail!(
                "Recorder input changed from {:?} to {:?}",
                self.input_signature,
                frame.input_signature
            );
        }
        self.source
            .push_buffer(frame.buffer)
            .with_context(|| "Failed to submit DMA-BUF frame to screen recorder")?;
        Ok(())
    }

    fn finalize(&mut self) -> eros::Result<()> {
        self.source
            .end_of_stream()
            .with_context(|| "Failed to send EOS to screen recorder")?;

        // Wait briefly for mp4mux to finalize the file after EOS.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match self.terminal_messages.recv_timeout(Duration::from_millis(100)) {
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
            "Local screen recording finished"
        );
        Ok(())
    }
}
