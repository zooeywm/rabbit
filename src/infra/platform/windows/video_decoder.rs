use std::future::Future;

use eros::Context as _;
use futures_util::StreamExt as _;

use crate::kernel::{
    geometry::PixelSize, screen_manager::ScreenId, session::ReceivedVideoFrame,
    video_decoder::VideoDecoder,
};

#[derive(Debug)]
pub(crate) struct WindowsDecodedFrame {
    pub(crate) screen_id: ScreenId,
    pub(crate) size: PixelSize,
    pub(crate) payload: bytes::Bytes,
}

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
        _enable_probing: bool,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(WindowsDecodedFrame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>,
    {
        run_windows_decoder(inputs, present_frame)
    }
}

async fn run_windows_decoder<Inputs, PresentFrame, PresentFuture>(
    mut inputs: Inputs,
    mut present_frame: PresentFrame,
) -> eros::Result<()>
where
    Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
    PresentFrame: FnMut(WindowsDecodedFrame) -> PresentFuture,
    PresentFuture: Future<Output = eros::Result<()>>,
{
    while let Some(frame) = inputs.next().await {
        let frame = frame?;
        let payload = frame
            .packets
            .into_iter()
            .next()
            .with_context(|| "Windows video frame contains no packets")?;
        let decoded = decode_placeholder_packet(frame.screen_id, payload)?;
        present_frame(decoded).await?;
    }
    Ok(())
}

fn decode_placeholder_packet(
    screen_id: ScreenId,
    payload: bytes::Bytes,
) -> eros::Result<WindowsDecodedFrame> {
    const MAGIC: &[u8] = b"RABBIT-WGC\0";
    if payload.len() < MAGIC.len() + 9 || &payload[..MAGIC.len()] != MAGIC {
        eros::bail!("Windows video decoder received an unsupported packet format");
    }
    let packet_screen_id = payload[MAGIC.len()];
    if packet_screen_id != screen_id.get() {
        eros::bail!(
            "Windows packet for screen {} cannot be decoded as screen {}",
            packet_screen_id,
            screen_id.get()
        );
    }
    let width_offset = MAGIC.len() + 1;
    let height_offset = width_offset + 4;
    let width = u32::from_le_bytes(
        payload[width_offset..width_offset + 4]
            .try_into()
            .expect("width slice length is fixed"),
    );
    let height = u32::from_le_bytes(
        payload[height_offset..height_offset + 4]
            .try_into()
            .expect("height slice length is fixed"),
    );
    Ok(WindowsDecodedFrame {
        screen_id,
        size: PixelSize { width, height },
        payload,
    })
}
