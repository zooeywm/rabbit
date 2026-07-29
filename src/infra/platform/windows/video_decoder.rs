use std::{collections::VecDeque, future::Future, ptr, time::Duration};

use eros::Context as _;
use futures_util::StreamExt as _;
use tracing::info;
use windows::{
    Win32::{
        Foundation::{HMODULE, RPC_E_CHANGED_MODE},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0},
            Direct3D10::ID3D10Multithread,
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device, ID3D11Texture2D,
            },
        },
        Media::MediaFoundation::{
            CLSID_MSH264DecoderMFT, IMFDXGIBuffer, IMFMediaEventGenerator, IMFSample, IMFTransform,
            METransformHaveOutput, METransformNeedInput, MF_E_NO_EVENTS_AVAILABLE,
            MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY,
            MF_MT_FRAME_SIZE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE,
            MF_SA_D3D11_AWARE, MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateDXGIDeviceManager,
            MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video,
            MFSTARTUP_FULL, MFStartup, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
            MFT_MESSAGE_NOTIFY_END_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
            MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
            MFT_OUTPUT_STREAM_PROVIDES_SAMPLES, MFVideoArea, MFVideoFormat_H264,
            MFVideoFormat_NV12,
        },
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
    },
    core::Interface as _,
};

use crate::kernel::{
    geometry::PixelSize, screen_manager::ScreenId, session::ReceivedVideoFrame,
    video_decoder::VideoDecoder,
};

use super::{
    client_video_probe::{ClientVideoFrameProbe, ClientVideoProbeClock},
    video_color::set_media_type_colorimetry,
};

#[derive(Debug)]
pub(crate) struct WindowsDecodedFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) size: PixelSize,
    pub(crate) source_x: u32,
    pub(crate) source_y: u32,
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) subresource_index: u32,
    pub(crate) _sample: SendMfSample,
    pub(crate) probe: Option<ClientVideoFrameProbe>,
}

#[derive(Debug)]
pub(crate) struct SendMfSample {
    _sample: IMFSample,
}

// Media Foundation samples returned by a D3D-aware decoder are designed to
// cross pipeline threads. Keeping this wrapper alive prevents the decoder from
// recycling the presented texture subresource too early.
unsafe impl Send for SendMfSample {}

impl crate::kernel::video_decoder::DecodedVideoFrame for WindowsDecodedFrame {
    fn screen_id(&self) -> ScreenId {
        self.screen_id
    }
}

pub(crate) struct WindowsVideoDecoder;

impl VideoDecoder for WindowsVideoDecoder {
    type Input = ReceivedVideoFrame;
    type Frame = WindowsDecodedFrame;

    fn run<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<Self::Input>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        Self::run_with_probing(inputs, present_frame, false)
    }
}

impl WindowsVideoDecoder {
    pub(crate) fn run_with_probing<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
        enable_probing: bool,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(WindowsDecodedFrame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        run_windows_decoder(inputs, present_frame, enable_probing)
    }
}

async fn run_windows_decoder<Inputs, PresentFrame, PresentFuture>(
    mut inputs: Inputs,
    mut present_frame: PresentFrame,
    enable_probing: bool,
) -> eros::Result<()>
where
    Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
    PresentFrame: FnMut(WindowsDecodedFrame) -> PresentFuture,
    PresentFuture: Future<Output = eros::Result<()>>,
{
    let Some(first) = inputs.next().await else {
        return Ok(());
    };
    let mut decoder = MfH264Decoder::new(enable_probing)?;
    for frame in decoder.decode(first?).await? {
        present_frame(frame).await?;
    }
    while let Some(input) = inputs.next().await {
        for frame in decoder.decode(input?).await? {
            present_frame(frame).await?;
        }
    }
    Ok(())
}

struct MfH264Decoder {
    transform: IMFTransform,
    events: Option<IMFMediaEventGenerator>,
    _device: ID3D11Device,
    _device_manager: windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager,
    output_provides_samples: bool,
    output_region: DecoderOutputRegion,
    next_sample_time: i64,
    logged_first_output_pending: bool,
    logged_first_output: bool,
    probe_clock: Option<ClientVideoProbeClock>,
    pending_probes: VecDeque<ClientVideoFrameProbe>,
    _com: ComApartment,
}

impl MfH264Decoder {
    fn new(enable_probing: bool) -> eros::Result<Self> {
        let com = ComApartment::initialize()?;
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .with_context(|| "Failed to start Media Foundation for H.264 decoding")?;
        let device = create_video_device()?;
        let device_manager = create_device_manager(&device)?;
        let transform = activate_h264_decoder()?;
        unsafe {
            if let Ok(attributes) = transform.GetAttributes() {
                let _ = attributes.SetUINT32(&MF_LOW_LATENCY, 1);
                let _ = attributes.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
            }
            transform
                .ProcessMessage(
                    MFT_MESSAGE_SET_D3D_MANAGER,
                    windows::core::Interface::as_raw(&device_manager) as usize,
                )
                .with_context(|| "Failed to attach D3D11 device manager to H.264 decoder")?;
        }
        let output_region = configure_decoder_types(&transform)?;
        let stream_info = unsafe { transform.GetOutputStreamInfo(0) }
            .with_context(|| "Failed to query H.264 decoder output stream")?;
        let output_provides_samples =
            stream_info.dwFlags & MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 != 0;
        if !output_provides_samples {
            eros::bail!("Windows H.264 decoder does not provide D3D11 output samples");
        }
        unsafe {
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        }
        info!(
            event = "windows_video_decoder_selected",
            framework = "media-foundation",
            codec = "h264",
            decoder_name = "Microsoft H.264 Decoder MFT",
            transform_clsid = ?CLSID_MSH264DecoderMFT,
            device_manager = "d3d11",
            output_memory = "d3d11-texture",
            output_format = "nv12",
            d3d11_aware = true,
            "Selected Windows video decoder pipeline"
        );
        Ok(Self {
            events: transform.cast().ok(),
            transform,
            _device: device,
            _device_manager: device_manager,
            output_provides_samples,
            output_region,
            next_sample_time: 0,
            logged_first_output_pending: false,
            logged_first_output: false,
            probe_clock: enable_probing.then(ClientVideoProbeClock::default),
            pending_probes: VecDeque::new(),
            _com: com,
        })
    }

    async fn decode(
        &mut self,
        input: ReceivedVideoFrame,
    ) -> eros::Result<Vec<WindowsDecodedFrame>> {
        if self.events.is_some() {
            self.wait_for_need_input().await?;
        }
        let rtp_bytes = input
            .packets
            .iter()
            .fold(0usize, |total, packet| total.saturating_add(packet.len()));
        let mut probe = self
            .probe_clock
            .as_mut()
            .map(|clock| clock.frame(input.packets.len(), rtp_bytes));
        let access_unit = depayload_h264(&input.packets)?;
        let sample = compressed_sample(&access_unit, self.next_sample_time)?;
        if self.next_sample_time == 0 {
            info!(
                event = "windows_video_decoder_first_input",
                bytes = access_unit.len(),
                rtp_packets = input.packets.len(),
                "Submitting first H.264 access unit to Media Foundation"
            );
        }
        self.next_sample_time = self.next_sample_time.saturating_add(1);
        if let Some(probe) = &mut probe {
            probe.mark_decoder_entered();
        }
        unsafe { self.transform.ProcessInput(0, &sample, 0) }
            .with_context(|| "Failed to submit H.264 access unit to Windows decoder")?;
        if let Some(probe) = probe {
            self.pending_probes.push_back(probe);
        }

        let mut frames = Vec::new();
        if self.events.is_some() {
            while let DecoderEvent::HaveOutput = self.next_event().await? {
                self.drain_output(input.screen_id, &mut frames)?;
            }
        } else {
            self.drain_output(input.screen_id, &mut frames)?;
        }
        Ok(frames)
    }

    fn drain_output(
        &mut self,
        screen_id: ScreenId,
        frames: &mut Vec<WindowsDecodedFrame>,
    ) -> eros::Result<()> {
        loop {
            match self.process_output(screen_id)? {
                Some(frame) => frames.push(frame),
                None => return Ok(()),
            }
        }
    }

    fn process_output(&mut self, screen_id: ScreenId) -> eros::Result<Option<WindowsDecodedFrame>> {
        debug_assert!(self.output_provides_samples);
        if !self.logged_first_output_pending {
            info!(
                event = "windows_video_decoder_first_output_pending",
                "Requesting the first D3D11 output sample from Media Foundation"
            );
            self.logged_first_output_pending = true;
        }
        loop {
            let mut output = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: std::mem::ManuallyDrop::new(None),
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
            if let Err(error) = result {
                if error.code() == MF_E_TRANSFORM_NEED_MORE_INPUT {
                    return Ok(None);
                }
                if error.code() == MF_E_TRANSFORM_STREAM_CHANGE {
                    self.output_region = configure_decoder_output_type(&self.transform)?;
                    continue;
                }
                Err::<(), _>(error)
                    .with_context(|| "Failed to receive Windows H.264 decoder output")?;
            }
            let sample =
                sample.with_context(|| "Windows H.264 decoder returned no output sample")?;
            let mut probe = self.pending_probes.pop_front();
            if let Some(probe) = &mut probe {
                probe.mark_decoder_completed();
            }
            let frame = decoded_frame(screen_id, sample, self.output_region, probe)?;
            if !self.logged_first_output {
                info!(
                    event = "windows_video_decoder_first_output",
                    width = frame.size.width,
                    height = frame.size.height,
                    source_x = frame.source_x,
                    source_y = frame.source_y,
                    subresource_index = frame.subresource_index,
                    "Received first D3D11 decoder surface"
                );
                self.logged_first_output = true;
            }
            return Ok(Some(frame));
        }
    }

    async fn wait_for_need_input(&self) -> eros::Result<()> {
        loop {
            if self.next_event().await? == DecoderEvent::NeedInput {
                return Ok(());
            }
        }
    }

    async fn next_event(&self) -> eros::Result<DecoderEvent> {
        let events = self
            .events
            .as_ref()
            .with_context(|| "Windows decoder has no asynchronous event source")?;
        loop {
            match unsafe {
                events.GetEvent(windows::Win32::Media::MediaFoundation::MF_EVENT_FLAG_NO_WAIT)
            } {
                Ok(event) => {
                    let status = unsafe { event.GetStatus() }?;
                    if status.is_err() {
                        Err::<(), _>(windows::core::Error::from_hresult(status))
                            .with_context(|| "Windows decoder event reported failure")?;
                    }
                    match unsafe { event.GetType() }? {
                        value if value == METransformNeedInput.0 as u32 => {
                            return Ok(DecoderEvent::NeedInput);
                        }
                        value if value == METransformHaveOutput.0 as u32 => {
                            return Ok(DecoderEvent::HaveOutput);
                        }
                        _ => {}
                    }
                }
                Err(error) if error.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    compio::time::sleep(Duration::from_millis(1)).await;
                }
                Err(error) => {
                    Err::<(), _>(error).with_context(|| "Failed to read Windows decoder event")?;
                }
            }
        }
    }
}

impl Drop for MfH264Decoder {
    fn drop(&mut self) {
        let _ = unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)
        };
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecoderEvent {
    NeedInput,
    HaveOutput,
}

struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn initialize() -> eros::Result<Self> {
        let status = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if status.is_ok() {
            return Ok(Self { uninitialize: true });
        }
        if status == RPC_E_CHANGED_MODE {
            return Ok(Self {
                uninitialize: false,
            });
        }
        Err::<(), _>(windows::core::Error::from_hresult(status))
            .with_context(|| "Failed to initialize COM for Windows video decoding")?;
        unreachable!()
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn create_video_device() -> eros::Result<ID3D11Device> {
    let mut device = None;
    let levels = [D3D_FEATURE_LEVEL_11_0];
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            Some(&levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            None,
        )
    }
    .with_context(|| "Failed to create D3D11 video decoder device")?;
    let device = device.with_context(|| "D3D11CreateDevice returned no decoder device")?;
    let multithread: ID3D10Multithread = device
        .cast()
        .with_context(|| "D3D11 decoder device does not expose ID3D10Multithread")?;
    unsafe {
        let _ = multithread.SetMultithreadProtected(true);
    }
    Ok(device)
}

fn create_device_manager(
    device: &ID3D11Device,
) -> eros::Result<windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager> {
    let mut token = 0;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }?;
    let manager = manager.with_context(|| "MFCreateDXGIDeviceManager returned no manager")?;
    unsafe { manager.ResetDevice(device, token) }?;
    Ok(manager)
}

fn activate_h264_decoder() -> eros::Result<IMFTransform> {
    let transform: IMFTransform =
        unsafe { CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER) }
            .with_context(|| "Failed to activate the Microsoft H.264 decoder MFT")?;
    let d3d_aware = unsafe {
        transform
            .GetAttributes()
            .and_then(|attributes| attributes.GetUINT32(&MF_SA_D3D11_AWARE))
            .unwrap_or(0)
            != 0
    };
    if !d3d_aware {
        eros::bail!("Microsoft H.264 decoder MFT is not D3D11-aware");
    }
    Ok(transform)
}

#[derive(Clone, Copy)]
struct DecoderOutputRegion {
    frame_size: PixelSize,
    source_x: u32,
    source_y: u32,
    visible_size: PixelSize,
}

fn configure_decoder_types(transform: &IMFTransform) -> eros::Result<DecoderOutputRegion> {
    let input = unsafe { MFCreateMediaType() }?;
    unsafe {
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        set_media_type_colorimetry(&input)?;
        transform.SetInputType(0, &input, 0)?;
    }
    configure_decoder_output_type(transform)
}

fn configure_decoder_output_type(transform: &IMFTransform) -> eros::Result<DecoderOutputRegion> {
    for index in 0..64 {
        let Ok(media_type) = (unsafe { transform.GetOutputAvailableType(0, index) }) else {
            break;
        };
        if unsafe { media_type.GetGUID(&MF_MT_SUBTYPE) }.ok() == Some(MFVideoFormat_NV12) {
            set_media_type_colorimetry(&media_type)?;
            unsafe { transform.SetOutputType(0, &media_type, 0) }?;
            let packed_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }
                .with_context(|| "Windows H.264 decoder output has no frame size")?;
            let frame_size = PixelSize {
                width: (packed_size >> 32) as u32,
                height: packed_size as u32,
            };
            let region = decoder_output_region(&media_type, frame_size)?;
            info!(
                event = "windows_video_decoder_output_type_configured",
                frame_width = region.frame_size.width,
                frame_height = region.frame_size.height,
                source_x = region.source_x,
                source_y = region.source_y,
                visible_width = region.visible_size.width,
                visible_height = region.visible_size.height,
                "Configured Windows decoder NV12 output region"
            );
            return Ok(region);
        }
    }
    eros::bail!("Windows H.264 decoder exposes no NV12 output type")
}

fn decoder_output_region(
    media_type: &windows::Win32::Media::MediaFoundation::IMFMediaType,
    frame_size: PixelSize,
) -> eros::Result<DecoderOutputRegion> {
    let mut aperture = MFVideoArea::default();
    let aperture_bytes = unsafe {
        std::slice::from_raw_parts_mut(
            (&mut aperture as *mut MFVideoArea).cast::<u8>(),
            std::mem::size_of::<MFVideoArea>(),
        )
    };
    if unsafe { media_type.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, aperture_bytes, None) }.is_err()
    {
        return Ok(DecoderOutputRegion {
            frame_size,
            source_x: 0,
            source_y: 0,
            visible_size: frame_size,
        });
    }
    if aperture.OffsetX.value < 0
        || aperture.OffsetY.value < 0
        || aperture.Area.cx <= 0
        || aperture.Area.cy <= 0
    {
        eros::bail!("Windows H.264 decoder returned an invalid display aperture");
    }
    let source_x = aperture.OffsetX.value as u32;
    let source_y = aperture.OffsetY.value as u32;
    let visible_size = PixelSize {
        width: aperture.Area.cx as u32,
        height: aperture.Area.cy as u32,
    };
    if source_x.saturating_add(visible_size.width) > frame_size.width
        || source_y.saturating_add(visible_size.height) > frame_size.height
    {
        eros::bail!(
            "Windows decoder display aperture {}x{}+{},{} exceeds media frame {}x{}",
            visible_size.width,
            visible_size.height,
            source_x,
            source_y,
            frame_size.width,
            frame_size.height
        );
    }
    Ok(DecoderOutputRegion {
        frame_size,
        source_x,
        source_y,
        visible_size,
    })
}

fn compressed_sample(bytes: &[u8], time: i64) -> eros::Result<IMFSample> {
    let length = u32::try_from(bytes.len()).with_context(|| "H.264 access unit exceeds u32")?;
    let buffer = unsafe { MFCreateMemoryBuffer(length) }?;
    let mut destination = ptr::null_mut();
    unsafe { buffer.Lock(&mut destination, None, None) }?;
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len()) };
    unsafe {
        buffer.Unlock()?;
        buffer.SetCurrentLength(length)?;
    }
    let sample = unsafe { MFCreateSample() }?;
    unsafe {
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(time)?;
    }
    Ok(sample)
}

fn decoded_frame(
    screen_id: ScreenId,
    sample: IMFSample,
    output_region: DecoderOutputRegion,
    probe: Option<ClientVideoFrameProbe>,
) -> eros::Result<WindowsDecodedFrame> {
    let buffer = unsafe { sample.GetBufferByIndex(0) }
        .with_context(|| "Windows decoded sample has no DXGI buffer")?;
    let dxgi: IMFDXGIBuffer = buffer
        .cast()
        .with_context(|| "Windows decoded sample is not a DXGI surface")?;
    let mut raw = ptr::null_mut();
    unsafe { dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw) }?;
    if raw.is_null() {
        eros::bail!("Windows decoded DXGI buffer returned a null D3D11 texture");
    }
    let texture = unsafe { ID3D11Texture2D::from_raw(raw) };
    let subresource_index = unsafe { dxgi.GetSubresourceIndex() }?;
    let mut desc = windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    if output_region.frame_size.width == 0
        || output_region.frame_size.height == 0
        || output_region.frame_size.width > desc.Width
        || output_region.frame_size.height > desc.Height
    {
        eros::bail!(
            "Windows decoder frame size {}x{} exceeds D3D11 texture {}x{}",
            output_region.frame_size.width,
            output_region.frame_size.height,
            desc.Width,
            desc.Height
        );
    }
    Ok(WindowsDecodedFrame {
        screen_id,
        size: output_region.visible_size,
        source_x: output_region.source_x,
        source_y: output_region.source_y,
        texture,
        subresource_index,
        _sample: SendMfSample { _sample: sample },
        probe,
    })
}

fn depayload_h264(packets: &[bytes::Bytes]) -> eros::Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut fragmented = false;
    for packet in packets {
        let payload = rtp_payload(packet)?;
        let header = *payload
            .first()
            .with_context(|| "H.264 RTP payload is empty")?;
        match header & 0x1f {
            1..=23 => {
                output.extend_from_slice(&[0, 0, 0, 1]);
                output.extend_from_slice(payload);
            }
            24 => {
                let mut remaining = &payload[1..];
                while remaining.len() >= 2 {
                    let length = usize::from(u16::from_be_bytes([remaining[0], remaining[1]]));
                    remaining = &remaining[2..];
                    let nal = remaining
                        .get(..length)
                        .with_context(|| "Truncated H.264 STAP-A NAL unit")?;
                    output.extend_from_slice(&[0, 0, 0, 1]);
                    output.extend_from_slice(nal);
                    remaining = &remaining[length..];
                }
                if !remaining.is_empty() {
                    eros::bail!("H.264 STAP-A payload has a trailing byte");
                }
            }
            28 => {
                let fu = *payload
                    .get(1)
                    .with_context(|| "H.264 FU-A header is missing")?;
                let start = fu & 0x80 != 0;
                let end = fu & 0x40 != 0;
                if start {
                    if fragmented {
                        eros::bail!("H.264 FU-A started before the previous fragment ended");
                    }
                    output.extend_from_slice(&[0, 0, 0, 1, (header & 0xe0) | (fu & 0x1f)]);
                    fragmented = true;
                } else if !fragmented {
                    eros::bail!("H.264 FU-A continuation has no start fragment");
                }
                output.extend_from_slice(&payload[2..]);
                if end {
                    fragmented = false;
                }
            }
            kind => eros::bail!("Unsupported H.264 RTP packetization type {}", kind),
        }
    }
    if fragmented {
        eros::bail!("H.264 access unit ends with an incomplete FU-A");
    }
    if output.is_empty() {
        eros::bail!("H.264 RTP frame contains no NAL units");
    }
    Ok(output)
}

fn rtp_payload(packet: &[u8]) -> eros::Result<&[u8]> {
    if packet.len() < 12 || packet[0] >> 6 != 2 {
        eros::bail!("Invalid RTP packet");
    }
    let mut offset = 12 + usize::from(packet[0] & 0x0f) * 4;
    if packet.len() < offset {
        eros::bail!("Truncated RTP CSRC list");
    }
    if packet[0] & 0x10 != 0 {
        let header = packet
            .get(offset..offset + 4)
            .with_context(|| "Truncated RTP extension header")?;
        offset += 4 + usize::from(u16::from_be_bytes([header[2], header[3]])) * 4;
    }
    let padding = if packet[0] & 0x20 != 0 {
        usize::from(
            *packet
                .last()
                .with_context(|| "RTP padding byte is missing")?,
        )
    } else {
        0
    };
    Ok(packet
        .get(offset..packet.len().saturating_sub(padding))
        .with_context(|| "Invalid RTP payload bounds")?)
}
