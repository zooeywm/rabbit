use std::{
    collections::HashMap,
    future::Future,
    rc::Rc,
    time::{Duration, Instant},
};

use bytes::{BufMut as _, Bytes, BytesMut};
use eros::Context as _;
use futures_util::{
    StreamExt as _,
    future::{Either, select},
};
use tracing::{debug, info, trace, warn};
use windows::{
    Win32::{
        Foundation::{E_FAIL, E_NOTIMPL, E_POINTER, RECT},
        Graphics::{
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VIDEO_ENCODER,
                D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device,
                ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D, ID3D11VideoContext,
                ID3D11VideoContext1, ID3D11VideoDevice, ID3D11VideoProcessorEnumerator,
            },
            Dxgi::Common::{
                DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
            },
            Dxgi::IDXGIDevice,
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonLowLatency,
            CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonQualityVsSpeed,
            CODECAPI_AVEncCommonRateControlMode, CODECAPI_AVEncCommonRealTime,
            CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize,
            CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode, ICodecAPI, IMFActivate,
            IMFAsyncCallback, IMFAsyncCallback_Impl, IMFAsyncResult, IMFDXGIDeviceManager,
            IMFMediaEventGenerator, IMFMediaType, IMFSample, IMFTransform,
            METransformDrainComplete, METransformHaveOutput, METransformNeedInput,
            MF_E_NO_MORE_TYPES, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE,
            MF_LOW_LATENCY, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
            MF_MT_FIXED_SIZE_SAMPLES, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
            MF_MT_MAJOR_TYPE, MF_MT_MAX_KEYFRAME_SPACING, MF_MT_MPEG2_PROFILE,
            MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SAMPLE_SIZE, MF_MT_SUBTYPE, MF_SA_D3D11_AWARE,
            MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateDXGIDeviceManager,
            MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
            MFMediaType_Video, MFSTARTUP_FULL, MFStartup, MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER, MFT_ENUM_FLAG_SYNCMFT,
            MFT_ENUM_HARDWARE_URL_Attribute, MFT_ENUM_HARDWARE_VENDOR_ID_Attribute,
            MFT_FRIENDLY_NAME_Attribute, MFT_MESSAGE_COMMAND_DRAIN,
            MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_END_OF_STREAM,
            MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
            MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
            MFT_TRANSFORM_CLSID_Attribute, MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12,
            MFVideoInterlace_Progressive, eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_Base,
            eAVEncH264VProfile_High, eAVEncH264VProfile_Main,
        },
        System::{
            Com::CoTaskMemFree,
            Variant::{VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_I4, VT_I8, VT_UI4, VT_UI8},
        },
    },
    core::Interface as _,
};

use crate::{
    infra::platform::{
        frame_pipeline::WindowsFramePipelineFrame,
        host_video_probe::{HostVideoFrameProbe, HostVideoProbeReporter},
    },
    kernel::{
        geometry::{FrameRate, PixelSize},
        video_encoder::{
            VideoBitrate, VideoCodec, VideoEncoder, VideoEncoderCommand, VideoEncoderParameters,
            VideoFrameRateMode,
        },
    },
};

use super::video_color::{
    captured_rgb_color_space, encoded_ycbcr_color_space, legacy_captured_rgb_color_space,
    legacy_encoded_ycbcr_color_space, set_media_type_colorimetry,
};

#[derive(Debug, Clone)]
pub(crate) struct WindowsVideoPacket(Bytes);

impl From<WindowsVideoPacket> for Bytes {
    fn from(packet: WindowsVideoPacket) -> Self {
        packet.0
    }
}

pub(crate) struct WindowsVideoEncoder;

impl VideoEncoder for WindowsVideoEncoder {
    type Input = WindowsFramePipelineFrame;
    type Packet = WindowsVideoPacket;

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
        run_windows_encoder(frames, commands, parameters, max_packet_size, send_packet)
    }
}

async fn run_windows_encoder<Frames, Commands, SendPacket, SendFuture>(
    mut frames: Frames,
    mut commands: Commands,
    parameters: VideoEncoderParameters,
    max_packet_size: usize,
    mut send_packet: SendPacket,
) -> eros::Result<()>
where
    Frames: futures_core::Stream<Item = eros::Result<Rc<WindowsFramePipelineFrame>>> + Unpin,
    Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
    SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
    SendFuture: Future<Output = eros::Result<()>>,
{
    if parameters.codec != VideoCodec::H264 {
        eros::bail!("Windows H.264 encoder cannot encode {:?}", parameters.codec);
    }
    let frame_rate = parameters.frame_rate;
    let Some(first_frame) = frames.next().await else {
        return Ok(());
    };
    let first_frame = first_frame?;
    let source_fixed_rate_paced =
        parameters.frame_rate_mode == VideoFrameRateMode::Fixed && first_frame.fixed_rate_paced;
    debug!(
        event = "windows_video_encoder_starting",
        screen_id = first_frame.screen_id.0,
        width = first_frame.size.width,
        height = first_frame.size.height,
        requested_frame_rate_numerator = frame_rate.numerator(),
        requested_frame_rate_denominator = frame_rate.denominator(),
        source_frame_rate_numerator = first_frame.source_frame_rate.numerator(),
        source_frame_rate_denominator = first_frame.source_frame_rate.denominator(),
        output_frame_rate_numerator = first_frame.frame_rate.numerator(),
        output_frame_rate_denominator = first_frame.frame_rate.denominator(),
        frame_rate_mode = ?parameters.frame_rate_mode,
        bitrate_bps = parameters.bitrate.bits_per_second(),
        max_packet_size,
        "Starting Windows H.264 encoder"
    );
    let mut encoder = MfH264Encoder::new(
        &first_frame,
        frame_rate,
        parameters.frame_rate_mode,
        parameters.bitrate,
        max_packet_size,
    )
    .with_context(|| "Failed to initialize Windows Media Foundation H.264 encoder")?;
    info!(
        event = "windows_video_encoder_scheduling_selected",
        frame_rate_mode = ?parameters.frame_rate_mode,
        input_handling = match (parameters.frame_rate_mode, source_fixed_rate_paced) {
            (VideoFrameRateMode::Dynamic, _) => "encode-on-content-update",
            (VideoFrameRateMode::Fixed, true) => "encode-source-paced-capture",
            (VideoFrameRateMode::Fixed, false) => "retain-latest-and-encode-on-fixed-clock",
        },
        media_foundation_events = if encoder.transform_event_generator.is_some() {
            "async-callback"
        } else {
            "synchronous-process-output"
        },
        "Selected Windows video encoder scheduling"
    );
    encoder.request_key_frame();
    encoder.encode_frame(&first_frame, &mut send_packet).await?;
    drop(first_frame);

    let mut commands_open = true;
    let mut fixed_clock = (parameters.frame_rate_mode == VideoFrameRateMode::Fixed
        && !source_fixed_rate_paced)
        .then(|| FixedFrameClock::new(frame_rate));
    loop {
        enum Event<Frame> {
            Frame(Option<eros::Result<Rc<Frame>>>),
            Command(Option<VideoEncoderCommand>),
            FixedFrameDue,
        }

        let next_frame = frames.next();
        let next_command = async {
            if commands_open {
                commands.next().await
            } else {
                std::future::pending().await
            }
        };
        futures_util::pin_mut!(next_frame, next_command);
        let input = select(next_command, next_frame);
        futures_util::pin_mut!(input);
        let event = match fixed_clock.as_ref() {
            None => match input.await {
                Either::Left((command, _)) => Event::Command(command),
                Either::Right((frame, _)) => Event::Frame(frame),
            },
            Some(clock) => {
                let tick = compio::time::sleep(clock.delay());
                futures_util::pin_mut!(tick);
                match select(tick, input).await {
                    Either::Left(((), _)) => Event::FixedFrameDue,
                    Either::Right((Either::Left((command, _)), _)) => Event::Command(command),
                    Either::Right((Either::Right((frame, _)), _)) => Event::Frame(frame),
                }
            }
        };

        match event {
            Event::Frame(Some(frame)) => {
                let frame = frame?;
                if !encoder.uses_capture_device(&frame)? {
                    let sequence = encoder.sequence;
                    let timeline = encoder.timeline.clone();
                    let next_frame_id = encoder.next_frame_id;
                    let mut replacement = MfH264Encoder::new(
                        &frame,
                        frame_rate,
                        parameters.frame_rate_mode,
                        parameters.bitrate,
                        max_packet_size,
                    )
                    .with_context(
                        || "Failed to rebuild the Windows Media Foundation H.264 encoder",
                    )?;
                    replacement.sequence = sequence;
                    replacement.timeline = timeline;
                    replacement.next_frame_id = next_frame_id;
                    replacement.request_key_frame();
                    encoder = replacement;
                    info!(
                        event = "windows_h264_encoder_reinitialized",
                        screen_id = frame.screen_id.0,
                        reason = "capture-d3d11-device-changed",
                        "Reinitialized Windows H.264 encoder after Desktop Duplication recovery"
                    );
                }
                if fixed_clock.is_some() {
                    encoder.update_latest_frame(&frame)?;
                } else {
                    encoder.encode_frame(&frame, &mut send_packet).await?;
                }
            }
            Event::Frame(None) => break,
            Event::Command(Some(VideoEncoderCommand::RequestKeyFrame)) => {
                encoder.request_key_frame();
                encoder
                    .encode_latest_frame(&mut send_packet)
                    .await
                    .with_context(|| "Failed to encode the retained Windows recovery key frame")?;
            }
            Event::Command(Some(VideoEncoderCommand::SetBitrate(bitrate))) => {
                if let Err(error) = encoder.set_bitrate(bitrate) {
                    warn!(
                        event = "windows_h264_encoder_bitrate_update_failed",
                        requested_bitrate_bps = bitrate.bits_per_second(),
                        error = ?error,
                        "Windows H.264 encoder rejected adaptive bitrate update"
                    );
                }
            }
            Event::Command(None) => commands_open = false,
            Event::FixedFrameDue => {
                encoder.encode_latest_frame(&mut send_packet).await?;
                fixed_clock
                    .as_mut()
                    .expect("Fixed frame-rate tick must have a clock")
                    .advance();
            }
        }
    }

    encoder.drain(&mut send_packet).await
}

struct MfH264Encoder {
    transform: IMFTransform,
    transform_event_generator: Option<IMFMediaEventGenerator>,
    transform_event_callback: Option<IMFAsyncCallback>,
    transform_event_results: Option<flume::Receiver<IMFAsyncResult>>,
    _dxgi_manager: IMFDXGIDeviceManager,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    d3d: ID3D11Device,
    output_stream_info_flags: u32,
    output_stream_buffer_size: u32,
    input_stream_id: u32,
    output_stream_id: u32,
    converter: Option<BgraToNv12Converter>,
    frame_size: PixelSize,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
    profile: H264Profile,
    frame_duration_hns: i64,
    timeline: EncoderTimeline,
    sequence: u16,
    max_packet_size: usize,
    force_key_frame: bool,
    logged_output_format: bool,
    async_accepts_input: bool,
    latest_input: Option<Rc<WindowsFramePipelineFrame>>,
    latest_capture_pending: bool,
    latest_probe: Option<HostVideoFrameProbe>,
    pending_inputs: HashMap<i64, PendingInput>,
    next_frame_id: u64,
    probe_reporter: Option<HostVideoProbeReporter>,
}

struct EncodedOutput {
    sample: IMFSample,
    sample_time_hns: i64,
    probe: Option<HostVideoFrameProbe>,
}

struct PacketizedOutput {
    packets: Vec<WindowsVideoPacket>,
    rtp_packets: u64,
    rtp_bytes: u64,
    output_size: usize,
    annex_b: bool,
    first_bytes: [u8; 16],
    first_bytes_len: usize,
}

struct PendingInput {
    frame_id: u64,
    sample_time_hns: i64,
    texture: Nv12InputTexture,
    probe: Option<HostVideoFrameProbe>,
}

#[derive(Clone)]
struct EncoderTimeline {
    mode: VideoFrameRateMode,
    frame_duration_hns: i64,
    capture_epoch: Option<Instant>,
    next_fixed_sample_time_hns: i64,
    last_sample_time_hns: Option<i64>,
}

impl EncoderTimeline {
    fn new(mode: VideoFrameRateMode, frame_rate: FrameRate) -> Self {
        Self {
            mode,
            frame_duration_hns: frame_duration_hns(frame_rate),
            capture_epoch: None,
            next_fixed_sample_time_hns: 0,
            last_sample_time_hns: None,
        }
    }

    fn next_sample_time_hns(&mut self, captured_at: Instant) -> i64 {
        let requested = match self.mode {
            VideoFrameRateMode::Fixed => {
                let current = self.next_fixed_sample_time_hns;
                self.next_fixed_sample_time_hns = self
                    .next_fixed_sample_time_hns
                    .saturating_add(self.frame_duration_hns);
                current
            }
            VideoFrameRateMode::Dynamic => {
                let epoch = *self.capture_epoch.get_or_insert(captured_at);
                duration_hns(captured_at.saturating_duration_since(epoch))
            }
        };
        let sample_time = match self.last_sample_time_hns {
            Some(previous) if requested <= previous => previous.saturating_add(1),
            _ => requested,
        };
        self.last_sample_time_hns = Some(sample_time);
        sample_time
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncEncoderEvent {
    NeedInput,
    HaveOutput,
    DrainComplete,
}

#[windows::core::implement(IMFAsyncCallback)]
struct EncoderEventCallback {
    results: flume::Sender<IMFAsyncResult>,
}

impl IMFAsyncCallback_Impl for EncoderEventCallback_Impl {
    fn GetParameters(&self, _flags: *mut u32, _queue: *mut u32) -> windows::core::Result<()> {
        Err(windows::core::Error::from_hresult(E_NOTIMPL))
    }

    fn Invoke(&self, result: windows::core::Ref<IMFAsyncResult>) -> windows::core::Result<()> {
        let result = result
            .cloned()
            .ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;
        self.results
            .send(result)
            .map_err(|_| windows::core::Error::from_hresult(E_FAIL))
    }
}

impl MfH264Encoder {
    fn new(
        first_frame: &WindowsFramePipelineFrame,
        frame_rate: FrameRate,
        frame_rate_mode: VideoFrameRateMode,
        bitrate: VideoBitrate,
        max_packet_size: usize,
    ) -> eros::Result<Self> {
        if max_packet_size <= RTP_FIXED_HEADER_SIZE + 2 {
            eros::bail!("Windows H.264 RTP packet size is too small: {max_packet_size}");
        }
        let frame_size = first_frame.size;
        if frame_size.width == 0 || frame_size.height == 0 {
            eros::bail!("Windows encoder cannot encode an empty frame");
        }

        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .with_context(|| "Failed to start Media Foundation")?;

        let d3d = unsafe { first_frame.texture().GetDevice() }
            .with_context(|| "Failed to get the Windows capture D3D11 device")?;
        let video: ID3D11VideoDevice = d3d
            .cast()
            .with_context(|| "D3D11 device does not expose ID3D11VideoDevice")?;
        let context = unsafe { d3d.GetImmediateContext() }
            .with_context(|| "Failed to get the D3D11 immediate context")?;
        let video_context: ID3D11VideoContext = context
            .cast()
            .with_context(|| "D3D11 immediate context does not expose ID3D11VideoContext")?;
        let dxgi_manager = create_dxgi_device_manager(&d3d)?;
        let adapter_vendor_id = d3d_adapter_vendor_id(&d3d)?;
        let (transform, profile) = activate_h264_encoder(
            &dxgi_manager,
            frame_size,
            frame_rate,
            bitrate,
            adapter_vendor_id,
        )?;
        let output_stream_info = unsafe { transform.GetOutputStreamInfo(0) }
            .with_context(|| "Failed to query H.264 encoder output stream info")?;
        let transform_event_generator = transform.cast::<IMFMediaEventGenerator>().ok();
        let (transform_event_callback, transform_event_results) =
            if let Some(events) = &transform_event_generator {
                let (results, receiver) = flume::unbounded();
                let callback: IMFAsyncCallback = EncoderEventCallback { results }.into();
                unsafe { events.BeginGetEvent(&callback, None::<&windows::core::IUnknown>) }
                    .with_context(|| "Failed to subscribe to Windows H.264 encoder events")?;
                (Some(callback), Some(receiver))
            } else {
                (None, None)
            };
        let async_accepts_input = transform_event_generator.is_none();
        let probe_reporter = first_frame
            .probe
            .as_ref()
            .map(|probe| HostVideoProbeReporter::new(probe.report_interval()));

        Ok(Self {
            transform,
            transform_event_generator,
            transform_event_callback,
            transform_event_results,
            _dxgi_manager: dxgi_manager,
            video,
            video_context,
            d3d,
            output_stream_info_flags: output_stream_info.dwFlags,
            output_stream_buffer_size: output_stream_info.cbSize,
            input_stream_id: 0,
            output_stream_id: 0,
            converter: None,
            frame_size,
            frame_rate,
            bitrate,
            profile,
            frame_duration_hns: frame_duration_hns(frame_rate),
            timeline: EncoderTimeline::new(frame_rate_mode, frame_rate),
            sequence: 0,
            max_packet_size,
            force_key_frame: false,
            logged_output_format: false,
            async_accepts_input,
            latest_input: None,
            latest_capture_pending: false,
            latest_probe: None,
            pending_inputs: HashMap::new(),
            next_frame_id: 0,
            probe_reporter,
        })
    }

    fn request_key_frame(&mut self) {
        self.force_key_frame = true;
    }

    fn uses_capture_device(&self, frame: &WindowsFramePipelineFrame) -> eros::Result<bool> {
        let device = unsafe { frame.texture().GetDevice() }
            .with_context(|| "Failed to query the recovered Windows capture D3D11 device")?;
        Ok(self.d3d.as_raw() == device.as_raw())
    }

    fn set_bitrate(&mut self, requested: VideoBitrate) -> eros::Result<()> {
        if requested == self.bitrate {
            return Ok(());
        }
        let codec_api: ICodecAPI = self
            .transform
            .cast()
            .with_context(|| "Windows H.264 encoder does not expose ICodecAPI")?;
        unsafe {
            codec_api.SetValue(
                &CODECAPI_AVEncCommonMeanBitRate,
                &VARIANT::from(requested.bits_per_second()),
            )
        }
        .with_context(|| "Failed to update Windows H.264 mean bitrate")?;
        let (_, cpb_frames, cpb_bytes) =
            configure_cpb_buffer(&codec_api, requested.bits_per_second(), self.frame_rate);
        let effective_bps = unsafe { codec_api.GetValue(&CODECAPI_AVEncCommonMeanBitRate) }
            .ok()
            .and_then(|value| variant_numeric_value(&value))
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(requested.bits_per_second());
        self.bitrate = VideoBitrate::new(effective_bps)?;
        info!(
            event = "windows_h264_encoder_bitrate_updated",
            requested_bitrate_bps = requested.bits_per_second(),
            effective_bitrate_bps = effective_bps,
            cpb_frames,
            cpb_bytes,
            "Updated Windows H.264 encoder bitrate"
        );
        Ok(())
    }

    async fn encode_frame<SendPacket, SendFuture>(
        &mut self,
        frame: &Rc<WindowsFramePipelineFrame>,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        self.update_latest_frame(frame)?;
        self.encode_latest_frame(send_packet).await
    }

    fn update_latest_frame(&mut self, frame: &Rc<WindowsFramePipelineFrame>) -> eros::Result<()> {
        if frame.size != self.frame_size {
            eros::bail!(
                "Windows encoder does not support dynamic frame size changes yet: got {}x{}, expected {}x{}",
                frame.size.width,
                frame.size.height,
                self.frame_size.width,
                self.frame_size.height
            );
        }

        let mut probe = frame
            .probe
            .clone()
            .filter(|_| self.probe_reporter.is_some());
        if let Some(probe) = &mut probe {
            probe.mark_encoder_received();
        }

        self.latest_input = Some(frame.clone());
        self.latest_capture_pending = true;
        self.latest_probe = probe;
        Ok(())
    }

    async fn encode_latest_frame<SendPacket, SendFuture>(
        &mut self,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        let frame = self
            .latest_input
            .as_ref()
            .cloned()
            .with_context(|| "Windows encoder has no retained frame to encode")?;
        let mut probe = self.latest_probe.take();
        let captured_at = if self.latest_capture_pending {
            frame.captured_at
        } else {
            Instant::now()
        };
        let texture = loop {
            if let Some(probe) = &mut probe {
                probe.mark_vpp_started();
            }
            match self.try_convert_to_nv12(frame.texture())? {
                Some(texture) => {
                    if let Some(probe) = &mut probe {
                        probe.mark_vpp_completed();
                    }
                    break texture;
                }
                None => {
                    self.wait_for_input_texture(send_packet).await?;
                }
            }
        };
        self.encode_input(texture, probe, captured_at, send_packet)
            .await?;
        self.latest_capture_pending = false;
        Ok(())
    }

    async fn encode_input<SendPacket, SendFuture>(
        &mut self,
        texture: Nv12InputTexture,
        mut probe: Option<HostVideoFrameProbe>,
        captured_at: Instant,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        if self.transform_event_generator.is_some() {
            while !self.async_accepts_input {
                match self.handle_async_encoder_event().await? {
                    AsyncEncoderEvent::HaveOutput => {
                        self.receive_packets(send_packet).await?;
                    }
                    AsyncEncoderEvent::NeedInput => {}
                    AsyncEncoderEvent::DrainComplete => {
                        eros::bail!(
                            "Windows H.264 encoder drained while waiting to accept an input"
                        );
                    }
                }
            }
        }

        let sample_time_hns = self.timeline.next_sample_time_hns(captured_at);
        let frame_id = self.next_frame_id;
        self.next_frame_id = self.next_frame_id.saturating_add(1);
        let sample = create_input_sample(texture.texture())?;
        unsafe {
            sample.SetSampleTime(sample_time_hns)?;
            sample.SetSampleDuration(self.frame_duration_hns)?;
        }
        if self.force_key_frame {
            if let Err(error) = self.force_key_frame() {
                warn!(
                    event = "windows_key_frame_request_failed",
                    error = ?error,
                    "Windows H.264 encoder rejected the on-demand key-frame request"
                );
            }
            self.force_key_frame = false;
        }
        unsafe {
            self.transform
                .ProcessInput(self.input_stream_id, &sample, 0)
        }
        .with_context(|| "Failed to submit a D3D11 sample to the H.264 encoder")?;
        if let Some(probe) = &mut probe {
            probe.mark_encoder_submitted();
        }
        let texture_slot_id = texture.slot_id();
        debug_assert!(
            !self
                .pending_inputs
                .values()
                .any(|pending| pending.texture.slot_id() == texture_slot_id),
            "one NV12 texture slot cannot belong to two pending encoder inputs"
        );
        if self
            .pending_inputs
            .insert(
                sample_time_hns,
                PendingInput {
                    frame_id,
                    sample_time_hns,
                    texture,
                    probe,
                },
            )
            .is_some()
        {
            eros::bail!(
                "Windows H.264 encoder reused input PTS {sample_time_hns} for frame {frame_id}"
            );
        }
        let mut produced_output = false;
        if self.transform_event_generator.is_some() {
            self.async_accepts_input = false;
            loop {
                match self.handle_async_encoder_event().await? {
                    AsyncEncoderEvent::NeedInput => break,
                    AsyncEncoderEvent::HaveOutput => {
                        produced_output = true;
                        self.receive_packets(send_packet).await?;
                    }
                    AsyncEncoderEvent::DrainComplete => break,
                }
            }
        }
        if self.transform_event_generator.is_none() {
            self.receive_packets(send_packet).await?;
        } else if !produced_output {
            debug!(
                event = "windows_h264_encoder_no_output",
                "Windows H.264 encoder accepted input without producing output"
            );
        }
        Ok(())
    }

    async fn drain<SendPacket, SendFuture>(
        &mut self,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0)?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0)?;
        }
        if self.transform_event_generator.is_some() {
            loop {
                match self.next_async_encoder_event().await? {
                    AsyncEncoderEvent::HaveOutput => self.receive_packets(send_packet).await?,
                    AsyncEncoderEvent::NeedInput => self.async_accepts_input = true,
                    AsyncEncoderEvent::DrainComplete => {
                        self.discard_pending_inputs("drain-complete");
                        return Ok(());
                    }
                }
            }
        }
        self.receive_packets(send_packet).await?;
        self.discard_pending_inputs("synchronous-drain-complete");
        Ok(())
    }

    async fn wait_for_input_texture<SendPacket, SendFuture>(
        &mut self,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        if self.transform_event_generator.is_some() {
            loop {
                match self.handle_async_encoder_event().await? {
                    AsyncEncoderEvent::HaveOutput => {
                        self.receive_packets(send_packet).await?;
                        return Ok(());
                    }
                    AsyncEncoderEvent::NeedInput => {}
                    AsyncEncoderEvent::DrainComplete => {
                        eros::bail!(
                            "Windows H.264 encoder drained while waiting for an NV12 input texture"
                        );
                    }
                }
            }
        }

        let pending_before = self.pending_inputs.len();
        self.receive_packets(send_packet).await?;
        if self.pending_inputs.len() >= pending_before {
            eros::bail!(
                "Windows H.264 encoder exhausted its NV12 texture pool without producing output"
            );
        }
        Ok(())
    }

    async fn receive_packets<SendPacket, SendFuture>(
        &mut self,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(Vec<WindowsVideoPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        loop {
            let Some(mut output) = self.process_output()? else {
                return Ok(());
            };
            let timestamp = rtp_timestamp_from_hns(output.sample_time_hns);
            let packetized = with_sample_bytes(&output.sample, |access_units| {
                let mut rtp_packets = 0u64;
                let mut rtp_bytes = 0u64;
                let mut packets = Vec::new();
                for access_unit in split_annex_b_access_units(access_units) {
                    let nals = split_h264_nals(access_unit);
                    for (index, nal) in nals.iter().enumerate() {
                        let marker = index + 1 == nals.len();
                        for packet in packetize_h264_nal(
                            nal,
                            timestamp,
                            &mut self.sequence,
                            self.max_packet_size,
                            marker,
                        )? {
                            rtp_packets = rtp_packets.saturating_add(1);
                            rtp_bytes = rtp_bytes
                                .saturating_add(u64::try_from(packet.len()).unwrap_or(u64::MAX));
                            packets.push(WindowsVideoPacket(packet));
                        }
                    }
                }
                let first_bytes_len = access_units.len().min(16);
                let mut first_bytes = [0; 16];
                first_bytes[..first_bytes_len].copy_from_slice(&access_units[..first_bytes_len]);
                Ok(PacketizedOutput {
                    packets,
                    rtp_packets,
                    rtp_bytes,
                    output_size: access_units.len(),
                    annex_b: find_start_code(access_units, 0).is_some(),
                    first_bytes,
                    first_bytes_len,
                })
            })?;
            if !self.logged_output_format {
                debug!(
                    event = "windows_h264_output_format",
                    output_size = packetized.output_size,
                    annex_b = packetized.annex_b,
                    first_bytes = ?&packetized.first_bytes[..packetized.first_bytes_len],
                    "Received first Windows H.264 encoder output"
                );
                self.logged_output_format = true;
            }
            if !packetized.packets.is_empty() {
                send_packet(packetized.packets).await?;
            }
            if let (Some(reporter), Some(probe)) = (&mut self.probe_reporter, output.probe.take()) {
                reporter.record_frame(probe, packetized.rtp_packets, packetized.rtp_bytes);
            }
            if self.transform_event_generator.is_some() {
                return Ok(());
            }
        }
    }

    async fn handle_async_encoder_event(&mut self) -> eros::Result<AsyncEncoderEvent> {
        let event = self.next_async_encoder_event().await?;
        match event {
            AsyncEncoderEvent::NeedInput => {
                self.async_accepts_input = true;
            }
            AsyncEncoderEvent::HaveOutput => {
                self.async_accepts_input = false;
            }
            AsyncEncoderEvent::DrainComplete => {
                self.async_accepts_input = false;
            }
        }
        Ok(event)
    }

    async fn next_async_encoder_event(&self) -> eros::Result<AsyncEncoderEvent> {
        let Some(events) = &self.transform_event_generator else {
            return Ok(AsyncEncoderEvent::HaveOutput);
        };
        let results = self
            .transform_event_results
            .as_ref()
            .with_context(|| "Windows H.264 encoder event callback has no result channel")?;
        let callback = self
            .transform_event_callback
            .as_ref()
            .with_context(|| "Windows H.264 encoder event callback is unavailable")?;

        loop {
            let result = results
                .recv_async()
                .await
                .with_context(|| "Windows H.264 encoder event callback disconnected")?;
            let event = unsafe { events.EndGetEvent(&result) }
                .with_context(|| "Failed to complete Windows H.264 encoder event")?;
            let event_type = unsafe { event.GetType() }
                .with_context(|| "Failed to read H.264 encoder event type")?;
            let status = unsafe { event.GetStatus() }
                .with_context(|| "Failed to read H.264 encoder event status")?;
            debug!(
                event = "windows_h264_encoder_event",
                event_type,
                status = ?status,
                "Received Windows H.264 encoder event"
            );
            if status.is_err() {
                Err(windows::core::Error::from_hresult(status))
                    .with_context(|| "Windows H.264 encoder event reported failure")?;
            }

            let encoder_event = if event_type == METransformHaveOutput.0 as u32 {
                Some(AsyncEncoderEvent::HaveOutput)
            } else if event_type == METransformNeedInput.0 as u32 {
                Some(AsyncEncoderEvent::NeedInput)
            } else if event_type == METransformDrainComplete.0 as u32 {
                Some(AsyncEncoderEvent::DrainComplete)
            } else {
                None
            };
            if encoder_event != Some(AsyncEncoderEvent::DrainComplete) {
                unsafe { events.BeginGetEvent(callback, None::<&windows::core::IUnknown>) }
                    .with_context(|| "Failed to rearm Windows H.264 encoder event callback")?;
            }
            if let Some(encoder_event) = encoder_event {
                return Ok(encoder_event);
            }
        }
    }

    fn process_output(&mut self) -> eros::Result<Option<EncodedOutput>> {
        for stream_change_count in 0..MAX_ENCODER_STREAM_CHANGES_PER_OUTPUT {
            let mut owned_sample = None;
            if self.output_stream_info_flags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 == 0 {
                let sample = unsafe { MFCreateSample() }
                    .with_context(|| "Failed to create an encoder output sample")?;
                let buffer_size = self
                    .output_stream_buffer_size
                    .max(MAX_ENCODER_OUTPUT_SAMPLE_SIZE);
                let buffer = unsafe { MFCreateMemoryBuffer(buffer_size) }
                    .with_context(|| "Failed to create an encoder output buffer")?;
                unsafe { sample.AddBuffer(&buffer) }
                    .with_context(|| "Failed to attach an output buffer to the encoder sample")?;
                owned_sample = Some(sample);
            }

            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: self.output_stream_id,
                pSample: std::mem::ManuallyDrop::new(owned_sample),
                dwStatus: 0,
                pEvents: std::mem::ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result = unsafe {
                self.transform
                    .ProcessOutput(0, std::slice::from_mut(&mut output), &mut status)
            };
            let sample = unsafe { std::mem::ManuallyDrop::take(&mut output.pSample) };
            let _events = unsafe { std::mem::ManuallyDrop::take(&mut output.pEvents) };
            match result {
                Ok(()) => {
                    let Some(sample) = sample else {
                        return Ok(None);
                    };
                    let output_sample_time_hns = unsafe { sample.GetSampleTime() }
                        .with_context(|| "Windows H.264 encoder output has no sample PTS")?;
                    let PendingInput {
                        frame_id,
                        sample_time_hns,
                        texture,
                        mut probe,
                    } = self
                        .pending_inputs
                        .remove(&output_sample_time_hns)
                        .with_context(|| {
                            format!(
                                "Windows H.264 encoder output PTS {output_sample_time_hns} has no matching pending input"
                            )
                        })?;
                    let texture_slot_id = texture.slot_id();
                    drop(texture);
                    if let Some(probe) = &mut probe {
                        probe.mark_encoder_completed();
                    }
                    trace!(
                        event = "windows_h264_encoder_input_completed",
                        frame_id,
                        sample_time_hns,
                        output_sample_time_hns,
                        texture_slot_id,
                        pending_inputs = self.pending_inputs.len(),
                        "Released an NV12 encoder input texture"
                    );
                    return Ok(Some(EncodedOutput {
                        sample,
                        sample_time_hns,
                        probe,
                    }));
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    warn!(
                        event = "windows_h264_encoder_stream_change",
                        output_status = output.dwStatus,
                        process_status = status,
                        attempt = stream_change_count + 1,
                        pending_inputs = self.pending_inputs.len(),
                        pending_policy = "retain-and-match-by-pts",
                        "Windows H.264 encoder requested output type renegotiation"
                    );
                    self.renegotiate_output_type()?;
                    if self.transform_event_generator.is_some() {
                        // An asynchronous MFT permits exactly one ProcessOutput
                        // call per METransformHaveOutput event. The stream-change
                        // result consumed this event, so wait for the next event
                        // instead of retrying immediately (which returns
                        // E_UNEXPECTED on Intel hardware encoders).
                        return Ok(None);
                    }
                }
                Err(error) => {
                    Err(error).with_context(|| "Failed to receive H.264 encoder output")?;
                    unreachable!()
                }
            }
        }

        eros::bail!(
            "Windows H.264 encoder repeatedly requested output type renegotiation without producing output"
        )
    }

    fn renegotiate_output_type(&mut self) -> eros::Result<()> {
        let mut available_types = Vec::new();
        for index in 0..64 {
            let media_type = match unsafe {
                self.transform
                    .GetOutputAvailableType(self.output_stream_id, index)
            } {
                Ok(media_type) => media_type,
                Err(error) if error.code() == MF_E_NO_MORE_TYPES => break,
                Err(error) => {
                    Err(error).with_context(
                        || "Failed to enumerate H.264 encoder output types after stream change",
                    )?;
                    unreachable!()
                }
            };
            if unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.ok() != Some(MFVideoFormat_H264) {
                continue;
            }
            let profile = unsafe { media_type.GetUINT32(&MF_MT_MPEG2_PROFILE) }
                .ok()
                .and_then(H264Profile::from_media_foundation_value);
            available_types.push((index, media_type, profile));
        }
        available_types.sort_by_key(|(_, _, profile)| match profile {
            Some(H264Profile::High) => 0,
            Some(H264Profile::Main) => 1,
            Some(H264Profile::Baseline) => 2,
            None => 3,
        });
        let h264_type_count = available_types.len();
        for (index, media_type, advertised_profile) in available_types {
            match unsafe {
                self.transform
                    .SetOutputType(self.output_stream_id, &media_type, 0)
            } {
                Ok(()) => {
                    self.profile = unsafe { media_type.GetUINT32(&MF_MT_MPEG2_PROFILE) }
                        .ok()
                        .and_then(H264Profile::from_media_foundation_value)
                        .or(advertised_profile)
                        .unwrap_or(self.profile);
                    self.refresh_output_stream_info()?;
                    info!(
                        event = "windows_h264_encoder_output_type_renegotiated",
                        available_type_index = index,
                        h264_profile = self.profile.name(),
                        output_buffer_size = self.output_stream_buffer_size,
                        output_provides_samples = self.output_stream_info_flags
                            & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32
                            != 0,
                        "Renegotiated Windows H.264 encoder output type"
                    );
                    return Ok(());
                }
                Err(error) => {
                    debug!(
                        event = "windows_h264_encoder_output_type_rejected",
                        available_type_index = index,
                        error = ?error,
                        "Windows H.264 encoder rejected an advertised output type"
                    );
                }
            }
        }

        // Some hardware encoders do not expose a complete available-type list
        // after invalidating their output type. Reapplying the requested type is
        // a useful compatibility fallback and still performs SetOutputType as
        // required to resume ProcessOutput.
        let requested_type =
            create_h264_output_type(self.frame_size, self.frame_rate, self.bitrate, self.profile)?;
        unsafe {
            self.transform
                .SetOutputType(self.output_stream_id, &requested_type, 0)
        }
        .with_context(|| {
            format!(
                "Failed to renegotiate H.264 encoder output type ({h264_type_count} H.264 types advertised)"
            )
        })?;
        self.refresh_output_stream_info()?;
        info!(
            event = "windows_h264_encoder_output_type_renegotiated",
            available_type_index = -1,
            h264_profile = self.profile.name(),
            output_buffer_size = self.output_stream_buffer_size,
            output_provides_samples =
                self.output_stream_info_flags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0,
            "Renegotiated Windows H.264 encoder with the requested output type"
        );
        Ok(())
    }

    fn refresh_output_stream_info(&mut self) -> eros::Result<()> {
        let info = unsafe { self.transform.GetOutputStreamInfo(self.output_stream_id) }
            .with_context(|| "Failed to refresh H.264 encoder output stream info")?;
        self.output_stream_info_flags = info.dwFlags;
        self.output_stream_buffer_size = info.cbSize;
        Ok(())
    }

    fn discard_pending_inputs(&mut self, reason: &'static str) {
        if self.pending_inputs.is_empty() {
            return;
        }
        let mut frame_ids = self
            .pending_inputs
            .values()
            .map(|pending| pending.frame_id)
            .collect::<Vec<_>>();
        frame_ids.sort_unstable();
        warn!(
            event = "windows_h264_encoder_pending_inputs_discarded",
            reason,
            pending_inputs = self.pending_inputs.len(),
            frame_ids = ?frame_ids,
            "Released Windows H.264 inputs that produced no output"
        );
        self.pending_inputs.clear();
    }

    fn force_key_frame(&self) -> eros::Result<()> {
        let codec_api: ICodecAPI = self
            .transform
            .cast()
            .with_context(|| "Windows H.264 encoder does not expose ICodecAPI")?;
        // CODECAPI_AVEncVideoForceKeyFrame is an ULONG (VT_UI4), not a VARIANT_BOOL.
        // Some hardware encoders accept the wrong variant type without acting on it.
        let value = VARIANT::from(1_u32);
        Ok(
            unsafe { codec_api.SetValue(&CODECAPI_AVEncVideoForceKeyFrame, &value) }
                .with_context(|| "Failed to request a Windows H.264 key frame")?,
        )
    }

    fn try_convert_to_nv12(
        &mut self,
        texture: &ID3D11Texture2D,
    ) -> eros::Result<Option<Nv12InputTexture>> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM && desc.Format != DXGI_FORMAT_NV12 {
            eros::bail!(
                "Windows capture frame has unsupported D3D11 format {:?}",
                desc.Format
            );
        }
        let source_size = PixelSize {
            width: desc.Width,
            height: desc.Height,
        };
        let converter = match &mut self.converter {
            Some(converter) if converter.matches(source_size, self.frame_size) => converter,
            _ => {
                self.converter = Some(BgraToNv12Converter::new(
                    &self.d3d,
                    &self.video,
                    &self.video_context,
                    source_size,
                    self.frame_size,
                    self.frame_duration_hns,
                )?);
                self.converter
                    .as_mut()
                    .with_context(|| "BGRA to NV12 converter disappeared")?
            }
        };
        converter.convert(texture)
    }
}

impl Drop for MfH264Encoder {
    fn drop(&mut self) {
        self.discard_pending_inputs("encoder-drop");
        if let Some(reporter) = &mut self.probe_reporter {
            reporter.finish();
        }
        let _ = unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)
        };
    }
}

struct BgraToNv12Converter {
    source_size: PixelSize,
    output_size: PixelSize,
    texture_pool: Nv12TexturePool,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessor,
    context: ID3D11DeviceContext,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
}

struct Nv12TexturePool {
    recycle: flume::Sender<Nv12TextureSlot>,
    available: flume::Receiver<Nv12TextureSlot>,
}

struct Nv12TextureSlot {
    id: usize,
    texture: ID3D11Texture2D,
    output_view: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessorOutputView,
}

struct Nv12InputTexture {
    slot: Option<Nv12TextureSlot>,
    recycle: flume::Sender<Nv12TextureSlot>,
}

impl Nv12InputTexture {
    fn slot_id(&self) -> usize {
        self.slot
            .as_ref()
            .expect("an NV12 input lease always owns its texture slot")
            .id
    }

    fn texture(&self) -> &ID3D11Texture2D {
        &self
            .slot
            .as_ref()
            .expect("an NV12 input lease always owns its texture slot")
            .texture
    }

    fn output_view(&self) -> &windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessorOutputView {
        &self
            .slot
            .as_ref()
            .expect("an NV12 input lease always owns its texture slot")
            .output_view
    }
}

impl Drop for Nv12InputTexture {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let _ = self.recycle.try_send(slot);
        }
    }
}

impl BgraToNv12Converter {
    fn new(
        d3d: &ID3D11Device,
        video: &ID3D11VideoDevice,
        video_context: &ID3D11VideoContext,
        source_size: PixelSize,
        output_size: PixelSize,
        frame_duration_hns: i64,
    ) -> eros::Result<Self> {
        let rate = frame_rate_rational_from_duration(frame_duration_hns);
        let content_desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: source_size.width,
            InputHeight: source_size.height,
            OutputFrameRate: rate,
            OutputWidth: output_size.width,
            OutputHeight: output_size.height,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        let enumerator = unsafe { video.CreateVideoProcessorEnumerator(&content_desc) }
            .with_context(|| "Failed to create D3D11 video processor enumerator")?;
        let processor = unsafe { video.CreateVideoProcessor(&enumerator, 0) }
            .with_context(|| "Failed to create D3D11 video processor")?;
        let source_rect = video_processor_rect(source_size.width, source_size.height)?;
        let output_rect = video_processor_rect(output_size.width, output_size.height)?;
        unsafe {
            video_context.VideoProcessorSetStreamSourceRect(
                &processor,
                0,
                true,
                Some(&source_rect),
            );
            video_context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&output_rect));
            video_context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&output_rect));
        }
        let output_desc = D3D11_TEXTURE2D_DESC {
            Width: output_size.width,
            Height: output_size.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0
                | D3D11_BIND_SHADER_RESOURCE.0
                | D3D11_BIND_VIDEO_ENCODER.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let output_view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        const NV12_TEXTURE_COUNT: usize = 4;
        let (recycle, available) = flume::bounded(NV12_TEXTURE_COUNT);
        for id in 0..NV12_TEXTURE_COUNT {
            let mut texture = None;
            unsafe { d3d.CreateTexture2D(&output_desc, None, Some(&mut texture)) }
                .with_context(|| "Failed to allocate pooled NV12 encoder texture")?;
            let texture =
                texture.with_context(|| "CreateTexture2D returned no pooled NV12 texture")?;
            let mut output_view = None;
            unsafe {
                video.CreateVideoProcessorOutputView(
                    &texture.cast::<ID3D11Resource>()?,
                    &enumerator,
                    &output_view_desc,
                    Some(&mut output_view),
                )
            }
            .with_context(|| "Failed to create pooled NV12 video processor output view")?;
            let output_view =
                output_view.with_context(|| "D3D11 returned no pooled processor output view")?;
            if recycle
                .try_send(Nv12TextureSlot {
                    id,
                    texture,
                    output_view,
                })
                .is_err()
            {
                eros::bail!("Failed to initialize the NV12 encoder texture pool");
            }
        }
        set_video_processor_color_space(video_context, &processor);
        let context = unsafe { d3d.GetImmediateContext() }
            .with_context(|| "Failed to get the NV12 texture pool D3D11 context")?;
        debug!(
            event = "windows_video_processor_scaler_configured",
            source_width = source_size.width,
            source_height = source_size.height,
            output_width = output_size.width,
            output_height = output_size.height,
            texture_pool_size = NV12_TEXTURE_COUNT,
            "Configured full-frame D3D11 video processor scaling"
        );

        Ok(Self {
            source_size,
            output_size,
            texture_pool: Nv12TexturePool { recycle, available },
            enumerator,
            processor,
            context,
            video: video.clone(),
            video_context: video_context.clone(),
        })
    }

    fn matches(&self, source_size: PixelSize, output_size: PixelSize) -> bool {
        self.source_size == source_size && self.output_size == output_size
    }

    fn convert(&mut self, texture: &ID3D11Texture2D) -> eros::Result<Option<Nv12InputTexture>> {
        let Ok(slot) = self.texture_pool.available.try_recv() else {
            return Ok(None);
        };
        let output = Nv12InputTexture {
            slot: Some(slot),
            recycle: self.texture_pool.recycle.clone(),
        };
        let mut source_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut source_desc) };
        if source_desc.Format == DXGI_FORMAT_NV12 && self.source_size == self.output_size {
            unsafe { self.context.CopyResource(output.texture(), texture) };
            return Ok(Some(output));
        }
        let input_view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut input_view = None;
        unsafe {
            self.video.CreateVideoProcessorInputView(
                &texture.cast::<ID3D11Resource>()?,
                &self.enumerator,
                &input_view_desc,
                Some(&mut input_view),
            )
        }
        .with_context(|| "Failed to create BGRA video processor input view")?;
        let input_view = input_view.with_context(|| "D3D11 returned no processor input view")?;
        let streams = [D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            OutputIndex: 0,
            InputFrameOrField: 0,
            PastFrames: 0,
            FutureFrames: 0,
            ppPastSurfaces: std::ptr::null_mut(),
            pInputSurface: std::mem::ManuallyDrop::new(Some(input_view)),
            ppFutureSurfaces: std::ptr::null_mut(),
            ppPastSurfacesRight: std::ptr::null_mut(),
            pInputSurfaceRight: std::mem::ManuallyDrop::new(None),
            ppFutureSurfacesRight: std::ptr::null_mut(),
        }];
        unsafe {
            self.video_context
                .VideoProcessorBlt(&self.processor, output.output_view(), 0, &streams)
        }
        .with_context(|| "Failed to convert BGRA capture texture to NV12")?;
        Ok(Some(output))
    }
}

fn video_processor_rect(width: u32, height: u32) -> eros::Result<RECT> {
    Ok(RECT {
        left: 0,
        top: 0,
        right: i32::try_from(width).with_context(|| "D3D11 video processor width exceeds i32")?,
        bottom: i32::try_from(height)
            .with_context(|| "D3D11 video processor height exceeds i32")?,
    })
}

fn set_video_processor_color_space(
    video_context: &ID3D11VideoContext,
    processor: &windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessor,
) {
    if let Ok(video_context_1) = video_context.cast::<ID3D11VideoContext1>() {
        let input = captured_rgb_color_space();
        let output = encoded_ycbcr_color_space();
        unsafe {
            video_context_1.VideoProcessorSetStreamColorSpace1(processor, 0, input);
            video_context_1.VideoProcessorSetOutputColorSpace1(processor, output);
        }
        debug!(
            event = "windows_video_processor_color_space",
            input = ?input,
            output = ?output,
            "Configured D3D11 video processor BT.709 color space"
        );
        return;
    }

    let input = legacy_captured_rgb_color_space();
    let output = legacy_encoded_ycbcr_color_space();
    unsafe {
        video_context.VideoProcessorSetStreamColorSpace(processor, 0, &input);
        video_context.VideoProcessorSetOutputColorSpace(processor, &output);
    }
    debug!(
        event = "windows_video_processor_color_space",
        input = input._bitfield,
        output = output._bitfield,
        "Configured D3D11 video processor BT.709 color space"
    );
}

fn create_dxgi_device_manager(d3d: &ID3D11Device) -> eros::Result<IMFDXGIDeviceManager> {
    let mut reset_token = 0;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager) }
        .with_context(|| "Failed to create Media Foundation DXGI device manager")?;
    let manager = manager.with_context(|| "MFCreateDXGIDeviceManager returned no manager")?;
    unsafe { manager.ResetDevice(d3d, reset_token) }
        .with_context(|| "Failed to attach the D3D11 device to the DXGI device manager")?;
    Ok(manager)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum H264Profile {
    High,
    Main,
    Baseline,
}

impl H264Profile {
    const PREFERRED: [Self; 3] = [Self::High, Self::Main, Self::Baseline];

    fn media_foundation_value(self) -> u32 {
        match self {
            Self::High => eAVEncH264VProfile_High.0 as u32,
            Self::Main => eAVEncH264VProfile_Main.0 as u32,
            Self::Baseline => eAVEncH264VProfile_Base.0 as u32,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Main => "main",
            Self::Baseline => "baseline",
        }
    }

    fn from_media_foundation_value(value: u32) -> Option<Self> {
        Self::PREFERRED
            .into_iter()
            .find(|profile| profile.media_foundation_value() == value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowsEncoderBackend {
    Nvenc,
    QuickSync,
    Amf,
    MediaFoundation,
}

impl WindowsEncoderBackend {
    fn name(self) -> &'static str {
        match self {
            Self::Nvenc => "nvenc",
            Self::QuickSync => "quick-sync",
            Self::Amf => "amf",
            Self::MediaFoundation => "media-foundation",
        }
    }

    fn from_activation(name: Option<&str>, vendor: Option<&str>) -> Self {
        let identity = format!(
            "{} {}",
            name.unwrap_or_default().to_ascii_lowercase(),
            vendor.unwrap_or_default().to_ascii_lowercase()
        );
        if identity.contains("nvidia") || identity.contains("nvenc") {
            Self::Nvenc
        } else if identity.contains("intel") || identity.contains("quick sync") {
            Self::QuickSync
        } else if identity.contains("amd")
            || identity.contains("advanced micro devices")
            || identity.contains("amf")
        {
            Self::Amf
        } else {
            Self::MediaFoundation
        }
    }

    fn for_adapter_vendor(vendor_id: u32) -> Self {
        match vendor_id {
            0x10de => Self::Nvenc,
            0x8086 => Self::QuickSync,
            0x1002 | 0x1022 => Self::Amf,
            _ => Self::MediaFoundation,
        }
    }
}

struct WindowsEncoderCandidate {
    activate: IMFActivate,
    backend: WindowsEncoderBackend,
    name: Option<String>,
    hardware_url: Option<String>,
    vendor: Option<String>,
    transform_clsid: Option<windows::core::GUID>,
}

fn d3d_adapter_vendor_id(d3d: &ID3D11Device) -> eros::Result<u32> {
    let dxgi: IDXGIDevice = d3d
        .cast()
        .with_context(|| "Windows encoder D3D11 device is not an IDXGIDevice")?;
    let adapter = unsafe { dxgi.GetAdapter() }
        .with_context(|| "Failed to get the Windows encoder DXGI adapter")?;
    let description = unsafe { adapter.GetDesc() }
        .with_context(|| "Failed to describe the Windows encoder DXGI adapter")?;
    Ok(description.VendorId)
}

fn activate_h264_encoder(
    dxgi_manager: &IMFDXGIDeviceManager,
    size: PixelSize,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
    adapter_vendor_id: u32,
) -> eros::Result<(IMFTransform, H264Profile)> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut activates = std::ptr::null_mut();
    let mut count = 0;
    let flags = MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0 | MFT_ENUM_FLAG_SYNCMFT.0;
    unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            windows::Win32::Media::MediaFoundation::MFT_ENUM_FLAG(flags),
            Some(&input),
            Some(&output),
            &mut activates,
            &mut count,
        )
    }
    .with_context(|| "Failed to enumerate hardware H.264 Media Foundation encoders")?;
    if count == 0 || activates.is_null() {
        eros::bail!("No hardware H.264 Media Foundation encoder supports NV12 input");
    }
    let activate_objects = unsafe {
        std::slice::from_raw_parts_mut(activates, count as usize)
            .iter_mut()
            .filter_map(Option::take)
            .collect::<Vec<_>>()
    };
    unsafe { CoTaskMemFree(Some(activates.cast())) };
    let mut candidates = activate_objects
        .into_iter()
        .map(|activate| {
            let name = media_foundation_activation_string(&activate, &MFT_FRIENDLY_NAME_Attribute);
            let hardware_url =
                media_foundation_activation_string(&activate, &MFT_ENUM_HARDWARE_URL_Attribute);
            let vendor = media_foundation_activation_string(
                &activate,
                &MFT_ENUM_HARDWARE_VENDOR_ID_Attribute,
            );
            WindowsEncoderCandidate {
                backend: WindowsEncoderBackend::from_activation(name.as_deref(), vendor.as_deref()),
                transform_clsid: unsafe { activate.GetGUID(&MFT_TRANSFORM_CLSID_Attribute) }.ok(),
                activate,
                name,
                hardware_url,
                vendor,
            }
        })
        .collect::<Vec<_>>();
    let preferred_backend = WindowsEncoderBackend::for_adapter_vendor(adapter_vendor_id);
    candidates.sort_by_key(|candidate| {
        if candidate.backend == preferred_backend {
            0
        } else if candidate.backend == WindowsEncoderBackend::MediaFoundation {
            1
        } else {
            2
        }
    });

    for (attempt, candidate) in candidates.iter().enumerate() {
        let transform = match unsafe { candidate.activate.ActivateObject::<IMFTransform>() } {
            Ok(transform) => transform,
            Err(error) => {
                warn!(
                    event = "windows_h264_encoder_candidate_rejected",
                    attempt = attempt + 1,
                    backend = candidate.backend.name(),
                    encoder_name = candidate.name.as_deref().unwrap_or("<unavailable>"),
                    error = ?error,
                    "Failed to activate a Windows hardware encoder candidate"
                );
                continue;
            }
        };
        let profile = match configure_transform(&transform, dxgi_manager, size, frame_rate, bitrate)
        {
            Ok(profile) => profile,
            Err(error) => {
                warn!(
                    event = "windows_h264_encoder_candidate_rejected",
                    attempt = attempt + 1,
                    backend = candidate.backend.name(),
                    encoder_name = candidate.name.as_deref().unwrap_or("<unavailable>"),
                    error = ?error,
                    "Windows hardware encoder candidate rejected the stream configuration"
                );
                continue;
            }
        };
        info!(
            event = "windows_h264_encoder_selected",
            backend = candidate.backend.name(),
            integration = "media-foundation-mft",
            codec = "h264",
            encoder_name = candidate.name.as_deref().unwrap_or("<unavailable>"),
            transform_clsid = ?candidate.transform_clsid,
            hardware_url = candidate.hardware_url.as_deref().unwrap_or("<unavailable>"),
            vendor_id = candidate.vendor.as_deref().unwrap_or("<unavailable>"),
            adapter_vendor_id = format_args!("{adapter_vendor_id:#06x}"),
            preferred_backend = preferred_backend.name(),
            input_memory = "d3d11-texture",
            input_format = "nv12",
            packetizer = "native-rtp-h264",
            hardware = true,
            candidate_count = count,
            selection_attempt = attempt + 1,
            h264_profile = profile.name(),
            "Selected Windows video encoder pipeline"
        );
        return Ok((transform, profile));
    }

    eros::bail!(
        "No Windows hardware H.264 encoder candidate accepted the D3D11 stream configuration"
    )
}

fn media_foundation_activation_string(
    activate: &IMFActivate,
    key: &windows::core::GUID,
) -> Option<String> {
    let length = unsafe { activate.GetStringLength(key) }.ok()? as usize;
    let mut value = vec![0; length + 1];
    unsafe { activate.GetString(key, &mut value, None) }.ok()?;
    Some(String::from_utf16_lossy(&value[..length]))
}

fn configure_transform(
    transform: &IMFTransform,
    dxgi_manager: &IMFDXGIDeviceManager,
    size: PixelSize,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
) -> eros::Result<H264Profile> {
    let mut attribute_low_latency = false;
    unsafe {
        if let Ok(attributes) = transform.GetAttributes() {
            attribute_low_latency = attributes.SetUINT32(&MF_LOW_LATENCY, 1).is_ok();
            let _ = attributes.SetUINT32(&MF_SA_D3D11_AWARE, 1);
            let _ = attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
        }
        transform
            .ProcessMessage(
                MFT_MESSAGE_SET_D3D_MANAGER,
                windows::core::Interface::as_raw(dxgi_manager) as usize,
            )
            .with_context(|| "Failed to attach DXGI device manager to H.264 encoder")?;
    }
    let codec_properties =
        configure_codec_low_latency(transform, attribute_low_latency, frame_rate, bitrate)?;

    let mut selected_profile = None;
    for profile in H264Profile::PREFERRED {
        let output_type = create_h264_output_type(size, frame_rate, bitrate, profile)?;
        match unsafe { transform.SetOutputType(0, &output_type, 0) } {
            Ok(()) => {
                selected_profile = Some(profile);
                break;
            }
            Err(error) => {
                debug!(
                    event = "windows_h264_encoder_profile_rejected",
                    h264_profile = profile.name(),
                    error = ?error,
                    "Windows H.264 encoder rejected an output profile"
                );
            }
        }
    }
    let selected_profile = selected_profile
        .with_context(|| "Windows H.264 encoder rejected High, Main, and Baseline")?;
    let input_type = create_nv12_input_type(size, frame_rate)?;
    unsafe { transform.SetInputType(0, &input_type, 0) }
        .with_context(|| "Failed to configure H.264 encoder NV12 input type")?;
    log_effective_codec_properties(transform, codec_properties)?;
    unsafe {
        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
            .with_context(|| "Failed to begin H.264 encoder streaming")?;
        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
            .with_context(|| "Failed to start H.264 encoder stream")?;
    }
    let effective_profile = unsafe {
        transform
            .GetOutputCurrentType(0)
            .and_then(|media_type| media_type.GetUINT32(&MF_MT_MPEG2_PROFILE))
    }
    .ok()
    .and_then(H264Profile::from_media_foundation_value)
    .unwrap_or(selected_profile);
    info!(
        event = "windows_h264_encoder_profile_selected",
        requested_profile = selected_profile.name(),
        effective_profile = effective_profile.name(),
        "Selected Windows H.264 encoder profile"
    );
    Ok(effective_profile)
}

#[derive(Clone, Copy)]
struct ConfiguredCodecProperties {
    bitrate_bps: u32,
    buffer_size_bytes: u32,
}

fn configure_codec_low_latency(
    transform: &IMFTransform,
    attribute_low_latency: bool,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
) -> eros::Result<ConfiguredCodecProperties> {
    let bitrate_bps = bitrate.bits_per_second();
    let codec_api: ICodecAPI = transform
        .cast()
        .with_context(|| "Windows H.264 encoder does not expose ICodecAPI")?;
    let enabled = variant_bool(true);
    let codec_low_latency =
        set_optional_codec_property(&codec_api, &CODECAPI_AVLowLatencyMode, &enabled);
    let common_low_latency =
        set_optional_codec_property(&codec_api, &CODECAPI_AVEncCommonLowLatency, &enabled);
    if !attribute_low_latency && !codec_low_latency && !common_low_latency {
        eros::bail!("Windows H.264 encoder does not support any low-latency mode");
    }
    let real_time =
        set_optional_codec_property(&codec_api, &CODECAPI_AVEncCommonRealTime, &enabled);
    let no_b_frames = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncMPVDefaultBPictureCount,
        &VARIANT::from(0u32),
    );
    let fastest_encoding = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncCommonQualityVsSpeed,
        &VARIANT::from(0u32),
    );
    set_required_codec_property(
        &codec_api,
        &CODECAPI_AVEncMPVGOPSize,
        &VARIANT::from(H264_INFINITE_GOP_LENGTH),
        "infinite H.264 GOP",
    )?;
    let constant_bitrate = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncCommonRateControlMode,
        &VARIANT::from(eAVEncCommonRateControlMode_CBR.0 as u32),
    );
    let mean_bitrate = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncCommonMeanBitRate,
        &VARIANT::from(bitrate_bps),
    );
    let (bounded_buffer, buffer_frames, buffer_size_bytes) =
        configure_cpb_buffer(&codec_api, bitrate_bps, frame_rate);
    info!(
        event = "windows_h264_encoder_low_latency_configured",
        attribute_low_latency,
        codec_low_latency,
        common_low_latency,
        real_time,
        no_b_frames,
        fastest_encoding,
        infinite_gop = true,
        gop_length = H264_INFINITE_GOP_LENGTH,
        key_frame_policy = "on-demand-idr-only",
        constant_bitrate,
        mean_bitrate,
        bounded_buffer,
        bitrate_bps,
        buffer_frames,
        buffer_size_bytes,
        "Configured Windows H.264 encoder for bounded real-time output without frame reordering"
    );
    Ok(ConfiguredCodecProperties {
        bitrate_bps,
        buffer_size_bytes,
    })
}

fn log_effective_codec_properties(
    transform: &IMFTransform,
    configured: ConfiguredCodecProperties,
) -> eros::Result<()> {
    let codec_api: ICodecAPI = transform
        .cast()
        .with_context(|| "Windows H.264 encoder does not expose ICodecAPI for property readback")?;
    for (property_name, property, requested) in [
        (
            "gop_frames",
            &CODECAPI_AVEncMPVGOPSize,
            i64::from(H264_INFINITE_GOP_LENGTH),
        ),
        (
            "cpb_bytes",
            &CODECAPI_AVEncCommonBufferSize,
            i64::from(configured.buffer_size_bytes),
        ),
        (
            "mean_bitrate_bps",
            &CODECAPI_AVEncCommonMeanBitRate,
            i64::from(configured.bitrate_bps),
        ),
        ("b_frame_count", &CODECAPI_AVEncMPVDefaultBPictureCount, 0),
        ("codec_low_latency", &CODECAPI_AVLowLatencyMode, 1),
        ("common_low_latency", &CODECAPI_AVEncCommonLowLatency, 1),
    ] {
        log_codec_property(&codec_api, property_name, property, requested);
    }
    Ok(())
}

fn log_codec_property(
    codec_api: &ICodecAPI,
    property_name: &'static str,
    property: &windows::core::GUID,
    requested: i64,
) {
    let effective = unsafe { codec_api.GetValue(property) }
        .ok()
        .and_then(|value| variant_numeric_value(&value));
    let mut minimum = VARIANT::default();
    let mut maximum = VARIANT::default();
    let mut step = VARIANT::default();
    let range_available =
        unsafe { codec_api.GetParameterRange(property, &mut minimum, &mut maximum, &mut step) }
            .is_ok();
    info!(
        event = "windows_h264_encoder_property_effective",
        property_name,
        requested,
        effective,
        minimum = range_available
            .then(|| variant_numeric_value(&minimum))
            .flatten(),
        maximum = range_available
            .then(|| variant_numeric_value(&maximum))
            .flatten(),
        step = range_available
            .then(|| variant_numeric_value(&step))
            .flatten(),
        range_available,
        "Read back a Windows H.264 encoder property"
    );
}

fn variant_numeric_value(value: &VARIANT) -> Option<i64> {
    let body = unsafe { &*value.Anonymous.Anonymous };
    match body.vt {
        VT_UI4 => Some(i64::from(unsafe { body.Anonymous.ulVal })),
        VT_I4 => Some(i64::from(unsafe { body.Anonymous.lVal })),
        VT_UI8 => i64::try_from(unsafe { body.Anonymous.ullVal }).ok(),
        VT_I8 => Some(unsafe { body.Anonymous.llVal }),
        VT_BOOL => Some(i64::from(unsafe { body.Anonymous.boolVal.0 != 0 })),
        _ => None,
    }
}

fn configure_cpb_buffer(
    codec_api: &ICodecAPI,
    bitrate_bps: u32,
    frame_rate: FrameRate,
) -> (bool, u32, u32) {
    for buffer_frames in [1_u32, 2, 4] {
        let buffer_size_bytes = cpb_size_bytes(bitrate_bps, frame_rate, buffer_frames);
        match unsafe {
            codec_api.SetValue(
                &CODECAPI_AVEncCommonBufferSize,
                &VARIANT::from(buffer_size_bytes),
            )
        } {
            Ok(()) => return (true, buffer_frames, buffer_size_bytes),
            Err(error) => {
                debug!(
                    event = "windows_h264_encoder_cpb_rejected",
                    buffer_frames,
                    buffer_size_bytes,
                    error = ?error,
                    "Windows H.264 encoder rejected a bounded CPB size"
                );
            }
        }
    }
    (false, 0, 0)
}

fn cpb_size_bytes(bitrate_bps: u32, frame_rate: FrameRate, buffer_frames: u32) -> u32 {
    let bits = u128::from(bitrate_bps)
        .saturating_mul(u128::from(frame_rate.denominator().max(1)))
        .saturating_mul(u128::from(buffer_frames.max(1)));
    let bits_per_byte_frame = u128::from(frame_rate.numerator().max(1)).saturating_mul(8);
    bits.div_ceil(bits_per_byte_frame)
        .clamp(1, u128::from(u32::MAX)) as u32
}

fn set_optional_codec_property(
    codec_api: &ICodecAPI,
    property: &windows::core::GUID,
    value: &VARIANT,
) -> bool {
    match unsafe { codec_api.SetValue(property, value) } {
        Ok(()) => true,
        Err(error) => {
            debug!(
                event = "windows_h264_encoder_optional_property_unsupported",
                property = ?property,
                error = ?error,
                "Windows H.264 encoder rejected an optional encoder property"
            );
            false
        }
    }
}

fn set_required_codec_property(
    codec_api: &ICodecAPI,
    property: &windows::core::GUID,
    value: &VARIANT,
    description: &str,
) -> eros::Result<()> {
    unsafe { codec_api.SetValue(property, value) }.with_context(|| {
        format!("Windows H.264 encoder does not support required {description}")
    })?;
    Ok(())
}

fn create_nv12_input_type(size: PixelSize, frame_rate: FrameRate) -> eros::Result<IMFMediaType> {
    let media_type =
        unsafe { MFCreateMediaType() }.with_context(|| "Failed to create NV12 input media type")?;
    set_video_type_common(&media_type, size, frame_rate, MFVideoFormat_NV12)?;
    let sample_size = size
        .width
        .checked_mul(size.height)
        .and_then(|pixels| pixels.checked_mul(3))
        .map(|bytes| bytes / 2)
        .with_context(|| "NV12 sample size overflow")?;
    unsafe {
        media_type.SetUINT32(&MF_MT_FIXED_SIZE_SAMPLES, 1)?;
        media_type.SetUINT32(&MF_MT_SAMPLE_SIZE, sample_size)?;
    }
    Ok(media_type)
}

fn create_h264_output_type(
    size: PixelSize,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
    profile: H264Profile,
) -> eros::Result<IMFMediaType> {
    let media_type = unsafe { MFCreateMediaType() }
        .with_context(|| "Failed to create H.264 output media type")?;
    set_video_type_common(&media_type, size, frame_rate, MFVideoFormat_H264)?;
    unsafe {
        media_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate.bits_per_second())?;
        media_type.SetUINT32(&MF_MT_MAX_KEYFRAME_SPACING, H264_INFINITE_GOP_LENGTH)?;
        media_type.SetUINT32(&MF_MT_MPEG2_PROFILE, profile.media_foundation_value())?;
        media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 0)?;
    }
    Ok(media_type)
}

fn set_video_type_common(
    media_type: &IMFMediaType,
    size: PixelSize,
    frame_rate: FrameRate,
    subtype: windows::core::GUID,
) -> eros::Result<()> {
    unsafe {
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
        media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(size.width, size.height))?;
        media_type.SetUINT64(
            &MF_MT_FRAME_RATE,
            pack_u32_pair(frame_rate.numerator(), frame_rate.denominator()),
        )?;
        media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
    }
    set_media_type_colorimetry(media_type)?;
    Ok(())
}

fn variant_bool(value: bool) -> VARIANT {
    let mut variant = VARIANT::default();
    variant.Anonymous.Anonymous = std::mem::ManuallyDrop::new(VARIANT_0_0 {
        vt: VT_BOOL,
        wReserved1: 0,
        wReserved2: 0,
        wReserved3: 0,
        Anonymous: VARIANT_0_0_0 {
            boolVal: if value {
                windows::Win32::Foundation::VARIANT_TRUE
            } else {
                windows::Win32::Foundation::VARIANT_FALSE
            },
        },
    });
    variant
}

fn with_sample_bytes<T>(
    sample: &IMFSample,
    operation: impl FnOnce(&[u8]) -> eros::Result<T>,
) -> eros::Result<T> {
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }
        .with_context(|| "Failed to get contiguous H.264 encoder output buffer")?;
    let mut data = std::ptr::null_mut();
    let mut length = 0;
    unsafe { buffer.Lock(&mut data, None, Some(&mut length)) }
        .with_context(|| "Failed to lock H.264 encoder output buffer")?;
    let bytes = if length == 0 {
        &[]
    } else {
        if data.is_null() {
            let _ = unsafe { buffer.Unlock() };
            eros::bail!("H.264 encoder output buffer returned a null data pointer");
        }
        unsafe { std::slice::from_raw_parts(data, length as usize) }
    };
    let result = operation(bytes);
    unsafe { buffer.Unlock() }.with_context(|| "Failed to unlock H.264 encoder output buffer")?;
    result
}

fn split_annex_b_access_units(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    vec![bytes]
}

fn split_h264_nals(bytes: &[u8]) -> Vec<&[u8]> {
    let annex_b = split_annex_b_nals(bytes);
    if annex_b.len() != 1 || find_start_code(bytes, 0).is_some() {
        return annex_b;
    }

    split_length_prefixed_nals(bytes).unwrap_or_else(|| vec![bytes])
}

fn split_length_prefixed_nals(bytes: &[u8]) -> Option<Vec<&[u8]>> {
    for length_size in [4usize, 2usize] {
        let mut offset = 0usize;
        let mut nals = Vec::new();
        while offset + length_size <= bytes.len() {
            let nal_len = match length_size {
                4 => u32::from_be_bytes([
                    bytes[offset],
                    bytes[offset + 1],
                    bytes[offset + 2],
                    bytes[offset + 3],
                ]) as usize,
                2 => u16::from_be_bytes([bytes[offset], bytes[offset + 1]]) as usize,
                _ => unreachable!(),
            };
            offset += length_size;
            if nal_len == 0 || offset + nal_len > bytes.len() {
                nals.clear();
                break;
            }
            nals.push(&bytes[offset..offset + nal_len]);
            offset += nal_len;
        }
        if offset == bytes.len() && !nals.is_empty() {
            return Some(nals);
        }
    }

    None
}

fn split_annex_b_nals(bytes: &[u8]) -> Vec<&[u8]> {
    let mut nals = Vec::new();
    let mut start = find_start_code(bytes, 0);
    let Some((mut nal_start_code, mut nal_start)) = start.take() else {
        return vec![bytes];
    };
    loop {
        let next = find_start_code(bytes, nal_start);
        let nal_end = next.map(|(offset, _)| offset).unwrap_or(bytes.len());
        if nal_start < nal_end {
            nals.push(&bytes[nal_start..nal_end]);
        }
        let Some((next_start_code, next_nal_start)) = next else {
            break;
        };
        nal_start_code = next_start_code;
        nal_start = next_nal_start;
    }
    let _ = nal_start_code;
    if nals.is_empty() { vec![bytes] } else { nals }
}

fn find_start_code(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut index = from;
    while index + 3 <= bytes.len() {
        if bytes[index..].starts_with(&[0, 0, 1]) {
            return Some((index, index + 3));
        }
        if index + 4 <= bytes.len() && bytes[index..].starts_with(&[0, 0, 0, 1]) {
            return Some((index, index + 4));
        }
        index += 1;
    }
    None
}

fn packetize_h264_nal(
    nal: &[u8],
    timestamp: u32,
    sequence: &mut u16,
    max_packet_size: usize,
    marker: bool,
) -> eros::Result<Vec<Bytes>> {
    if nal.is_empty() {
        return Ok(Vec::new());
    }
    let payload = strip_annex_b_start_codes(nal);
    if payload.len() <= max_packet_size - RTP_FIXED_HEADER_SIZE {
        let packet = rtp_packet(*sequence, timestamp, marker, payload);
        *sequence = sequence.wrapping_add(1);
        return Ok(vec![packet]);
    }
    let nal_header = *payload
        .first()
        .with_context(|| "Cannot packetize an empty H.264 NAL unit")?;
    let fragment_payload_capacity = max_packet_size - RTP_FIXED_HEADER_SIZE - 2;
    let mut packets = Vec::new();
    let mut offset = 1;
    while offset < payload.len() {
        let end = (offset + fragment_payload_capacity).min(payload.len());
        let start = offset == 1;
        let fragment_is_last = end == payload.len();
        let rtp_marker = marker && fragment_is_last;
        let fu_indicator = (nal_header & 0xe0) | 28;
        let fu_header = (if start { 0x80 } else { 0 })
            | (if fragment_is_last { 0x40 } else { 0 })
            | (nal_header & 0x1f);
        packets.push(rtp_packet_parts(
            *sequence,
            timestamp,
            rtp_marker,
            &[fu_indicator, fu_header],
            &payload[offset..end],
        ));
        *sequence = sequence.wrapping_add(1);
        offset = end;
    }
    Ok(packets)
}

fn rtp_packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> Bytes {
    rtp_packet_parts(sequence, timestamp, marker, &[], payload)
}

fn rtp_packet_parts(
    sequence: u16,
    timestamp: u32,
    marker: bool,
    payload_prefix: &[u8],
    payload: &[u8],
) -> Bytes {
    let mut packet =
        BytesMut::with_capacity(RTP_FIXED_HEADER_SIZE + payload_prefix.len() + payload.len());
    packet.put_u8(2 << 6);
    packet.put_u8((u8::from(marker) << 7) | H264_RTP_PAYLOAD_TYPE);
    packet.put_u16(sequence);
    packet.put_u32(timestamp);
    packet.put_u32(RTP_SSRC);
    packet.extend_from_slice(payload_prefix);
    packet.extend_from_slice(payload);
    packet.freeze()
}

fn strip_annex_b_start_codes(mut bytes: &[u8]) -> &[u8] {
    loop {
        if bytes.starts_with(&[0, 0, 0, 1]) {
            bytes = &bytes[4..];
        } else if bytes.starts_with(&[0, 0, 1]) {
            bytes = &bytes[3..];
        } else {
            return bytes;
        }
    }
}

fn frame_duration_hns(frame_rate: FrameRate) -> i64 {
    let numerator = frame_rate.numerator().max(1) as i64;
    let denominator = frame_rate.denominator().max(1) as i64;
    ((10_000_000_i64 * denominator) / numerator).max(1)
}

fn duration_hns(duration: Duration) -> i64 {
    (duration.as_nanos() / 100).min(i64::MAX as u128) as i64
}

fn frame_duration(frame_rate: FrameRate) -> Duration {
    let numerator = u128::from(frame_rate.numerator().max(1));
    let denominator = u128::from(frame_rate.denominator().max(1));
    let nanoseconds = 1_000_000_000_u128
        .saturating_mul(denominator)
        .div_ceil(numerator)
        .min(u64::MAX.into()) as u64;
    Duration::from_nanos(nanoseconds.max(1))
}

struct FixedFrameClock {
    period: Duration,
    next_frame_at: Instant,
}

impl FixedFrameClock {
    fn new(frame_rate: FrameRate) -> Self {
        Self::new_at(frame_rate, Instant::now())
    }

    fn new_at(frame_rate: FrameRate, now: Instant) -> Self {
        let period = frame_duration(frame_rate);
        Self {
            period,
            next_frame_at: now.checked_add(period).unwrap_or(now),
        }
    }

    fn delay(&self) -> Duration {
        self.delay_at(Instant::now())
    }

    fn delay_at(&self, now: Instant) -> Duration {
        self.next_frame_at.saturating_duration_since(now)
    }

    fn advance(&mut self) {
        self.advance_at(Instant::now());
    }

    fn advance_at(&mut self, now: Instant) {
        let next = self.next_frame_at.checked_add(self.period).unwrap_or(now);
        self.next_frame_at = if next > now {
            next
        } else {
            now.checked_add(self.period).unwrap_or(now)
        };
    }
}

fn create_input_sample(texture: &ID3D11Texture2D) -> eros::Result<IMFSample> {
    let buffer = unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false) }
        .with_context(|| "Failed to wrap NV12 D3D11 texture for Media Foundation")?;
    let sample = unsafe { MFCreateSample() }
        .with_context(|| "Failed to create Media Foundation input sample")?;
    unsafe { sample.AddBuffer(&buffer) }
        .with_context(|| "Failed to attach the D3D11 texture buffer to the input sample")?;
    Ok(sample)
}

fn frame_rate_rational_from_duration(duration_hns: i64) -> DXGI_RATIONAL {
    DXGI_RATIONAL {
        Numerator: 10_000_000,
        Denominator: duration_hns.max(1) as u32,
    }
}

fn rtp_timestamp_from_hns(sample_time_hns: i64) -> u32 {
    let ticks = u128::try_from(sample_time_hns.max(0))
        .unwrap_or(u128::MAX)
        .saturating_mul(90_000)
        / 10_000_000;
    1_u32.wrapping_add(ticks as u32)
}

fn pack_u32_pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

const RTP_FIXED_HEADER_SIZE: usize = 12;
const H264_RTP_PAYLOAD_TYPE: u8 = 96;
const RTP_SSRC: u32 = 0x5242_4954;
const MAX_ENCODER_OUTPUT_SAMPLE_SIZE: u32 = 4 * 1024 * 1024;
const MAX_ENCODER_STREAM_CHANGES_PER_OUTPUT: usize = 8;
const H264_INFINITE_GOP_LENGTH: u32 = u32::MAX;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use crate::kernel::{geometry::FrameRate, video_encoder::VideoFrameRateMode};

    use super::{
        EncoderTimeline, FixedFrameClock, H264_INFINITE_GOP_LENGTH, WindowsEncoderBackend,
        cpb_size_bytes, rtp_timestamp_from_hns, variant_bool, variant_numeric_value,
    };

    #[test]
    fn windows_h264_policy_disables_periodic_key_frames() {
        assert_eq!(H264_INFINITE_GOP_LENGTH, u32::MAX);
    }

    #[test]
    fn windows_h264_cpb_defaults_to_one_frame() {
        let frame_rate = FrameRate::new(120, 1).expect("frame rate");

        assert_eq!(cpb_size_bytes(20_000_000, frame_rate, 1), 20_834);
        assert_eq!(cpb_size_bytes(20_000_000, frame_rate, 2), 41_667);
        assert_eq!(cpb_size_bytes(20_000_000, frame_rate, 4), 83_334);
    }

    #[test]
    fn codec_property_readback_accepts_unsigned_and_boolean_variants() {
        assert_eq!(
            variant_numeric_value(&windows::Win32::System::Variant::VARIANT::from(
                74_000_000_u32
            )),
            Some(74_000_000)
        );
        assert_eq!(variant_numeric_value(&variant_bool(true)), Some(1));
        assert_eq!(variant_numeric_value(&variant_bool(false)), Some(0));
    }

    #[test]
    fn windows_encoder_backend_tracks_the_capture_adapter_vendor() {
        assert_eq!(
            WindowsEncoderBackend::for_adapter_vendor(0x10de),
            WindowsEncoderBackend::Nvenc
        );
        assert_eq!(
            WindowsEncoderBackend::for_adapter_vendor(0x8086),
            WindowsEncoderBackend::QuickSync
        );
        assert_eq!(
            WindowsEncoderBackend::for_adapter_vendor(0x1002),
            WindowsEncoderBackend::Amf
        );
        assert_eq!(
            WindowsEncoderBackend::for_adapter_vendor(0xffff),
            WindowsEncoderBackend::MediaFoundation
        );
    }

    #[test]
    fn windows_encoder_backend_classifies_hardware_mft_identity() {
        assert_eq!(
            WindowsEncoderBackend::from_activation(Some("NVIDIA H.264 Encoder MFT"), None),
            WindowsEncoderBackend::Nvenc
        );
        assert_eq!(
            WindowsEncoderBackend::from_activation(Some("Intel Quick Sync Video H.264"), None),
            WindowsEncoderBackend::QuickSync
        );
        assert_eq!(
            WindowsEncoderBackend::from_activation(Some("AMD AMF H.264 Encoder"), None),
            WindowsEncoderBackend::Amf
        );
    }

    #[test]
    fn dynamic_timeline_preserves_two_seconds_of_capture_idle_time() {
        let start = Instant::now();
        let mut timeline = EncoderTimeline::new(
            VideoFrameRateMode::Dynamic,
            FrameRate::new(120, 1).expect("frame rate"),
        );
        let first = timeline.next_sample_time_hns(start);
        let after_idle = timeline.next_sample_time_hns(start + Duration::from_secs(2));

        assert_eq!(first, 0);
        assert_eq!(after_idle, 20_000_000);
        assert_eq!(
            rtp_timestamp_from_hns(after_idle).wrapping_sub(rtp_timestamp_from_hns(first)),
            180_000
        );
    }

    #[test]
    fn dynamic_timeline_keeps_repeated_recovery_frames_monotonic() {
        let start = Instant::now();
        let mut timeline = EncoderTimeline::new(
            VideoFrameRateMode::Dynamic,
            FrameRate::new(120, 1).expect("frame rate"),
        );
        let first = timeline.next_sample_time_hns(start);
        let repeated = timeline.next_sample_time_hns(start);

        assert_eq!(first, 0);
        assert_eq!(repeated, 1);
    }

    #[test]
    fn fixed_frame_clock_keeps_its_deadline_when_content_arrives() {
        let start = Instant::now();
        let clock = FixedFrameClock::new_at(FrameRate::new(100, 1).expect("frame rate"), start);

        assert_eq!(
            clock.delay_at(start + Duration::from_millis(4)),
            Duration::from_millis(6)
        );
        assert_eq!(
            clock.delay_at(start + Duration::from_millis(9)),
            Duration::from_millis(1)
        );
    }

    #[test]
    fn fixed_frame_clock_skips_missed_ticks_without_bursting() {
        let start = Instant::now();
        let mut clock = FixedFrameClock::new_at(FrameRate::new(100, 1).expect("frame rate"), start);

        clock.advance_at(start + Duration::from_millis(35));

        assert_eq!(
            clock.delay_at(start + Duration::from_millis(35)),
            Duration::from_millis(10)
        );
    }
}
