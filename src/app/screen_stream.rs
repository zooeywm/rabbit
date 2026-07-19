use std::rc::Rc;

use eros::Context as _;

use crate::{
    infra::{GStreamerVideoEncoder, GbmFramePipelineFrame},
    kernel::{
        screen_manager::ScreenId,
        session::{SessionSend, VideoMessage},
        transport::TransportSend,
    },
};

pub(crate) async fn run_host_screen_stream<Frames, Send>(
    frames: Frames,
    screen_id: ScreenId,
    session: Rc<SessionSend<Send>>,
) -> eros::Result<()>
where
    Frames: futures_core::Stream<Item = eros::Result<Rc<GbmFramePipelineFrame>>> + Unpin,
    Send: TransportSend,
{
    let Some(max_packet_size) = session.max_video_packet_size() else {
        eros::bail!(
            "Session transport does not support video datagrams for screen {}",
            screen_id.0
        );
    };

    Ok(
        GStreamerVideoEncoder::run(frames, max_packet_size, move |packet| {
            let session = Rc::clone(&session);

            async move {
                session
                    .send_video(VideoMessage {
                        screen_id,
                        payload: packet.into(),
                    })
                    .await
            }
        })
        .await
        .with_context(|| format!("Failed to stream screen {}", screen_id.0))?,
    )
}
