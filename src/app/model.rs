use std::{cell::RefCell, collections::HashMap, rc::Rc};

use eros::Context as _;

use crate::{
    app::App,
    infra::{
        GbmFramePipelineManagerState, KmsScreenCaptureManagerState, NiriScreenLayoutManagerState,
        PendingQuicConnectionRequest, QuicTransportSend, unsync_queue::UnsyncQueue,
    },
    kernel::{
        screen_configuration::{ScreenStreamRequestId, ScreenStreamsConfigured},
        screen_manager::ScreenId,
        session::{ReceivedVideoFrame, SessionId, SessionSend},
        session_control::ScreenInfo,
    },
};

pub(crate) type RabbitApp =
    App<NiriScreenLayoutManagerState, KmsScreenCaptureManagerState, GbmFramePipelineManagerState>;

pub(crate) struct RunningSession {
    pub(crate) send: Rc<SessionSend<QuicTransportSend>>,
    pub(crate) screen_streams: HashMap<ScreenId, RunningScreenStream>,
    pub(crate) received_video_frames: LatestVideoFrames,
    pub(crate) _receiver: compio::runtime::JoinHandle<()>,
}

#[derive(Clone, Default)]
pub(crate) struct LatestVideoFrames {
    frames: Rc<RefCell<HashMap<ScreenId, ReceivedVideoFrame>>>,
}

impl LatestVideoFrames {
    pub(crate) fn publish(&self, frame: ReceivedVideoFrame) -> bool {
        self.frames
            .borrow_mut()
            .insert(frame.screen_id, frame)
            .is_none()
    }

    pub(crate) fn take(&self, screen_id: &ScreenId) -> Option<ReceivedVideoFrame> {
        self.frames.borrow_mut().remove(screen_id)
    }
}

pub(crate) struct RunningScreenStream {
    pub(crate) id: u64,
    pub(crate) cancellation: UnsyncQueue<()>,
    pub(crate) task: Option<compio::runtime::JoinHandle<()>>,
}

impl Drop for RunningScreenStream {
    fn drop(&mut self) {
        self.cancellation.push(());

        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

pub(crate) struct ApplicationModel {
    pub(crate) requester_name: String,
    pub(crate) pending_connection_requests: Vec<PendingQuicConnectionRequest>,
    pub(crate) selected_connection_request: Option<usize>,
    pub(crate) sessions: Vec<RunningSession>,
    pub(crate) remote_screens: HashMap<SessionId, Vec<ScreenInfo>>,
    pub(crate) remote_screen_entries: Vec<(SessionId, ScreenId)>,
    pub(crate) selected_remote_screen: Option<(SessionId, ScreenId)>,
    pub(crate) screen_stream_results: HashMap<SessionId, ScreenStreamsConfigured>,
    next_session_id: u32,
    next_screen_stream_id: u64,
    next_screen_stream_request_id: u32,
    pub(crate) app: RabbitApp,
}

impl ApplicationModel {
    pub(crate) fn new(app: RabbitApp, requester_name: String) -> Self {
        Self {
            app,
            requester_name,
            pending_connection_requests: Vec::new(),
            selected_connection_request: None,
            sessions: Vec::new(),
            remote_screens: HashMap::new(),
            remote_screen_entries: Vec::new(),
            selected_remote_screen: None,
            screen_stream_results: HashMap::new(),
            next_session_id: 0,
            next_screen_stream_id: 0,
            next_screen_stream_request_id: 0,
        }
    }

    pub(crate) fn next_session_id(&mut self) -> eros::Result<SessionId> {
        let id = SessionId(self.next_session_id);
        self.next_session_id = self
            .next_session_id
            .checked_add(1)
            .context("Failed to allocate a Session ID")?;

        Ok(id)
    }

    pub(crate) fn next_screen_stream_id(&mut self) -> eros::Result<u64> {
        let id = self.next_screen_stream_id;
        self.next_screen_stream_id = self
            .next_screen_stream_id
            .checked_add(1)
            .context("Failed to allocate a screen stream task ID")?;

        Ok(id)
    }

    pub(crate) fn next_screen_stream_request_id(&mut self) -> eros::Result<ScreenStreamRequestId> {
        let id = ScreenStreamRequestId(self.next_screen_stream_request_id);
        self.next_screen_stream_request_id = self
            .next_screen_stream_request_id
            .checked_add(1)
            .context("Failed to allocate a screen stream request ID")?;

        Ok(id)
    }

    pub(crate) fn remove_session(&mut self, id: SessionId) {
        self.sessions.retain(|session| session.send.id() != id);
        self.remote_screens.remove(&id);
        self.screen_stream_results.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use crate::{
        app::model::LatestVideoFrames,
        kernel::{screen_manager::ScreenId, session::ReceivedVideoFrame},
    };

    #[test]
    fn video_queue_keeps_only_the_latest_complete_frame_per_screen() {
        let frames = LatestVideoFrames::default();
        let screen_id = ScreenId(2);

        assert!(frames.publish(ReceivedVideoFrame {
            screen_id,
            packets: vec![Bytes::from_static(b"old")],
        }));
        assert!(!frames.publish(ReceivedVideoFrame {
            screen_id,
            packets: vec![Bytes::from_static(b"new")],
        }));

        assert_eq!(
            frames
                .take(&screen_id)
                .expect("Latest video frame should remain queued")
                .packets,
            vec![Bytes::from_static(b"new")]
        );
    }
}
