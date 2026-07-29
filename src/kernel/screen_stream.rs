use std::{future::Future, marker::PhantomData, rc::Rc};

use crate::kernel::video_encoder::{VideoEncoder, VideoEncoderCommand, VideoEncoderParameters};

/// Connects one processed screen-frame stream to one long-lived video encoder.
pub struct ScreenStream<Frames, Commands, Encoder, SendPacket> {
    frames: Frames,
    commands: Commands,
    parameters: VideoEncoderParameters,
    max_packet_size: usize,
    send_packet: SendPacket,
    encoder: PhantomData<Encoder>,
}

impl<Frames, Commands, Encoder, SendPacket> ScreenStream<Frames, Commands, Encoder, SendPacket>
where
    Encoder: VideoEncoder,
    Frames: futures_core::Stream<Item = eros::Result<Rc<Encoder::Input>>> + Unpin,
    Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
{
    pub fn new(
        frames: Frames,
        commands: Commands,
        parameters: VideoEncoderParameters,
        max_packet_size: usize,
        send_packet: SendPacket,
    ) -> Self {
        Self {
            frames,
            commands,
            parameters,
            max_packet_size,
            send_packet,
            encoder: PhantomData,
        }
    }

    pub fn run<SendFuture>(self) -> impl Future<Output = eros::Result<()>>
    where
        SendPacket: FnMut(Vec<Encoder::Packet>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        Encoder::run(
            self.frames,
            self.commands,
            self.parameters,
            self.max_packet_size,
            self.send_packet,
        )
    }
}

// Focused test: cargo test kernel::screen_stream::tests --lib
#[cfg(test)]
mod tests {
    use std::{future::Future, rc::Rc};

    use futures_util::StreamExt as _;

    use crate::kernel::{
        geometry::FrameRate,
        screen_stream::ScreenStream,
        video_encoder::{
            VideoBitrate, VideoCodec, VideoEncoder, VideoEncoderCommand, VideoEncoderParameters,
        },
    };

    struct ProcessedFrame(u8);

    #[derive(Debug, PartialEq, Eq)]
    struct EncodedPacket(u8);

    struct EmptyEncoder;

    impl VideoEncoder for EmptyEncoder {
        type Input = ProcessedFrame;
        type Packet = EncodedPacket;

        fn run<Frames, Commands, SendPacket, SendFuture>(
            frames: Frames,
            commands: Commands,
            parameters: VideoEncoderParameters,
            _max_packet_size: usize,
            send_packet: SendPacket,
        ) -> impl Future<Output = eros::Result<()>>
        where
            Frames: futures_core::Stream<Item = eros::Result<Rc<Self::Input>>> + Unpin,
            Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
            SendPacket: FnMut(Vec<Self::Packet>) -> SendFuture,
            SendFuture: Future<Output = eros::Result<()>>,
        {
            assert_eq!(parameters.codec, VideoCodec::H264);
            assert_eq!(parameters.bitrate.bits_per_second(), 100_000_000);
            drive_empty_encoder(frames, commands, send_packet)
        }
    }

    async fn drive_empty_encoder<Frames, Commands, SendPacket, SendFuture>(
        mut frames: Frames,
        mut commands: Commands,
        mut send_packet: SendPacket,
    ) -> eros::Result<()>
    where
        Frames: futures_core::Stream<Item = eros::Result<Rc<ProcessedFrame>>> + Unpin,
        Commands: futures_core::Stream<Item = VideoEncoderCommand> + Unpin,
        SendPacket: FnMut(Vec<EncodedPacket>) -> SendFuture,
        SendFuture: Future<Output = eros::Result<()>>,
    {
        assert_eq!(
            commands.next().await,
            Some(VideoEncoderCommand::RequestKeyFrame)
        );
        while let Some(frame) = frames.next().await {
            let frame = frame.expect("Screen stream should contain a processed frame");
            send_packet(vec![EncodedPacket(frame.0)]).await?;
        }

        Ok(())
    }

    #[test]
    fn drives_processed_frames_through_the_selected_encoder() {
        let frames = futures_util::stream::iter([Ok(Rc::new(ProcessedFrame(11)))]);
        let packets = std::cell::RefCell::new(Vec::new());
        let stream = ScreenStream::<_, _, EmptyEncoder, _>::new(
            frames,
            futures_util::stream::iter([VideoEncoderCommand::RequestKeyFrame]),
            VideoEncoderParameters {
                codec: VideoCodec::H264,
                frame_rate: FrameRate::new(120, 1).expect("Test frame rate should be valid"),
                frame_rate_mode: crate::kernel::video_encoder::VideoFrameRateMode::Dynamic,
                bitrate: VideoBitrate::new(100_000_000).expect("Test bitrate should be valid"),
            },
            1_200,
            |packet| {
                packets.borrow_mut().extend(packet);
                std::future::ready(Ok(()))
            },
        );
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime
            .block_on(stream.run())
            .expect("Screen stream should finish normally");

        assert_eq!(packets.into_inner(), vec![EncodedPacket(11)]);
    }
}
