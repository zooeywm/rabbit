use std::marker::PhantomData;

use crate::{
    app::platform::RemoteVideoStack,
    infra::unsync_queue::UnsyncQueue,
    kernel::screen_manager::ScreenId,
    kernel::session::{ReceivedVideoFrame, SessionId},
};

pub(super) struct RunningVideoDecoder<Video>
where
    Video: RemoteVideoStack,
{
    pub(super) session_id: SessionId,
    pub(super) screen_id: ScreenId,
    pub(super) input: UnsyncQueue<VideoDecoderInput>,
    pub(super) task: Option<compio::runtime::JoinHandle<()>>,
    pub(super) video: PhantomData<Video>,
}

pub(super) enum VideoDecoderInput {
    Frame(ReceivedVideoFrame),
    Shutdown,
}

impl<Video> RunningVideoDecoder<Video>
where
    Video: RemoteVideoStack,
{
    pub(super) fn publish(&self, frame: ReceivedVideoFrame) {
        self.input.push(VideoDecoderInput::Frame(frame));
    }

    pub(super) fn matches(&self, session_id: SessionId, screen_id: ScreenId) -> bool {
        self.session_id == session_id && self.screen_id == screen_id
    }
}

impl<Video> Drop for RunningVideoDecoder<Video>
where
    Video: RemoteVideoStack,
{
    fn drop(&mut self) {
        while self.input.try_pop().is_some() {}
        self.input.push(VideoDecoderInput::Shutdown);
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}
