use eros::Context;

use crate::domain::stream::models::vo::CaptureSourceId;

use super::{AppHandle, AppMessage};

pub(crate) enum CaptureOnlyMessage {
    Start {
        capture_source_id: CaptureSourceId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    Stop {
        capture_source_id: CaptureSourceId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
}

impl AppHandle {
    pub(crate) async fn start_capture_only(
        &self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::CaptureOnly(CaptureOnlyMessage::Start {
                capture_source_id,
                response_sender,
            }))
            .with_context(|| "App stopped before capture could be started")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while starting capture")?
    }

    pub(crate) async fn stop_capture_only(
        &self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::CaptureOnly(CaptureOnlyMessage::Stop {
                capture_source_id,
                response_sender,
            }))
            .with_context(|| "App stopped before capture could be stopped")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while stopping capture")?
    }
}
