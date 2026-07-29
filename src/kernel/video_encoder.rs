use std::{future::Future, rc::Rc};

use crate::kernel::geometry::{FrameRate, PixelSize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VideoCodec {
    H264 = 1,
}

impl VideoCodec {
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn recommended_bitrate(self, frame_size: PixelSize, frame_rate: FrameRate) -> VideoBitrate {
        match self {
            Self::H264 => recommended_h264_bitrate(frame_size, frame_rate),
        }
    }
}

impl TryFrom<u8> for VideoCodec {
    type Error = UnknownVideoCodec;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H264),
            other => Err(UnknownVideoCodec(other)),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("unknown video codec tag {0}")]
pub struct UnknownVideoCodec(u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoBitrate(u32);

impl VideoBitrate {
    pub fn new(bits_per_second: u32) -> eros::Result<Self> {
        if bits_per_second == 0 {
            eros::bail!("Video bitrate must be positive");
        }
        Ok(Self(bits_per_second))
    }

    pub const fn bits_per_second(self) -> u32 {
        self.0
    }

    pub fn kilobits_per_second(self) -> u32 {
        self.0.div_ceil(1_000)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoEncoderParameters {
    pub codec: VideoCodec,
    pub frame_rate: FrameRate,
    pub frame_rate_mode: VideoFrameRateMode,
    pub bitrate: VideoBitrate,
    pub fec_percentage: VideoFecPercentage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VideoFecPercentage(u8);

impl VideoFecPercentage {
    pub const DEFAULT: Self = Self(15);
    pub const MINIMUM: u8 = 10;
    pub const MAXIMUM: u8 = 20;

    pub fn new(percentage: u8) -> eros::Result<Self> {
        if !(Self::MINIMUM..=Self::MAXIMUM).contains(&percentage) {
            eros::bail!(
                "Video FEC percentage must be between {} and {}, got {}",
                Self::MINIMUM,
                Self::MAXIMUM,
                percentage
            );
        }
        Ok(Self(percentage))
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VideoFrameRateMode {
    Fixed,
    #[default]
    Dynamic,
}

fn recommended_h264_bitrate(frame_size: PixelSize, frame_rate: FrameRate) -> VideoBitrate {
    // Screen-content H.264 reference: 1/6 bit per pixel per frame, rounded up
    // to whole Mbps. Keep codec-specific policy here so H.265/AV1 can use
    // independent models without changing stream configuration semantics.
    let pixels = u128::from(frame_size.width) * u128::from(frame_size.height);
    let scaled = pixels
        .saturating_mul(u128::from(frame_rate.numerator()))
        .div_ceil(u128::from(frame_rate.denominator()).saturating_mul(H264_PIXELS_PER_BIT));
    let rounded = scaled.div_ceil(BITS_PER_MEGABIT) * BITS_PER_MEGABIT;
    let bits_per_second = rounded.clamp(
        u128::from(MIN_RECOMMENDED_BITRATE_BPS),
        u128::from(MAX_RECOMMENDED_BITRATE_BPS),
    ) as u32;
    VideoBitrate(bits_per_second)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoEncoderCommand {
    RequestKeyFrame,
}

/// Runs one long-lived encoder over a stream of processed video frames.
pub trait VideoEncoder {
    type Input;
    type Packet;

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
        SendFuture: Future<Output = eros::Result<()>>;
}

const H264_PIXELS_PER_BIT: u128 = 6;
const BITS_PER_MEGABIT: u128 = 1_000_000;
const MIN_RECOMMENDED_BITRATE_BPS: u32 = 1_000_000;
const MAX_RECOMMENDED_BITRATE_BPS: u32 = 1_000_000_000;

// Focused test: cargo test kernel::video_encoder::tests --lib
#[cfg(test)]
mod tests {
    use std::{future::Future, rc::Rc};

    use futures_util::StreamExt as _;

    use crate::kernel::{
        geometry::FrameRate,
        video_encoder::{
            VideoBitrate, VideoCodec, VideoEncoder, VideoEncoderCommand, VideoEncoderParameters,
        },
    };

    struct NonCloneFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct NonClonePacket(u8);

    struct EmptyVideoEncoder;

    impl VideoEncoder for EmptyVideoEncoder {
        type Input = NonCloneFrame;
        type Packet = NonClonePacket;

        fn run<Frames, Commands, SendPacket, SendFuture>(
            frames: Frames,
            commands: Commands,
            _parameters: VideoEncoderParameters,
            _max_packet_size: usize,
            send_packet: SendPacket,
        ) -> impl Future<Output = eros::Result<()>>
        where
            Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
            Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
            SendPacket: FnMut(Vec<Self::Packet>) -> SendFuture,
            SendFuture: Future<Output = eros::Result<()>>,
        {
            drive_empty_encoder(frames, commands, send_packet)
        }
    }

    async fn drive_empty_encoder<Frames, Commands, SendPacket, SendFuture>(
        mut frames: Frames,
        mut commands: Commands,
        mut send_packet: SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<NonCloneFrame>>> + Unpin,
        Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
        SendPacket: FnMut(Vec<NonClonePacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        assert_eq!(
            commands.next().await,
            Some(VideoEncoderCommand::RequestKeyFrame)
        );
        while let Some(frame) = frames.next().await {
            let frame = frame.expect("Encoder input should contain a frame");
            send_packet(vec![NonClonePacket(frame.0)]).await?;
        }

        Ok(())
    }

    #[test]
    fn encoder_drives_a_frame_stream_into_a_packet_sink() {
        let frames = futures_util::stream::iter([Ok(Rc::new(NonCloneFrame(9)))]);
        let packets = std::cell::RefCell::new(Vec::new());
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(EmptyVideoEncoder::run(
                frames,
                futures_util::stream::iter([VideoEncoderCommand::RequestKeyFrame]),
                VideoEncoderParameters {
                    codec: VideoCodec::H264,
                    frame_rate: FrameRate::new(120, 1).expect("Test frame rate should be valid"),
                    frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode::Dynamic,
                    bitrate: VideoBitrate::new(100_000_000).expect("Test bitrate should be valid"),
                    fec_percentage: crate::kernel::video_encoder::VideoFecPercentage::DEFAULT,
                },
                1_200,
                |packet| {
                    packets.borrow_mut().extend(packet);
                    std::future::ready(Ok(()))
                },
            ))
            .expect("Encoder should drive its complete input stream");

        assert_eq!(packets.into_inner(), vec![NonClonePacket(9)]);
    }

    #[test]
    fn h264_recommendation_scales_with_resolution_and_frame_rate() {
        let codec = VideoCodec::H264;
        let sixty = FrameRate::new(60, 1).expect("fps");
        let one_twenty = FrameRate::new(120, 1).expect("fps");

        assert_eq!(
            codec
                .recommended_bitrate(
                    crate::kernel::geometry::PixelSize {
                        width: 1920,
                        height: 1080,
                    },
                    sixty,
                )
                .bits_per_second(),
            21_000_000
        );
        assert_eq!(
            codec
                .recommended_bitrate(
                    crate::kernel::geometry::PixelSize {
                        width: 2880,
                        height: 1800,
                    },
                    one_twenty,
                )
                .bits_per_second(),
            104_000_000
        );
    }
}
