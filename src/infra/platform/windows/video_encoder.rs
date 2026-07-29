use std::{collections::VecDeque, future::Future, rc::Rc, time::Duration};

use bytes::Bytes;
use eros::Context as _;
use futures_util::{FutureExt as _, StreamExt as _};
use tracing::{debug, info, warn};
use windows::{
    Win32::{
        Foundation::RECT,
        Graphics::{
            Direct3D11::{
                D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_BIND_VIDEO_ENCODER,
                D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
                D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0,
                D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D, ID3D11Device,
                ID3D11Resource, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoContext1,
                ID3D11VideoDevice, ID3D11VideoProcessorEnumerator,
            },
            Dxgi::Common::{
                DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
            },
        },
        Media::MediaFoundation::{
            CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonLowLatency,
            CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
            CODECAPI_AVEncCommonRealTime, CODECAPI_AVEncMPVDefaultBPictureCount,
            CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoForceKeyFrame, CODECAPI_AVLowLatencyMode,
            ICodecAPI, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaEventGenerator, IMFMediaType,
            IMFSample, IMFTransform, METransformDrainComplete, METransformHaveOutput,
            METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE, MF_E_NO_MORE_TYPES,
            MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY,
            MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE, MF_MT_FIXED_SIZE_SAMPLES,
            MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
            MF_MT_MAX_KEYFRAME_SPACING, MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO,
            MF_MT_SAMPLE_SIZE, MF_MT_SUBTYPE, MF_SA_D3D11_AWARE, MF_TRANSFORM_ASYNC_UNLOCK,
            MF_VERSION, MFCreateDXGIDeviceManager, MFCreateDXGISurfaceBuffer, MFCreateMediaType,
            MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSTARTUP_FULL, MFStartup,
            MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
            MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_END_OF_STREAM, MFT_MESSAGE_NOTIFY_END_STREAMING,
            MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER,
            MFT_OUTPUT_DATA_BUFFER, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFT_REGISTER_TYPE_INFO,
            MFTEnumEx, MFVideoFormat_H264, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
            eAVEncCommonRateControlMode_CBR, eAVEncH264VProfile_Base,
        },
        System::{
            Com::CoTaskMemFree,
            Variant::{VARIANT, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL},
        },
    },
    core::Interface as _,
};

use crate::{
    infra::platform::{
        frame_pipeline::WgcFramePipelineFrame,
        host_video_probe::{HostVideoFrameProbe, HostVideoProbeReporter},
    },
    kernel::{
        geometry::{FrameRate, PixelSize},
        video_encoder::{
            VideoBitrate, VideoCodec, VideoEncoder, VideoEncoderCommand, VideoEncoderParameters,
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
    type Input = WgcFramePipelineFrame;
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
        SendPacket: FnMut(Self::Packet) -> SendFuture,
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
    Frames: futures_core::Stream<Item = eros::Result<Rc<WgcFramePipelineFrame>>> + Unpin,
    Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
    SendPacket: FnMut(WindowsVideoPacket) -> SendFuture,
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
        bitrate_bps = parameters.bitrate.bits_per_second(),
        max_packet_size,
        "Starting Windows H.264 encoder"
    );
    let mut encoder = MfH264Encoder::new(
        &first_frame,
        frame_rate,
        parameters.bitrate,
        max_packet_size,
    )
    .with_context(|| "Failed to initialize Windows Media Foundation H.264 encoder")?;
    encoder.request_key_frame();
    encoder.encode_frame(&first_frame, &mut send_packet).await?;

    while let Some(frame) = frames.next().await {
        while let Some(command) = commands.next().now_or_never().flatten() {
            match command {
                VideoEncoderCommand::RequestKeyFrame => encoder.request_key_frame(),
            }
        }
        let frame = frame?;
        encoder.encode_frame(&frame, &mut send_packet).await?;
    }

    encoder.drain(&mut send_packet).await
}

struct MfH264Encoder {
    transform: IMFTransform,
    transform_event_generator: Option<IMFMediaEventGenerator>,
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
    frame_duration_hns: i64,
    next_sample_time_hns: i64,
    sequence: u16,
    timestamp: u32,
    timestamp_step: u32,
    max_packet_size: usize,
    force_key_frame: bool,
    logged_output_format: bool,
    async_accepts_input: bool,
    pending_probes: VecDeque<Option<HostVideoFrameProbe>>,
    probe_reporter: Option<HostVideoProbeReporter>,
}

struct EncodedOutput {
    sample: IMFSample,
    probe: Option<HostVideoFrameProbe>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncEncoderEvent {
    NeedInput,
    HaveOutput,
    DrainComplete,
}

impl MfH264Encoder {
    fn new(
        first_frame: &WgcFramePipelineFrame,
        frame_rate: FrameRate,
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
            .with_context(|| "Failed to get the WGC frame D3D11 device")?;
        let video: ID3D11VideoDevice = d3d
            .cast()
            .with_context(|| "D3D11 device does not expose ID3D11VideoDevice")?;
        let context = unsafe { d3d.GetImmediateContext() }
            .with_context(|| "Failed to get the D3D11 immediate context")?;
        let video_context: ID3D11VideoContext = context
            .cast()
            .with_context(|| "D3D11 immediate context does not expose ID3D11VideoContext")?;
        let dxgi_manager = create_dxgi_device_manager(&d3d)?;
        let transform = activate_h264_encoder()?;
        configure_transform(&transform, &dxgi_manager, frame_size, frame_rate, bitrate)?;
        let output_stream_info = unsafe { transform.GetOutputStreamInfo(0) }
            .with_context(|| "Failed to query H.264 encoder output stream info")?;
        let transform_event_generator = transform.cast::<IMFMediaEventGenerator>().ok();
        let async_accepts_input = transform_event_generator.is_none();
        let probe_reporter = first_frame
            .probe
            .as_ref()
            .map(|probe| HostVideoProbeReporter::new(probe.report_interval()));

        Ok(Self {
            transform,
            transform_event_generator,
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
            frame_duration_hns: frame_duration_hns(frame_rate),
            next_sample_time_hns: 0,
            sequence: 0,
            timestamp: 1,
            timestamp_step: rtp_timestamp_step(frame_rate),
            max_packet_size,
            force_key_frame: false,
            logged_output_format: false,
            async_accepts_input,
            pending_probes: VecDeque::new(),
            probe_reporter,
        })
    }

    fn request_key_frame(&mut self) {
        self.force_key_frame = true;
    }

    async fn encode_frame<SendPacket, SendFuture>(
        &mut self,
        frame: &WgcFramePipelineFrame,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(WindowsVideoPacket) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
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

        if self.transform_event_generator.is_some() {
            while !self.async_accepts_input {
                self.handle_async_encoder_event().await?;
            }
        }

        if let Some(probe) = &mut probe {
            probe.mark_vpp_started();
        }
        let sample = self.create_input_sample(frame)?;
        if let Some(probe) = &mut probe {
            probe.mark_vpp_completed();
        }
        unsafe {
            sample.SetSampleTime(self.next_sample_time_hns)?;
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
        if self.probe_reporter.is_some() {
            self.pending_probes.push_back(probe);
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
        self.next_sample_time_hns = self
            .next_sample_time_hns
            .saturating_add(self.frame_duration_hns);
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
        SendPacket: FnMut(WindowsVideoPacket) -> SendFuture,
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
                    AsyncEncoderEvent::DrainComplete => return Ok(()),
                }
            }
        }
        self.receive_packets(send_packet).await
    }

    async fn receive_packets<SendPacket, SendFuture>(
        &mut self,
        send_packet: &mut SendPacket,
    ) -> eros::Result<()>
    where
        SendPacket: FnMut(WindowsVideoPacket) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        loop {
            let Some(mut output) = self.process_output()? else {
                return Ok(());
            };
            let access_units = sample_to_bytes(output.sample)?;
            if !self.logged_output_format {
                let first_bytes = &access_units[..access_units.len().min(16)];
                debug!(
                    event = "windows_h264_output_format",
                    output_size = access_units.len(),
                    annex_b = find_start_code(&access_units, 0).is_some(),
                    first_bytes = ?first_bytes,
                    "Received first Windows H.264 encoder output"
                );
                self.logged_output_format = true;
            }
            let mut rtp_packets = 0u64;
            let mut rtp_bytes = 0u64;
            for access_unit in split_annex_b_access_units(&access_units) {
                let nals = split_h264_nals(access_unit);
                for (index, nal) in nals.iter().enumerate() {
                    let marker = index + 1 == nals.len();
                    let packets = packetize_h264_nal(
                        nal,
                        self.timestamp,
                        &mut self.sequence,
                        self.max_packet_size,
                        marker,
                    )?;
                    for packet in packets {
                        rtp_packets = rtp_packets.saturating_add(1);
                        rtp_bytes = rtp_bytes
                            .saturating_add(u64::try_from(packet.len()).unwrap_or(u64::MAX));
                        send_packet(WindowsVideoPacket(packet)).await?;
                    }
                }
                self.timestamp = self.timestamp.wrapping_add(self.timestamp_step);
            }
            if let (Some(reporter), Some(probe)) = (&mut self.probe_reporter, output.probe.take()) {
                reporter.record_frame(probe, rtp_packets, rtp_bytes);
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

        for _ in 0..100 {
            match unsafe {
                events.GetEvent(windows::Win32::Media::MediaFoundation::MF_EVENT_FLAG_NO_WAIT)
            } {
                Ok(event) => {
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
                    if event_type == METransformHaveOutput.0 as u32 {
                        return Ok(AsyncEncoderEvent::HaveOutput);
                    }
                    if event_type == METransformNeedInput.0 as u32 {
                        return Ok(AsyncEncoderEvent::NeedInput);
                    }
                    if event_type == METransformDrainComplete.0 as u32 {
                        return Ok(AsyncEncoderEvent::DrainComplete);
                    }
                }
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    compio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => {
                    Err(error).with_context(|| "Failed to read Windows H.264 encoder event")?;
                }
            }
        }

        Ok(AsyncEncoderEvent::NeedInput)
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
                    let mut probe = self.pending_probes.pop_front().flatten();
                    if let Some(probe) = &mut probe {
                        probe.mark_encoder_completed();
                    }
                    return Ok(Some(EncodedOutput { sample, probe }));
                }
                Err(error) if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(None),
                Err(error) if error.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    warn!(
                        event = "windows_h264_encoder_stream_change",
                        output_status = output.dwStatus,
                        process_status = status,
                        attempt = stream_change_count + 1,
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
        let mut h264_type_count = 0u32;
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
            h264_type_count += 1;
            match unsafe {
                self.transform
                    .SetOutputType(self.output_stream_id, &media_type, 0)
            } {
                Ok(()) => {
                    self.refresh_output_stream_info()?;
                    info!(
                        event = "windows_h264_encoder_output_type_renegotiated",
                        available_type_index = index,
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
            create_h264_output_type(self.frame_size, self.frame_rate, self.bitrate)?;
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

    fn create_input_sample(&mut self, frame: &WgcFramePipelineFrame) -> eros::Result<IMFSample> {
        let nv12 = self.convert_to_nv12(frame.texture())?;
        let buffer = unsafe { MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &nv12, 0, false) }
            .with_context(|| "Failed to wrap NV12 D3D11 texture for Media Foundation")?;
        let sample = unsafe { MFCreateSample() }
            .with_context(|| "Failed to create Media Foundation input sample")?;
        unsafe { sample.AddBuffer(&buffer) }
            .with_context(|| "Failed to attach the D3D11 texture buffer to the input sample")?;
        Ok(sample)
    }

    fn convert_to_nv12(&mut self, texture: &ID3D11Texture2D) -> eros::Result<ID3D11Texture2D> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { texture.GetDesc(&mut desc) };
        if desc.Format == DXGI_FORMAT_NV12
            && desc.Width == self.frame_size.width
            && desc.Height == self.frame_size.height
        {
            return Ok(texture.clone());
        }
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            eros::bail!("WGC frame has unsupported D3D11 format {:?}", desc.Format);
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
    output: ID3D11Texture2D,
    enumerator: ID3D11VideoProcessorEnumerator,
    output_view: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessorOutputView,
    processor: windows::Win32::Graphics::Direct3D11::ID3D11VideoProcessor,
    video: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
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
        let mut output = None;
        unsafe { d3d.CreateTexture2D(&output_desc, None, Some(&mut output)) }
            .with_context(|| "Failed to allocate NV12 encoder texture")?;
        let output = output.with_context(|| "CreateTexture2D returned no NV12 texture")?;

        let output_view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
            ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
            },
        };
        let mut output_view = None;
        unsafe {
            video.CreateVideoProcessorOutputView(
                &output.cast::<ID3D11Resource>()?,
                &enumerator,
                &output_view_desc,
                Some(&mut output_view),
            )
        }
        .with_context(|| "Failed to create NV12 video processor output view")?;
        let output_view = output_view.with_context(|| "D3D11 returned no processor output view")?;
        set_video_processor_color_space(video_context, &processor);
        debug!(
            event = "windows_video_processor_scaler_configured",
            source_width = source_size.width,
            source_height = source_size.height,
            output_width = output_size.width,
            output_height = output_size.height,
            "Configured full-frame D3D11 video processor scaling"
        );

        Ok(Self {
            source_size,
            output_size,
            output,
            enumerator,
            output_view,
            processor,
            video: video.clone(),
            video_context: video_context.clone(),
        })
    }

    fn matches(&self, source_size: PixelSize, output_size: PixelSize) -> bool {
        self.source_size == source_size && self.output_size == output_size
    }

    fn convert(&mut self, texture: &ID3D11Texture2D) -> eros::Result<ID3D11Texture2D> {
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
                .VideoProcessorBlt(&self.processor, &self.output_view, 0, &streams)
        }
        .with_context(|| "Failed to convert BGRA WGC texture to NV12")?;
        Ok(self.output.clone())
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

fn activate_h264_encoder() -> eros::Result<IMFTransform> {
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
    let activate = unsafe { (*activates).clone() }
        .with_context(|| "Media Foundation returned a null encoder activation object")?;
    unsafe { CoTaskMemFree(Some(activates.cast())) };
    Ok(unsafe { activate.ActivateObject::<IMFTransform>() }
        .with_context(|| "Failed to activate the hardware H.264 Media Foundation encoder")?)
}

fn configure_transform(
    transform: &IMFTransform,
    dxgi_manager: &IMFDXGIDeviceManager,
    size: PixelSize,
    frame_rate: FrameRate,
    bitrate: VideoBitrate,
) -> eros::Result<()> {
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
    configure_codec_low_latency(transform, attribute_low_latency, bitrate)?;

    let output_type = create_h264_output_type(size, frame_rate, bitrate)?;
    unsafe { transform.SetOutputType(0, &output_type, 0) }
        .with_context(|| "Failed to configure H.264 encoder output type")?;
    let input_type = create_nv12_input_type(size, frame_rate)?;
    unsafe { transform.SetInputType(0, &input_type, 0) }
        .with_context(|| "Failed to configure H.264 encoder NV12 input type")?;
    unsafe {
        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
            .with_context(|| "Failed to begin H.264 encoder streaming")?;
        transform
            .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
            .with_context(|| "Failed to start H.264 encoder stream")?;
    }
    Ok(())
}

fn configure_codec_low_latency(
    transform: &IMFTransform,
    attribute_low_latency: bool,
    bitrate: VideoBitrate,
) -> eros::Result<()> {
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
    let bounded_gop = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncMPVGOPSize,
        &VARIANT::from(H264_KEY_FRAME_INTERVAL),
    );
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
    let buffer_size_bytes = bitrate_bps / 8 / H264_CPB_INTERVALS_PER_SECOND;
    let bounded_buffer = set_optional_codec_property(
        &codec_api,
        &CODECAPI_AVEncCommonBufferSize,
        &VARIANT::from(buffer_size_bytes),
    );
    info!(
        event = "windows_h264_encoder_low_latency_configured",
        attribute_low_latency,
        codec_low_latency,
        common_low_latency,
        real_time,
        no_b_frames,
        bounded_gop,
        key_frame_interval = H264_KEY_FRAME_INTERVAL,
        constant_bitrate,
        mean_bitrate,
        bounded_buffer,
        bitrate_bps,
        buffer_size_bytes,
        "Configured Windows H.264 encoder for bounded real-time output without frame reordering"
    );
    Ok(())
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
) -> eros::Result<IMFMediaType> {
    let media_type = unsafe { MFCreateMediaType() }
        .with_context(|| "Failed to create H.264 output media type")?;
    set_video_type_common(&media_type, size, frame_rate, MFVideoFormat_H264)?;
    unsafe {
        media_type.SetUINT32(&MF_MT_AVG_BITRATE, bitrate.bits_per_second())?;
        media_type.SetUINT32(&MF_MT_MAX_KEYFRAME_SPACING, H264_KEY_FRAME_INTERVAL)?;
        media_type.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Base.0 as u32)?;
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

fn sample_to_bytes(sample: IMFSample) -> eros::Result<Vec<u8>> {
    let buffer = unsafe { sample.ConvertToContiguousBuffer() }
        .with_context(|| "Failed to get contiguous H.264 encoder output buffer")?;
    media_buffer_to_bytes(&buffer)
}

fn media_buffer_to_bytes(buffer: &IMFMediaBuffer) -> eros::Result<Vec<u8>> {
    let mut data = std::ptr::null_mut();
    let mut length = 0;
    unsafe { buffer.Lock(&mut data, None, Some(&mut length)) }
        .with_context(|| "Failed to lock H.264 encoder output buffer")?;
    let bytes = if length == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(data, length as usize) }.to_vec()
    };
    unsafe { buffer.Unlock() }.with_context(|| "Failed to unlock H.264 encoder output buffer")?;
    Ok(bytes)
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
        let mut rtp_payload = Vec::with_capacity(2 + end - offset);
        rtp_payload.push(fu_indicator);
        rtp_payload.push(fu_header);
        rtp_payload.extend_from_slice(&payload[offset..end]);
        packets.push(rtp_packet(*sequence, timestamp, rtp_marker, &rtp_payload));
        *sequence = sequence.wrapping_add(1);
        offset = end;
    }
    Ok(packets)
}

fn rtp_packet(sequence: u16, timestamp: u32, marker: bool, payload: &[u8]) -> Bytes {
    let mut packet = Vec::with_capacity(RTP_FIXED_HEADER_SIZE + payload.len());
    packet.push(2 << 6);
    packet.push((u8::from(marker) << 7) | H264_RTP_PAYLOAD_TYPE);
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(&timestamp.to_be_bytes());
    packet.extend_from_slice(&RTP_SSRC.to_be_bytes());
    packet.extend_from_slice(payload);
    Bytes::from(packet)
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

fn frame_rate_rational_from_duration(duration_hns: i64) -> DXGI_RATIONAL {
    DXGI_RATIONAL {
        Numerator: 10_000_000,
        Denominator: duration_hns.max(1) as u32,
    }
}

fn rtp_timestamp_step(frame_rate: FrameRate) -> u32 {
    let numerator = frame_rate.numerator().max(1) as u64;
    let denominator = frame_rate.denominator().max(1) as u64;
    ((90_000_u64 * denominator) / numerator).max(1) as u32
}

fn pack_u32_pair(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

const RTP_FIXED_HEADER_SIZE: usize = 12;
const H264_RTP_PAYLOAD_TYPE: u8 = 96;
const RTP_SSRC: u32 = 0x5242_4954;
const MAX_ENCODER_OUTPUT_SAMPLE_SIZE: u32 = 4 * 1024 * 1024;
const MAX_ENCODER_STREAM_CHANGES_PER_OUTPUT: usize = 8;
const H264_KEY_FRAME_INTERVAL: u32 = 1_024;
const H264_CPB_INTERVALS_PER_SECOND: u32 = 10;
