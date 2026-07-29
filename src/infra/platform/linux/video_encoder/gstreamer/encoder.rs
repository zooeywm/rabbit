//! Long-lived GStreamer H.264 encode pipeline.

use std::{
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
    time::Duration,
};

use eros::Context as _;
use futures_core::Stream as _;
use futures_util::future::{Either, select};
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{
    Cast as _, ElementExt as _, ElementExtManual as _, GObjectExtManualGst as _, GstBinExt as _,
    GstBinExtManual as _, GstObjectExt as _, PadExt as _,
};

use super::{
    frame::{DmaBufInputSignature, GStreamerVideoFrame},
    pipeline_util::{
        H264_KEY_INT_MAX, configure_low_latency_encoder, create_pipeline_stage_queue,
        create_required_element, h264_rtp_caps, is_hardware_video_encoder, rtp_mtu,
        terminal_message_result, terminal_messages, va_vpp_output_caps,
    },
    probe::GStreamerVideoProbe,
    rtp::GStreamerRtpPacket,
};
use crate::infra::platform::{frame_pipeline::GbmFramePipelineFrame, video_probe::VideoFrameProbe};
use crate::kernel::{
    geometry::FrameRate,
    video_encoder::{VideoBitrate, VideoCodec, VideoEncoderCommand, VideoEncoderParameters},
};

#[derive(Debug)]

pub(crate) struct GStreamerVideoEncoder {
    pub(super) pipeline: gstreamer::Pipeline,
    pub(super) source: gstreamer_app::AppSrc,
    pub(super) output: gstreamer_app::app_sink::AppSinkStream,
    pub(super) terminal_messages: flume::Receiver<gstreamer::Message>,
    pub(super) input_caps: gstreamer::Caps,
    pub(super) input_signature: DmaBufInputSignature,
    pub(super) source_frame_rate: FrameRate,
    pub(super) latest_input: Option<gstreamer::Buffer>,
    pub(super) probe: Option<GStreamerVideoProbe>,
}

impl GStreamerVideoEncoder {
    pub(super) async fn run_inner<Frames, Commands, SendPacket, SendFuture>(
        mut frames: Frames,
        mut commands: Commands,
        parameters: VideoEncoderParameters,
        max_rtp_packet_size: usize,
        mut send_packet: SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
        Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
        SendPacket: FnMut(Vec<GStreamerRtpPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        if parameters.codec != VideoCodec::H264 {
            eros::bail!(
                "GStreamer H.264 encoder cannot encode {:?}",
                parameters.codec
            );
        }
        let frame_rate = parameters.frame_rate;
        let Some(first_frame) = poll_fn(|context| Pin::new(&mut frames).poll_next(context)).await
        else {
            return Ok(());
        };
        let first_frame =
            first_frame.with_context(|| "Failed to receive first frame-pipeline output")?;
        let source_frame_rate = first_frame.source_frame_rate;
        let first_frame = GStreamerVideoFrame::from_pipeline_frame(first_frame, frame_rate, None)?;
        let mut encoder = Self::new(
            first_frame,
            source_frame_rate,
            parameters.bitrate,
            max_rtp_packet_size,
        )?;
        let result = encoder
            .drive(&mut frames, &mut commands, &mut send_packet)
            .await;
        let stop = encoder
            .stop()
            .with_context(|| "Failed to stop GStreamer video encoder");

        match (result, stop) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(stop_error)) => eros::bail!(
                "Video encoding failed: {}; additionally failed to stop encoder: {}",
                error,
                stop_error
            ),
        }
    }

    pub(crate) fn new(
        first_frame: GStreamerVideoFrame,
        source_frame_rate: FrameRate,
        bitrate: VideoBitrate,
        max_rtp_packet_size: usize,
    ) -> eros::Result<Self> {
        let probe_interval = first_frame
            .probe
            .as_ref()
            .map(VideoFrameProbe::report_interval);
        let mut encoder = Self::create(
            first_frame.input_caps(),
            bitrate,
            max_rtp_packet_size,
            probe_interval,
        )?;
        encoder.source_frame_rate = source_frame_rate;
        if let Some(context) = &first_frame.va_context {
            encoder.pipeline.set_context(context);
        }
        encoder.submit_frame(first_frame)?;
        encoder.start()?;

        Ok(encoder)
    }

    pub(super) fn create(
        input_caps: &gstreamer::CapsRef,
        bitrate: VideoBitrate,
        max_rtp_packet_size: usize,
        probe_interval: Option<Duration>,
    ) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let rtp_mtu = rtp_mtu(max_rtp_packet_size)?;
        let input_signature = DmaBufInputSignature::try_from(input_caps)?;
        let vpp_caps = if Self::is_xrgb_dmabuf_input_caps(input_caps) {
            Some(va_vpp_output_caps(input_caps)?)
        } else {
            if !Self::is_nv12_dmabuf_input_caps(input_caps) {
                eros::bail!(
                    "First-version H.264 encoding requires NV12 or XRGB8888 DMA-BUF input caps, got {}",
                    input_caps
                );
            }
            None
        };
        let encoder_caps = vpp_caps.as_deref().unwrap_or(input_caps);
        let factory = Self::select_hardware_h264_encoder(encoder_caps)?;
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
        let (bitrate_kbps, cpb_size_kbits) = configure_low_latency_encoder(&element, bitrate)?;
        let (h264_profile, h264_profile_caps) = select_h264_profile(&element)?;
        let profile_filter = create_required_element("capsfilter", "h264-profile")?;
        profile_filter.set_property("caps", &h264_profile_caps);
        let source = create_required_element("appsrc", "video-input")?;
        let Ok(source) = source.downcast::<gstreamer_app::AppSrc>() else {
            eros::bail!("GStreamer appsrc factory returned an unexpected element type");
        };
        source.set_caps(Some(&input_caps));
        source.set_format(gstreamer::Format::Time);
        source.set_is_live(true);
        source.set_do_timestamp(true);
        source.set_max_buffers(1);
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
        let encoded_output_queue = create_pipeline_stage_queue("encoded-output-queue", 2)?;
        let payloader = create_required_element("rtph264pay", "rtp-payloader")?;
        payloader.set_property("mtu", rtp_mtu);
        payloader.set_property("config-interval", -1_i32);
        let sink = create_required_element("appsink", "rtp-output")?;
        let Ok(sink) = sink.downcast::<gstreamer_app::AppSink>() else {
            eros::bail!("GStreamer appsink factory returned an unexpected element type");
        };
        sink.set_caps(Some(&h264_rtp_caps()));
        sink.set_async(false);
        sink.set_sync(false);
        let output = sink.stream();

        let pipeline = gstreamer::Pipeline::new();
        let base_elements = [
            source.upcast_ref(),
            &element,
            &profile_filter,
            &encoded_output_queue,
            &parser,
            &payloader,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(base_elements)
            .with_context(|| "Failed to add H.264 encoding elements to GStreamer pipeline")?;
        if let Some((vpp, filter, queue)) = &vpp {
            pipeline
                .add_many([vpp, filter, queue])
                .with_context(|| "Failed to add VAAPI VPP elements to GStreamer pipeline")?;
            gstreamer::Element::link_many([
                source.upcast_ref(),
                vpp,
                filter,
                queue,
                &element,
                &profile_filter,
                &encoded_output_queue,
                &parser,
                &payloader,
                sink.upcast_ref(),
            ])
            .with_context(|| "Failed to link VAAPI VPP H.264 RTP encoding pipeline")?;
        } else {
            gstreamer::Element::link_many(base_elements)
                .with_context(|| "Failed to link GStreamer H.264 RTP encoding pipeline")?;
        }
        let probe = if let Some(report_interval) = probe_interval {
            Some(GStreamerVideoProbe::new(
                &source,
                vpp.as_ref().map(|(vpp, _, _)| vpp),
                &element,
                report_interval,
            )?)
        } else {
            None
        };
        let terminal_messages = terminal_messages(&pipeline)?;
        tracing::info!(
            target: "rabbit::video_encoder",
            event = "linux_video_encoder_selected",
            framework = "gstreamer",
            codec = "h264",
            factory = %factory_name,
            input_memory = "dma-buf",
            video_processor = if vpp.is_some() { "vapostproc" } else { "none" },
            packetizer = "rtph264pay",
            transport_payload = "rtp",
            frame_rate_numerator = input_signature.frame_rate.numerator(),
            frame_rate_denominator = input_signature.frame_rate.denominator(),
            bitrate_kbps,
            cpb_size_kbits,
            h264_profile,
            key_int_max = H264_KEY_INT_MAX,
            "Selected Linux video encoder pipeline"
        );

        Ok(Self {
            pipeline,
            source,
            output,
            terminal_messages,
            input_caps,
            input_signature,
            source_frame_rate: input_signature.frame_rate,
            latest_input: None,
            probe,
        })
    }

    pub(crate) fn start(&self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Playing)
            .with_context(|| "Failed to start GStreamer H.264 encoding pipeline")?;

        Ok(())
    }

    pub(crate) fn stop(&mut self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Null)
            .with_context(|| "Failed to stop GStreamer H.264 encoding pipeline")?;
        if let Some(probe) = &mut self.probe {
            probe.finish();
        }

        Ok(())
    }

    pub(crate) fn finish(&self) -> eros::Result<()> {
        self.source
            .end_of_stream()
            .with_context(|| "Failed to finish GStreamer H.264 encoding input")?;

        Ok(())
    }

    pub(crate) fn submit_frame(&mut self, mut frame: GStreamerVideoFrame) -> eros::Result<()> {
        if frame.input_signature != self.input_signature {
            eros::bail!(
                "GStreamer encoder input changed from {:?} to {:?}",
                self.input_signature,
                frame.input_signature
            );
        }

        if let Some(frame_probe) = frame.probe.take()
            && let Some(probe) = &mut self.probe
        {
            probe.submit_frame(frame_probe);
        }

        self.latest_input = Some(frame.buffer.clone());
        self.source
            .push_buffer(frame.buffer)
            .with_context(|| "Failed to submit DMA-BUF frame to GStreamer H.264 encoder")?;

        Ok(())
    }

    pub(crate) fn submit_latest_frame(&self) -> eros::Result<()> {
        let mut buffer = self
            .latest_input
            .as_ref()
            .cloned()
            .with_context(|| "GStreamer encoder has no retained frame to encode")?;
        {
            let buffer = buffer.make_mut();
            buffer.set_pts(gstreamer::ClockTime::NONE);
            buffer.set_dts(gstreamer::ClockTime::NONE);
            buffer.set_duration(gstreamer::ClockTime::NONE);
        }
        self.source
            .push_buffer(buffer)
            .with_context(|| "Failed to resubmit retained DMA-BUF frame to GStreamer")?;
        Ok(())
    }

    pub(super) fn request_key_frame(&self) -> eros::Result<()> {
        let event = gstreamer_video::DownstreamForceKeyUnitEvent::builder()
            .all_headers(true)
            .build();
        if !self.source.send_event(event) {
            tracing::warn!(
                target: "rabbit::video_encoder",
                event = "video_encoder_key_frame_request_rejected",
                "Hardware H.264 encoder rejected the preferred force-key-unit request"
            );
            eros::bail!("GStreamer H.264 encoder rejected a force-key-unit request");
        }

        Ok(())
    }

    pub(super) fn prepare_frame(
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

    pub(crate) async fn receive_packet(&mut self) -> eros::Result<Option<GStreamerRtpPacket>> {
        enum ReceiveEvent {
            Sample(Option<gstreamer::Sample>),
            Terminal(Result<gstreamer::Message, flume::RecvError>),
        }

        let event = {
            let output = poll_fn(|context| Pin::new(&mut self.output).poll_next(context));
            let terminal = self.terminal_messages.recv_async();
            futures_util::pin_mut!(output, terminal);

            match select(output, terminal).await {
                Either::Left((sample, _)) => ReceiveEvent::Sample(sample),
                Either::Right((message, _)) => ReceiveEvent::Terminal(message),
            }
        };

        match event {
            ReceiveEvent::Sample(Some(sample)) => self.packet_from_sample(sample).map(Some),
            ReceiveEvent::Sample(None) => Ok(None),
            ReceiveEvent::Terminal(Ok(message)) => {
                terminal_message_result(&message)?;
                Ok(None)
            }
            ReceiveEvent::Terminal(Err(_)) => {
                eros::bail!("GStreamer H.264 terminal message channel disconnected")
            }
        }
    }

    fn packet_from_sample(
        &mut self,
        sample: gstreamer::Sample,
    ) -> eros::Result<GStreamerRtpPacket> {
        let packet = GStreamerRtpPacket::try_from(sample)?;
        if let Some(probe) = &mut self.probe {
            probe.record_packet(&packet);
        }

        Ok(packet)
    }

    #[cfg(test)]
    pub(crate) async fn wait_terminal(&self) -> eros::Result<()> {
        let message = self
            .terminal_messages
            .recv_async()
            .await
            .with_context(|| "GStreamer H.264 terminal message channel disconnected")?;

        terminal_message_result(&message)
    }

    pub(super) fn find_hardware_h264_encoders() -> eros::Result<Vec<gstreamer::ElementFactory>> {
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

    pub(crate) fn select_hardware_h264_encoder(
        input_caps: &gstreamer::CapsRef,
    ) -> eros::Result<gstreamer::ElementFactory> {
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

    pub(crate) fn is_nv12_dmabuf_input_caps(caps: &gstreamer::CapsRef) -> bool {
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

    pub(crate) fn is_xrgb_dmabuf_input_caps(caps: &gstreamer::CapsRef) -> bool {
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
                .is_ok_and(|format| format == "XR24" || format.starts_with("XR24:"))
    }

    async fn drive<Frames, Commands, SendPacket, SendFuture>(
        &mut self,
        frames: &mut Frames,
        commands: &mut Commands,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
        Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
        SendPacket: FnMut(Vec<GStreamerRtpPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        enum Event {
            Frame(Option<eros::Result<Rc<GbmFramePipelineFrame>>>),
            Command(Option<VideoEncoderCommand>),
            Packet(eros::Result<Option<GStreamerRtpPacket>>),
        }

        let mut input_open = true;
        let mut commands_open = true;
        let mut packet_batch = Vec::new();

        loop {
            if !input_open {
                let Some(packet) = self.receive_packet().await? else {
                    log_incomplete_access_unit(&packet_batch);
                    return Ok(());
                };
                send_rtp_packet(packet, &mut packet_batch, send_packet).await?;
                continue;
            }

            let event = if commands_open {
                let next_frame = poll_fn(|context| Pin::new(&mut *frames).poll_next(context));
                let next_command = poll_fn(|context| Pin::new(&mut *commands).poll_next(context));
                let next_packet = self.receive_packet();
                futures_util::pin_mut!(next_frame, next_command, next_packet);

                let input = select(next_command, next_frame);
                futures_util::pin_mut!(input);
                match select(input, next_packet).await {
                    Either::Left((Either::Left((command, _)), _)) => Event::Command(command),
                    Either::Left((Either::Right((frame, _)), _)) => Event::Frame(frame),
                    Either::Right((packet, _)) => Event::Packet(packet),
                }
            } else {
                let next_frame = poll_fn(|context| Pin::new(&mut *frames).poll_next(context));
                let next_packet = self.receive_packet();
                futures_util::pin_mut!(next_frame, next_packet);

                match select(next_frame, next_packet).await {
                    Either::Left((frame, _)) => Event::Frame(frame),
                    Either::Right((packet, _)) => Event::Packet(packet),
                }
            };

            match event {
                Event::Frame(Some(frame)) => {
                    let frame = self.prepare_frame(
                        frame.with_context(|| "Failed to receive frame-pipeline output")?,
                    )?;
                    self.submit_frame(frame)?;
                }
                Event::Frame(None) => {
                    self.finish()?;
                    input_open = false;
                }
                Event::Command(Some(VideoEncoderCommand::RequestKeyFrame)) => {
                    self.request_key_frame()?;
                    self.submit_latest_frame()?;
                }
                Event::Command(Some(VideoEncoderCommand::SetBitrate(bitrate))) => {
                    self.set_bitrate(bitrate)?;
                }
                Event::Command(None) => commands_open = false,
                Event::Packet(packet) => match packet? {
                    Some(packet) => send_rtp_packet(packet, &mut packet_batch, send_packet).await?,
                    None => {
                        log_incomplete_access_unit(&packet_batch);
                        return Ok(());
                    }
                },
            }
        }
    }

    fn set_bitrate(&self, bitrate: VideoBitrate) -> eros::Result<()> {
        let encoder = self
            .pipeline
            .by_name("h264-encoder")
            .with_context(|| "Running GStreamer pipeline has no H.264 encoder")?;
        configure_low_latency_encoder(&encoder, bitrate)?;
        tracing::info!(
            event = "linux_h264_encoder_bitrate_updated",
            requested_bitrate_bps = bitrate.bits_per_second(),
            effective_bitrate_kbps = encoder.property::<u32>("bitrate"),
            effective_cpb_size_kbits = encoder.property::<u32>("cpb-size"),
            "Updated Linux H.264 encoder bitrate"
        );
        Ok(())
    }
}

fn select_h264_profile(
    encoder: &gstreamer::Element,
) -> eros::Result<(&'static str, gstreamer::Caps)> {
    let source = encoder
        .static_pad("src")
        .with_context(|| "Linux H.264 encoder has no source pad")?;
    let supported = source.query_caps(None);
    for (name, caps_name) in [
        ("high", "high"),
        ("main", "main"),
        ("baseline", "constrained-baseline"),
        ("baseline", "baseline"),
    ] {
        let caps = gstreamer::Caps::builder("video/x-h264")
            .field("profile", caps_name)
            .build();
        if supported.can_intersect(&caps) {
            return Ok((name, caps));
        }
    }
    eros::bail!(
        "Linux H.264 encoder supports none of High, Main, or Baseline: {}",
        supported
    )
}

async fn send_rtp_packet<SendPacket, SendFuture>(
    packet: GStreamerRtpPacket,
    packet_batch: &mut Vec<GStreamerRtpPacket>,
    send_packet: &mut SendPacket,
) -> eros::Result<()>
where
    SendPacket: FnMut(Vec<GStreamerRtpPacket>) -> SendFuture,
    SendFuture: Future<Output = eros::Result<()>>,
{
    let frame_complete = packet.is_frame_end();
    packet_batch.push(packet);
    if frame_complete {
        send_packet(std::mem::take(packet_batch)).await?;
    }
    Ok(())
}

fn log_incomplete_access_unit(packet_batch: &[GStreamerRtpPacket]) {
    if !packet_batch.is_empty() {
        tracing::warn!(
            event = "linux_h264_incomplete_access_unit_dropped",
            packet_count = packet_batch.len(),
            "Dropped an incomplete encoded access unit at end of stream"
        );
    }
}

impl crate::kernel::video_encoder::VideoEncoder for GStreamerVideoEncoder {
    type Input = GbmFramePipelineFrame;
    type Packet = GStreamerRtpPacket;

    fn run<Frames, Commands, SendPacket, SendFuture>(
        frames: Frames,
        commands: Commands,
        parameters: VideoEncoderParameters,
        max_packet_size: usize,
        send_packet: SendPacket,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
        Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
        SendPacket: FnMut(Vec<Self::Packet>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        Self::run_inner(frames, commands, parameters, max_packet_size, send_packet)
    }
}
