use std::{
    future::{Future, poll_fn},
    pin::Pin,
    rc::Rc,
};

use drm::buffer::{DrmFourcc, DrmModifier};
use eros::Context as _;
use futures_core::Stream as _;
use futures_util::future::{Either, select};
use gstreamer::glib::prelude::ObjectExt as _;
use gstreamer::prelude::{
    Cast as _, ElementExt as _, GObjectExtManualGst as _, GstBinExtManual as _, GstObjectExt as _,
};
use gstreamer_allocators::prelude::DmaBufAllocatorExtManual as _;

use crate::infra::platform::{
    dma_buf::DmaBufFrame, frame_pipeline::GbmFramePipelineFrame,
    video_encoder::gstreamer::probe::GStreamerVideoProbe, video_probe::VideoFrameProbe,
};

mod probe;

#[derive(Debug)]
pub(crate) struct GStreamerVideoFrame {
    buffer: gstreamer::Buffer,
    input_caps: gstreamer::Caps,
    probe: Option<VideoFrameProbe>,
}

impl TryFrom<Rc<GbmFramePipelineFrame>> for GStreamerVideoFrame {
    type Error = eros::ErrorUnion;

    fn try_from(source: Rc<GbmFramePipelineFrame>) -> Result<Self, Self::Error> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let frame = &source.buffer;

        if frame.readiness_fence.is_some() {
            eros::bail!("GStreamer input frame still has an unresolved readiness fence");
        }
        let expected_planes = match frame.format {
            DrmFourcc::Nv12 => 2,
            DrmFourcc::Xrgb8888 => 1,
            format => eros::bail!(
                "GStreamer input frame must use NV12 or XRGB8888, got {:?}",
                format
            ),
        };
        if frame.planes.len() != expected_planes {
            eros::bail!(
                "GStreamer {:?} input frame must contain {} planes, got {}",
                frame.format,
                expected_planes,
                frame.planes.len()
            );
        }
        if frame.objects.is_empty() {
            eros::bail!("GStreamer NV12 input frame does not contain DMA-BUF objects");
        }

        let modifier = frame.planes[0].modifier;
        if frame.planes.iter().any(|plane| plane.modifier != modifier) {
            eros::bail!(
                "GStreamer {:?} input frame planes use different DRM modifiers",
                frame.format
            );
        }

        let mut object_offsets = Vec::with_capacity(frame.objects.len());
        let mut buffer_size = 0_usize;
        for object in &frame.objects {
            object_offsets.push(buffer_size);
            buffer_size = buffer_size
                .checked_add(object.size)
                .with_context(|| "GStreamer DMA-BUF object sizes exceed usize")?;
        }

        let mut offsets = Vec::with_capacity(frame.planes.len());
        let mut strides = Vec::with_capacity(frame.planes.len());
        for (plane_index, plane) in frame.planes.iter().enumerate() {
            let Some(object) = frame.objects.get(plane.object_index) else {
                eros::bail!(
                    "GStreamer NV12 plane {} references missing DMA-BUF object {}",
                    plane_index,
                    plane.object_index
                );
            };
            let plane_offset = usize::try_from(plane.offset)
                .with_context(|| "GStreamer NV12 plane offset exceeds usize")?;
            if plane_offset >= object.size {
                eros::bail!(
                    "GStreamer NV12 plane {} offset {} exceeds DMA-BUF object {} size {}",
                    plane_index,
                    plane.offset,
                    plane.object_index,
                    object.size
                );
            }
            offsets.push(
                object_offsets[plane.object_index]
                    .checked_add(plane_offset)
                    .with_context(|| "GStreamer NV12 plane offset exceeds usize")?,
            );
            strides.push(
                i32::try_from(plane.stride)
                    .with_context(|| "GStreamer NV12 plane stride exceeds i32")?,
            );
        }

        let input_caps = dmabuf_caps(frame, modifier)?;
        let allocator = gstreamer_allocators::DmaBufAllocator::new();
        let mut buffer = gstreamer::Buffer::new();
        let Some(buffer_mut) = buffer.get_mut() else {
            eros::bail!("New GStreamer DMA-BUF input buffer is unexpectedly shared");
        };
        for (object_index, object) in frame.objects.iter().enumerate() {
            let fd = object.fd.try_clone().with_context(|| {
                format!(
                    "Failed to duplicate DMA-BUF object {} for GStreamer",
                    object_index
                )
            })?;
            let memory = unsafe { allocator.alloc_dmabuf(fd, object.size) }.with_context(|| {
                format!(
                    "Failed to wrap DMA-BUF object {} as GStreamer memory",
                    object_index
                )
            })?;
            buffer_mut.append_memory(memory);
        }
        gstreamer_video::VideoMeta::add_full(
            buffer_mut,
            gstreamer_video::VideoFrameFlags::empty(),
            gstreamer_video::VideoFormat::DmaDrm,
            frame.size.width,
            frame.size.height,
            &offsets,
            &strides,
        )
        .with_context(|| "Failed to attach NV12 DMA-BUF layout to GStreamer input frame")?;

        validate_dmabuf_buffer(&buffer)?;

        let probe = source.probe.clone();
        if let Some(probe) = &probe {
            let Some(buffer) = buffer.get_mut() else {
                eros::bail!("New GStreamer DMA-BUF input buffer is unexpectedly shared");
            };
            buffer.set_pts(gstreamer::ClockTime::from_nseconds(probe.pts_ns()));
        }

        Ok(Self {
            buffer,
            input_caps,
            probe,
        })
    }
}

impl GStreamerVideoFrame {
    pub(crate) fn input_caps(&self) -> &gstreamer::CapsRef {
        &self.input_caps
    }
}

fn validate_dmabuf_buffer(buffer: &gstreamer::BufferRef) -> eros::Result<()> {
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

    Ok(())
}

fn dmabuf_caps(frame: &DmaBufFrame, modifier: DrmModifier) -> eros::Result<gstreamer::Caps> {
    let width = i32::try_from(frame.size.width)
        .with_context(|| "GStreamer NV12 frame width exceeds i32")?;
    let height = i32::try_from(frame.size.height)
        .with_context(|| "GStreamer NV12 frame height exceeds i32")?;
    let drm_format = if modifier == DrmModifier::Invalid {
        match frame.format {
            DrmFourcc::Nv12 => String::from("NV12"),
            DrmFourcc::Xrgb8888 => String::from("XR24"),
            format => eros::bail!("Unsupported modifierless DMA-BUF format: {:?}", format),
        }
    } else {
        gstreamer_video::dma_drm_fourcc_to_string(frame.format as u32, modifier.into()).to_string()
    };

    Ok(gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .field("drm-format", drm_format)
        .field("width", width)
        .field("height", height)
        .field("framerate", gstreamer::Fraction::new(0, 1))
        .field("interlace-mode", "progressive")
        .field("colorimetry", "bt709")
        .build())
}

pub(crate) fn hardware_h264_encoder_for(
    frame: &DmaBufFrame,
) -> eros::Result<gstreamer::glib::GString> {
    gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
    let modifier = frame
        .planes
        .first()
        .with_context(|| "NV12 DMA-BUF probe frame has no planes")?
        .modifier;
    let caps = dmabuf_caps(frame, modifier)?;
    Ok(GStreamerVideoEncoder::select_hardware_h264_encoder(&caps)?.name())
}

pub(crate) fn va_vpp_input_modifier(format: DrmFourcc) -> eros::Result<DrmModifier> {
    gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
    let factory = gstreamer::ElementFactory::find("vapostproc")
        .with_context(|| "GStreamer VAAPI video postprocessor is unavailable")?;

    for template in factory
        .static_pad_templates()
        .into_iter()
        .filter(|template| template.direction() == gstreamer::PadDirection::Sink)
    {
        for (structure, features) in template.caps().iter_with_features() {
            if !features.contains("memory:DMABuf") {
                continue;
            }
            let Ok(value) = structure.value("drm-format") else {
                continue;
            };
            let mut candidates = Vec::new();
            if let Ok(candidate) = value.get::<&str>() {
                candidates.push(candidate.to_owned());
            } else if let Ok(candidate_list) = value.get::<gstreamer::List>() {
                candidates.extend(
                    candidate_list
                        .as_slice()
                        .iter()
                        .filter_map(|candidate| candidate.get::<&str>().ok())
                        .map(str::to_owned),
                );
            }

            for candidate in candidates {
                let Ok((fourcc, modifier)) = gstreamer_video::dma_drm_fourcc_from_str(&candidate)
                else {
                    continue;
                };
                if fourcc == format as u32 {
                    return Ok(DrmModifier::from(modifier));
                }
            }
        }
    }

    eros::bail!(
        "GStreamer VAAPI video postprocessor exposes no {:?} DMA-BUF modifier",
        format
    )
}

#[derive(Debug)]
pub(crate) struct GStreamerRtpPacket {
    payload: bytes::Bytes,
    marker: bool,
    pts_ns: Option<u64>,
}

impl TryFrom<gstreamer::Sample> for GStreamerRtpPacket {
    type Error = eros::ErrorUnion;

    fn try_from(sample: gstreamer::Sample) -> Result<Self, Self::Error> {
        let Some(caps) = sample.caps() else {
            eros::bail!("GStreamer encoded packet sample is missing caps");
        };

        if !caps.is_subset(&h264_rtp_caps()) {
            eros::bail!("GStreamer encoded packet has non-H.264 RTP caps {}", caps);
        }

        let Some(buffer) = sample.buffer_owned() else {
            eros::bail!("GStreamer H.264 RTP sample is missing its buffer");
        };
        let pts_ns = buffer.pts().map(gstreamer::ClockTime::nseconds);
        let Ok(buffer) = buffer.into_mapped_buffer_readable() else {
            eros::bail!("Failed to map GStreamer H.264 RTP packet for reading");
        };
        let payload = bytes::Bytes::from_owner(buffer);

        if payload.len() < 12 {
            eros::bail!("GStreamer H.264 RTP packet is shorter than its 12-byte fixed header");
        }
        if payload[0] >> 6 != 2 {
            eros::bail!(
                "GStreamer H.264 RTP packet has unsupported RTP version {}",
                payload[0] >> 6
            );
        }
        let marker = payload[1] & 0x80 != 0;

        Ok(Self {
            payload,
            marker,
            pts_ns,
        })
    }
}

impl GStreamerRtpPacket {
    pub(crate) fn is_frame_end(&self) -> bool {
        self.marker
    }

    fn pts_ns(&self) -> Option<u64> {
        self.pts_ns
    }

    fn payload_len(&self) -> usize {
        self.payload.len()
    }
}

impl From<GStreamerRtpPacket> for bytes::Bytes {
    fn from(packet: GStreamerRtpPacket) -> Self {
        packet.payload
    }
}

#[derive(Debug)]
pub(crate) struct GStreamerVideoEncoder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    element: gstreamer::Element,
    sink: gstreamer_app::AppSink,
    output: gstreamer_app::app_sink::AppSinkStream,
    terminal_messages: flume::Receiver<gstreamer::Message>,
    input_caps: gstreamer::Caps,
    probe: Option<GStreamerVideoProbe>,
}

impl GStreamerVideoEncoder {
    async fn run_inner<Frames, SendPacket, SendFuture>(
        mut frames: Frames,
        max_rtp_packet_size: usize,
        mut send_packet: SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
        SendPacket: FnMut(GStreamerRtpPacket) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        let Some(first_frame) = poll_fn(|context| Pin::new(&mut frames).poll_next(context)).await
        else {
            return Ok(());
        };
        let first_frame = GStreamerVideoFrame::try_from(
            first_frame.with_context(|| "Failed to receive first frame-pipeline output")?,
        )?;
        let mut encoder = Self::new(first_frame, max_rtp_packet_size)?;
        let result = encoder.drive(&mut frames, &mut send_packet).await;
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
        max_rtp_packet_size: usize,
    ) -> eros::Result<Self> {
        let enable_probing = first_frame.probe.is_some();
        let mut encoder = Self::create(
            first_frame.input_caps(),
            max_rtp_packet_size,
            enable_probing,
        )?;
        encoder.submit_frame(first_frame)?;
        encoder.start()?;

        Ok(encoder)
    }

    fn create(
        input_caps: &gstreamer::CapsRef,
        max_rtp_packet_size: usize,
        enable_probing: bool,
    ) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let rtp_mtu = rtp_mtu(max_rtp_packet_size)?;
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
        configure_low_latency_encoder(&element);
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
            Some((vpp, filter))
        } else {
            None
        };

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
        let output = sink.stream();

        let pipeline = gstreamer::Pipeline::new();
        let base_elements = [
            source.upcast_ref(),
            &element,
            &parser,
            &payloader,
            sink.upcast_ref(),
        ];
        pipeline
            .add_many(base_elements)
            .with_context(|| "Failed to add H.264 encoding elements to GStreamer pipeline")?;
        if let Some((vpp, filter)) = &vpp {
            pipeline
                .add_many([vpp, filter])
                .with_context(|| "Failed to add VAAPI VPP elements to GStreamer pipeline")?;
            gstreamer::Element::link_many([
                source.upcast_ref(),
                vpp,
                filter,
                &element,
                &parser,
                &payloader,
                sink.upcast_ref(),
            ])
            .with_context(|| "Failed to link VAAPI VPP H.264 RTP encoding pipeline")?;
        } else {
            gstreamer::Element::link_many(base_elements)
                .with_context(|| "Failed to link GStreamer H.264 RTP encoding pipeline")?;
        }
        let probe = if enable_probing {
            Some(GStreamerVideoProbe::new(
                &source,
                vpp.as_ref().map(|(vpp, _)| vpp),
                &element,
            )?)
        } else {
            None
        };
        let terminal_messages = terminal_messages(&pipeline)?;

        Ok(Self {
            pipeline,
            source,
            element,
            sink,
            output,
            terminal_messages,
            input_caps,
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
        if frame.input_caps() != self.input_caps.as_ref() {
            eros::bail!(
                "GStreamer encoder input caps changed from {} to {}",
                self.input_caps,
                frame.input_caps()
            );
        }

        if let Some(frame_probe) = frame.probe.take()
            && let Some(probe) = &mut self.probe
        {
            probe.submit_frame(frame_probe);
        }

        self.source
            .push_buffer(frame.buffer)
            .with_context(|| "Failed to submit DMA-BUF frame to GStreamer H.264 encoder")?;

        Ok(())
    }

    pub(crate) async fn receive_packet(&mut self) -> eros::Result<Option<GStreamerRtpPacket>> {
        let output = poll_fn(|context| Pin::new(&mut self.output).poll_next(context));
        let terminal = self.terminal_messages.recv_async();
        futures_util::pin_mut!(output, terminal);

        match select(output, terminal).await {
            Either::Left((Some(sample), _)) => {
                let packet = GStreamerRtpPacket::try_from(sample)?;
                if let Some(probe) = &mut self.probe {
                    probe.record_packet(&packet);
                }
                Ok(Some(packet))
            }
            Either::Left((None, _)) => Ok(None),
            Either::Right((Ok(message), _)) => {
                terminal_message_result(&message)?;
                Ok(None)
            }
            Either::Right((Err(_), _)) => {
                eros::bail!("GStreamer H.264 terminal message channel disconnected")
            }
        }
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

    fn is_xrgb_dmabuf_input_caps(caps: &gstreamer::CapsRef) -> bool {
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

    async fn drive<Frames, SendPacket, SendFuture>(
        &mut self,
        frames: &mut Frames,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
        SendPacket: FnMut(GStreamerRtpPacket) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        enum Event {
            Frame(Option<eros::Result<Rc<GbmFramePipelineFrame>>>),
            Packet(eros::Result<Option<GStreamerRtpPacket>>),
        }

        let mut input_open = true;

        loop {
            if !input_open {
                let Some(packet) = self.receive_packet().await? else {
                    return Ok(());
                };
                send_packet(packet).await?;
                continue;
            }

            let event = {
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
                    let frame = GStreamerVideoFrame::try_from(
                        frame.with_context(|| "Failed to receive frame-pipeline output")?,
                    )?;
                    self.submit_frame(frame)?;
                }
                Event::Frame(None) => {
                    self.finish()?;
                    input_open = false;
                }
                Event::Packet(packet) => match packet? {
                    Some(packet) => send_packet(packet).await?,
                    None => return Ok(()),
                },
            }
        }
    }
}

impl crate::kernel::video_encoder::VideoEncoder for GStreamerVideoEncoder {
    type Input = GbmFramePipelineFrame;
    type Packet = GStreamerRtpPacket;

    fn run<Frames, SendPacket, SendFuture>(
        frames: Frames,
        max_packet_size: usize,
        send_packet: SendPacket,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
        SendPacket: FnMut(Self::Packet) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        Self::run_inner(frames, max_packet_size, send_packet)
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

fn va_vpp_output_caps(input: &gstreamer::CapsRef) -> eros::Result<gstreamer::Caps> {
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
        .build())
}

fn configure_low_latency_encoder(encoder: &gstreamer::Element) {
    let is_vaapi = encoder
        .factory()
        .is_some_and(|factory| factory.name().starts_with("va"));
    if !is_vaapi {
        return;
    }

    encoder.set_property("b-frames", 0_u32);
    encoder.set_property("ref-frames", 1_u32);
    encoder.set_property("target-usage", 7_u32);
    encoder.set_property_from_str("rate-control", "cqp");
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
        ElementExt as _, GObjectExtManualGst as _, GstBinExt as _, GstBinExtManual as _,
        PadExtManual as _,
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
                GbmFramePipelineManager, GbmFramePipelineManagerState, KmsScreenCaptureManager,
                KmsScreenCaptureManagerState,
                dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
                frame_pipeline::GbmFramePipelineFrame,
                gpu::{GpuContext, GpuDevice},
                screen_capture::{KmsCaptureLease, KmsFrameReceiver},
                video_encoder::gstreamer::{
                    GStreamerRtpPacket, GStreamerVideoEncoder, GStreamerVideoFrame,
                    create_required_element, h264_rtp_caps, va_vpp_input_modifier,
                    validate_dmabuf_buffer,
                },
            },
        },
        kernel::{
            frame_pipeline::{FramePipelineManager, FramePipelineParameters},
            geometry::PixelSize,
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

        fn poll_next(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<Option<Self::Item>> {
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
        let source_size = host_video_test_source_size(&screen_name);
        let target_size = host_video_test_target_size(source_size);
        eprintln!(
            "Host video test source: {}x{}, target: {}x{}",
            source_size.width, source_size.height, target_size.width, target_size.height
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
                capture: KmsScreenCaptureManagerState::new(true, reaper_handle.clone()),
                pipeline: GbmFramePipelineManagerState::new(reaper_handle),
                screens: vec![host_video_test_screen(screen_name, source_size)],
            };
            let frames = GbmFramePipelineManager::inj_ref_mut(&mut deps)
                .subscribe(
                    &ScreenId(0),
                    FramePipelineParameters {
                        frame_size: target_size,
                    },
                )
                .expect("Host video frame pipeline should start");
            let frames = TimedFrames::new(frames, run_duration);
            let encoding =
                GStreamerVideoEncoder::run_inner(frames, MAX_RTP_PACKET_SIZE, move |packet| {
                    assert!(
                        packet.payload.len() <= MAX_RTP_PACKET_SIZE,
                        "Encoded RTP packet should respect the transport packet size"
                    );
                    rtp_packets_for_callback.set(rtp_packets_for_callback.get() + 1);
                    if packet.is_frame_end() {
                        encoded_frames_for_callback.set(encoded_frames_for_callback.get() + 1);
                    }

                    ready(Ok::<(), eros::ErrorUnion>(()))
                });

            let result = compio::time::timeout(test_timeout, encoding).await;
            result
                .expect("Host video smoke test should finish within its shutdown timeout")
                .expect("Host video chain should encode H.264 RTP frames");
        });

        assert!(
            encoded_frames.get() >= REQUIRED_ENCODED_FRAMES,
            "Host video chain should encode at least {REQUIRED_ENCODED_FRAMES} frames, got {}",
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
        let encoder = GStreamerVideoEncoder::create(&input_caps, MAX_RTP_PACKET_SIZE, false)
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

        GStreamerVideoEncoder::create(&input_caps, 1_200, false)
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

        let error = GStreamerVideoEncoder::create(&input_caps, 27, false)
            .expect_err("The RTP payloader should reject packet sizes below 28 bytes");

        assert!(error.to_string().contains("at least 28 bytes"));
    }

    #[test]
    #[ignore = "run through scripts/test-gstreamer"]
    fn starts_and_stops_hardware_h264_pipeline() {
        gstreamer::init().expect("GStreamer should initialize before inspecting encoder caps");
        let input_caps = registered_nv12_dmabuf_input_caps();
        let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, false)
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
        let encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, false)
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
        let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, false)
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
        let mut encoder = GStreamerVideoEncoder::create(&input_caps, 1_200, false)
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
        let mut encoder = GStreamerVideoEncoder::new(frame, 1_200)
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
        let mut encoder = GStreamerVideoEncoder::create(frame.input_caps(), 1_200, false)
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

        assert_eq!(packet.payload.len(), 12);
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

        GStreamerRtpPacket::try_from(sample)
            .expect_err("A non-H.264 RTP sample should be rejected");
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
                "VAAPI frame {frame_index}: cold={}, submit_to_vpp_ms={:.3}, vpp_ms={:.3}, encode_ms={:.3}, total_ms={:.3}",
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
            "VAAPI warm average: frames={warm_frames}, vpp_ms={:.3}, encode_ms={:.3}, total_ms={:.3}",
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
            .field("framerate", gstreamer::Fraction::new(0, 1))
            .build();
        let filter = create_required_element("capsfilter", "vaapi-test-output-caps")
            .expect("GStreamer capsfilter should be available");
        filter.set_property("caps", &output_caps);
        let encoder = create_required_element("vah264enc", "vaapi-test-encoder")
            .expect("GStreamer VAAPI H.264 encoder should be available");
        encoder.set_property("b-frames", 0_u32);
        encoder.set_property("ref-frames", 1_u32);
        encoder.set_property("target-usage", 7_u32);
        encoder.set_property_from_str("rate-control", "cqp");
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
            .field("framerate", gstreamer::Fraction::new(0, 1))
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

    fn init_host_video_tracing() {
        let targets = Targets::new()
            .with_default(LevelFilter::WARN)
            .with_target("rabbit::video_probe", LevelFilter::TRACE)
            .with_target("rabbit::frame_pipeline", LevelFilter::DEBUG);

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

    fn host_video_test_source_size(screen_name: &str) -> PixelSize {
        let (_reaper, reaper_handle) =
            WorkerReaper::new().expect("Test worker reaper should start");
        let ScreenCaptureSource { lease, receiver } =
            KmsCaptureLease::new(screen_name.to_owned(), false, reaper_handle)
                .expect("KMS capture source should start");
        let (device, frames) = receiver.into_parts();
        device
            .recv()
            .expect("KMS capture worker should report its GPU")
            .expect("KMS capture GPU discovery should succeed");
        let frame = frames
            .recv()
            .expect("KMS capture worker should remain connected")
            .expect("KMS capture worker should publish one frame");
        let size = frame.buffer.size;
        drop(lease);

        size
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

    fn host_video_test_screen(name: String, resolution: PixelSize) -> Screen {
        Screen {
            id: ScreenId(0),
            name,
            resolution,
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
        const WIDTH: u32 = 16;
        const HEIGHT: u32 = 16;
        const Y_SIZE: usize = WIDTH as usize * HEIGHT as usize;
        const BUFFER_SIZE: usize = Y_SIZE + Y_SIZE / 2;

        let file = File::open("/dev/zero").expect("The test DMA-BUF fd should open");
        let frame = Rc::new(GbmFramePipelineFrame {
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
                        offset: u32::try_from(Y_SIZE)
                            .expect("The test Y plane size should fit u32"),
                        stride: WIDTH,
                        modifier: DrmModifier::Invalid,
                    },
                ],
                readiness_fence: None,
            },
            probe: None,
        });

        GStreamerVideoFrame::try_from(frame)
            .expect("The test buffer should satisfy the encoder input boundary")
    }
}
