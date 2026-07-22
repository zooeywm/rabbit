use std::{
    future::{Future, poll_fn},
    os::fd::BorrowedFd,
    pin::Pin,
};

use drm::buffer::{DrmFourcc, DrmModifier};
use eros::Context as _;
use futures_util::future::{Either, select};
use gstreamer::glib::prelude::Cast as _;
use gstreamer::prelude::{ElementExt as _, GstBinExtManual as _, GstObjectExt as _};
use tracing::info;

use crate::{
    infra::platform::{
        client_video_probe::{ClientVideoDecodeProbe, ClientVideoFrameProbe},
        dma_buf::{DmaBufFrame, DmaBufObject, DmaBufPlane},
    },
    kernel::{
        geometry::PixelSize, screen_manager::ScreenId, session::ReceivedVideoFrame,
        video_decoder::VideoDecoder,
    },
};

pub(crate) struct GStreamerVideoDecoder {
    pipeline: gstreamer::Pipeline,
    source: gstreamer_app::AppSrc,
    output: flume::Receiver<DecodedSample>,
    terminal_messages: flume::Receiver<gstreamer::Message>,
    screen_id: Option<ScreenId>,
    probe: Option<ClientVideoDecodeProbe>,
}

struct DecodedSample {
    sample: gstreamer::Sample,
    probe: Option<ClientVideoFrameProbe>,
}

#[derive(Debug)]
pub(crate) struct GStreamerDecodedFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) buffer: DmaBufFrame,
    pub(crate) probe: Option<ClientVideoFrameProbe>,
    _owner: gstreamer::Buffer,
}

impl GStreamerVideoDecoder {
    fn create(enable_probing: bool) -> eros::Result<Self> {
        gstreamer::init().with_context(|| "Failed to initialize GStreamer")?;
        let input_caps = h264_rtp_caps();
        let output_caps = decoded_dma_buf_caps();
        let factory = Self::select_hardware_h264_decoder(&output_caps)?;
        let factory_name = factory.name();
        info!(
            event = "video_decoder_selected",
            factory = %factory_name,
            "Selected hardware H.264 DMA-BUF decoder"
        );
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
        let (probe, decoded_probes) = if enable_probing {
            let (probe, decoded_probes) = ClientVideoDecodeProbe::new(&decoder)?;
            (Some(probe), Some(decoded_probes))
        } else {
            (None, None)
        };
        let (decoded_frames, output) = flume::bounded(1);
        let stale_frame = output.clone();
        sink.set_callbacks(
            gstreamer_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let Ok(sample) = sink.pull_sample() else {
                        return Err(gstreamer::FlowError::Error);
                    };
                    let probe =
                        decoded_probes
                            .as_ref()
                            .and_then(|probes| match probes.try_recv() {
                                Ok(probe) => Some(probe),
                                Err(error) => {
                                    tracing::warn!(
                                        target: "rabbit::client_video_probe",
                                        error = ?error,
                                        "Decoded sample has no matching client video probe"
                                    );
                                    None
                                }
                            });
                    let mut decoded = DecodedSample { sample, probe };
                    loop {
                        match decoded_frames.try_send(decoded) {
                            Ok(()) => return Ok(gstreamer::FlowSuccess::Ok),
                            Err(flume::TrySendError::Full(returned)) => {
                                decoded = returned;
                                match stale_frame.try_recv() {
                                    Ok(_) | Err(flume::TryRecvError::Empty) => {}
                                    Err(flume::TryRecvError::Disconnected) => {
                                        return Err(gstreamer::FlowError::Eos);
                                    }
                                }
                            }
                            Err(flume::TrySendError::Disconnected(_)) => {
                                return Err(gstreamer::FlowError::Eos);
                            }
                        }
                    }
                })
                .propose_allocation(|_, query| {
                    query.add_allocation_meta::<gstreamer_video::VideoMeta>(None);
                    true
                })
                .build(),
        );

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
        let terminal_messages = terminal_messages(&pipeline)?;

        Ok(Self {
            pipeline,
            source,
            output,
            terminal_messages,
            screen_id: None,
            probe,
        })
    }

    fn start(&self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Playing)
            .with_context(|| "Failed to start GStreamer H.264 decoding pipeline")?;
        Ok(())
    }

    fn stop(&self) -> eros::Result<()> {
        self.pipeline
            .set_state(gstreamer::State::Null)
            .with_context(|| "Failed to stop GStreamer H.264 decoding pipeline")?;
        Ok(())
    }

    fn finish(&self) -> eros::Result<()> {
        self.source
            .end_of_stream()
            .with_context(|| "Failed to finish GStreamer H.264 RTP input")?;
        Ok(())
    }

    fn submit_input(&mut self, frame: ReceivedVideoFrame) -> eros::Result<()> {
        match self.screen_id {
            Some(screen_id) if screen_id != frame.screen_id => eros::bail!(
                "GStreamer decoder for screen {} received screen {} input",
                screen_id.get(),
                frame.screen_id.get()
            ),
            None => self.screen_id = Some(frame.screen_id),
            Some(_) => {}
        }

        if let Some(probe) = &mut self.probe {
            let rtp_bytes = frame
                .packets
                .iter()
                .fold(0_usize, |total, packet| total.saturating_add(packet.len()));
            probe.submit_frame(frame.packets.len(), rtp_bytes)?;
        }

        for packet in frame.packets {
            self.source
                .push_buffer(gstreamer::Buffer::from_slice(packet))
                .with_context(|| {
                    format!(
                        "Failed to submit screen {} H.264 RTP packet to GStreamer decoder",
                        frame.screen_id.get()
                    )
                })?;
        }

        Ok(())
    }

    async fn receive_frame(&mut self) -> eros::Result<Option<GStreamerDecodedFrame>> {
        enum ReceiveEvent {
            Sample(Result<DecodedSample, flume::RecvError>),
            Terminal(Result<gstreamer::Message, flume::RecvError>),
        }

        let event = {
            let output = self.output.recv_async();
            let terminal = self.terminal_messages.recv_async();
            futures_util::pin_mut!(output, terminal);

            match select(output, terminal).await {
                Either::Left((sample, _)) => ReceiveEvent::Sample(sample),
                Either::Right((message, _)) => ReceiveEvent::Terminal(message),
            }
        };

        match event {
            ReceiveEvent::Sample(Ok(decoded)) => {
                let screen_id = self
                    .screen_id
                    .with_context(|| "GStreamer decoder produced a frame before receiving input")?;
                Ok(Some(GStreamerDecodedFrame::try_from_sample(
                    screen_id,
                    decoded.sample,
                    decoded.probe,
                )?))
            }
            ReceiveEvent::Sample(Err(_)) => {
                eros::bail!("GStreamer decoded-frame channel disconnected")
            }
            ReceiveEvent::Terminal(Ok(message)) => {
                terminal_message_result(&message)?;
                Ok(None)
            }
            ReceiveEvent::Terminal(Err(_)) => {
                eros::bail!("GStreamer H.264 decoder terminal channel disconnected")
            }
        }
    }

    async fn drive<Inputs, PresentFrame, PresentFuture>(
        &mut self,
        inputs: &mut Inputs,
        present_frame: &mut PresentFrame,
    ) -> eros::Result<()>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(GStreamerDecodedFrame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        enum Event {
            Input(Option<eros::Result<ReceivedVideoFrame>>),
            Frame(Box<eros::Result<Option<GStreamerDecodedFrame>>>),
        }

        let mut input_open = true;
        loop {
            if !input_open {
                let Some(frame) = self.receive_frame().await? else {
                    return Ok(());
                };
                present_frame(frame).await?;
                continue;
            }

            let event = {
                let frame = self.receive_frame();
                let input = poll_fn(|context| Pin::new(&mut *inputs).poll_next(context));
                futures_util::pin_mut!(frame, input);

                match select(frame, input).await {
                    Either::Left((frame, _)) => Event::Frame(Box::new(frame)),
                    Either::Right((input, _)) => Event::Input(input),
                }
            };

            match event {
                Event::Input(Some(input)) => self.submit_input(
                    input.with_context(|| "Failed to receive H.264 decoder input")?,
                )?,
                Event::Input(None) => {
                    self.finish()?;
                    input_open = false;
                }
                Event::Frame(frame) => match *frame {
                    Ok(Some(frame)) => present_frame(frame).await?,
                    Ok(None) => return Ok(()),
                    Err(error) => return Err(error),
                },
            }
        }
    }

    async fn run_inner<Inputs, PresentFrame, PresentFuture>(
        mut inputs: Inputs,
        mut present_frame: PresentFrame,
        enable_probing: bool,
    ) -> eros::Result<()>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(GStreamerDecodedFrame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        let Some(first_input) = poll_fn(|context| Pin::new(&mut inputs).poll_next(context)).await
        else {
            return Ok(());
        };
        let mut decoder = Self::create(enable_probing)?;
        decoder.start()?;
        let result = match first_input {
            Ok(first_input) => {
                decoder.submit_input(first_input)?;
                decoder.drive(&mut inputs, &mut present_frame).await
            }
            Err(error) => Err(error),
        };
        let stop = decoder
            .stop()
            .with_context(|| "Failed to stop GStreamer video decoder");

        match (result, stop) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(error),
            (Err(error), Err(stop_error)) => eros::bail!(
                "Video decoding failed: {}; additionally failed to stop decoder: {}",
                error,
                stop_error
            ),
        }
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
    type Frame = GStreamerDecodedFrame;

    fn run<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        Self::run_inner(inputs, present_frame, false)
    }
}

impl GStreamerVideoDecoder {
    pub(crate) fn run_with_probing<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
        enable_probing: bool,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(GStreamerDecodedFrame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        Self::run_inner(inputs, present_frame, enable_probing)
    }
}

impl GStreamerDecodedFrame {
    fn try_from_sample(
        screen_id: ScreenId,
        sample: gstreamer::Sample,
        probe: Option<ClientVideoFrameProbe>,
    ) -> eros::Result<Self> {
        let caps = sample
            .caps()
            .with_context(|| "GStreamer decoded frame sample is missing caps")?;
        if !caps.is_subset(&decoded_dma_buf_caps()) {
            eros::bail!("GStreamer decoded frame has non-DMA-BUF caps {}", caps);
        }
        let info = gstreamer_video::VideoInfoDmaDrm::from_caps(caps)
            .with_context(|| "Failed to parse decoded DMA-BUF video caps")?;
        let owner = sample
            .buffer_owned()
            .with_context(|| "GStreamer decoded frame sample is missing its buffer")?;
        let video = owner
            .meta::<gstreamer_video::VideoMeta>()
            .with_context(|| "GStreamer decoded DMA-BUF frame is missing VideoMeta")?;
        if video.n_planes() == 0 {
            eros::bail!("GStreamer decoded DMA-BUF frame has no planes");
        }
        if owner.n_memory() == 0 {
            eros::bail!("GStreamer decoded DMA-BUF frame has no memory objects");
        }

        let mut objects = Vec::with_capacity(owner.n_memory());
        for (object_index, memory) in owner.iter_memories().enumerate() {
            let dma_buf = memory
                .downcast_memory_ref::<gstreamer_allocators::DmaBufMemory>()
                .with_context(|| {
                    format!(
                        "GStreamer decoded frame memory {} is not DMA-BUF",
                        object_index
                    )
                })?;
            let borrowed = unsafe { BorrowedFd::borrow_raw(dma_buf.fd()) };
            let fd = borrowed.try_clone_to_owned().with_context(|| {
                format!(
                    "Failed to duplicate GStreamer decoded DMA-BUF object {}",
                    object_index
                )
            })?;
            objects.push(DmaBufObject::try_from(fd).with_context(|| {
                format!(
                    "Failed to inspect GStreamer decoded DMA-BUF object {}",
                    object_index
                )
            })?);
        }

        let modifier = DrmModifier::from(info.modifier());
        let mut planes = Vec::with_capacity(video.n_planes() as usize);
        for (plane_index, (&offset, &stride)) in
            video.offset().iter().zip(video.stride()).enumerate()
        {
            let (memory_range, skip) = owner
                .find_memory(offset..offset.saturating_add(1))
                .with_context(|| {
                    format!(
                        "Failed to locate GStreamer decoded DMA-BUF plane {} memory",
                        plane_index
                    )
                })?;
            if memory_range.len() != 1 {
                eros::bail!(
                    "GStreamer decoded DMA-BUF plane {} spans {} memory objects",
                    plane_index,
                    memory_range.len()
                );
            }
            let memory = owner.peek_memory(memory_range.start);
            let object_offset = memory
                .offset()
                .checked_add(skip)
                .with_context(|| "Decoded DMA-BUF plane offset exceeds usize")?;
            planes.push(DmaBufPlane {
                object_index: memory_range.start,
                offset: u32::try_from(object_offset)
                    .with_context(|| "Decoded DMA-BUF plane offset exceeds u32")?,
                stride: u32::try_from(stride)
                    .with_context(|| "Decoded DMA-BUF plane stride is negative")?,
                modifier,
            });
        }

        Ok(Self {
            screen_id,
            buffer: DmaBufFrame {
                size: PixelSize {
                    width: info.width(),
                    height: info.height(),
                },
                format: DrmFourcc::try_from(info.fourcc())
                    .with_context(|| "Decoded DMA-BUF frame has an unknown DRM fourcc")?,
                objects,
                planes,
                readiness_fence: None,
                lease: None,
                va_backing: None,
            },
            probe,
            _owner: owner,
        })
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

fn terminal_messages(
    pipeline: &gstreamer::Pipeline,
) -> eros::Result<flume::Receiver<gstreamer::Message>> {
    let bus = pipeline
        .bus()
        .with_context(|| "GStreamer H.264 decoding pipeline has no Bus")?;
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
            let source = error
                .src()
                .map(|source| source.path_string().to_string())
                .unwrap_or_else(|| "unknown source".to_string());
            let message = error.error();

            match error.debug() {
                Some(debug) => eros::bail!(
                    "GStreamer H.264 decoding pipeline failed at {}: {}; debug: {}",
                    source,
                    message,
                    debug
                ),
                None => eros::bail!(
                    "GStreamer H.264 decoding pipeline failed at {}: {}",
                    source,
                    message
                ),
            }
        }
        _ => eros::bail!("GStreamer decoder terminal channel received a non-terminal message"),
    }
}

// Focused tests: cargo test infra::platform::video_decoder::tests:: --lib
// Hardware construction test: scripts/test-gstreamer
#[cfg(test)]
mod tests {
    use gstreamer::glib::prelude::Cast as _;
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
        let decoder = GStreamerVideoDecoder::create(false)
            .expect("A hardware H.264 DMA-BUF decoder should be available");
        let factory = decoder
            .pipeline
            .by_name("h264-decoder")
            .expect("The pipeline should retain its decoder element")
            .factory()
            .expect("The selected decoder should have a factory");
        let sink = decoder
            .pipeline
            .by_name("decoded-output")
            .expect("The pipeline should retain its decoded-output element")
            .downcast::<gstreamer_app::AppSink>()
            .expect("The decoded-output element should remain an appsink");

        assert!(factory.can_src_any_caps(&decoded_dma_buf_caps()));
        assert_eq!(
            decoder
                .source
                .caps()
                .expect("Decoder appsrc should retain its RTP caps"),
            h264_rtp_caps()
        );
        assert_eq!(
            sink.caps()
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
