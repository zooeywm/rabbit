use std::{
    sync::{Arc, mpsc},
    thread::{self, JoinHandle},
};

use super::frame_queue::PacketizerFrameQueue;

use eros::Context;

use crate::{
    app::{
        container::{
            packetization::{PacketizerContainer, outbound_port::Packetizer},
            root::outbound_port::MetricsRecorder,
            stream_pipeline::outbound_port::EncodedVideoFrame,
        },
        runtime::AppMessage,
    },
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

type EncodedBufferFor<State> = <PacketizerContainer<State> as Packetizer>::EncodedBuffer;

pub(crate) struct PacketizerWorker;

pub(crate) struct PacketizerWorkerHandle<Buffer> {
    frame_queue: Arc<PacketizerFrameQueue<EncodedVideoFrame<Buffer>>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct PacketizerWorkerExitGuard<Frame> {
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    frame_queue: Arc<PacketizerFrameQueue<Frame>>,
    app_message_sender: flume::Sender<AppMessage>,
}

impl PacketizerWorker {
    pub(crate) fn spawn<State>(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        state_constructor: impl FnOnce() -> eros::Result<State> + Send + 'static,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<PacketizerWorkerHandle<EncodedBufferFor<State>>>
    where
        EncodedBufferFor<State>: Send + 'static,
        PacketizerContainer<State>: Packetizer + MetricsRecorder + 'static,
    {
        let frame_queue = Arc::new(PacketizerFrameQueue::new());
        let worker_frame_queue = Arc::clone(&frame_queue);
        let exit_frame_queue = Arc::clone(&frame_queue);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);

        let worker_thread = thread::Builder::new()
            .name(format!("packetizer-{}", stream_id.value()))
            .spawn(move || {
                let _exit_guard = PacketizerWorkerExitGuard {
                    capture_source_id,
                    stream_id,
                    frame_queue: exit_frame_queue,
                    app_message_sender,
                };

                run_packetizer_worker(
                    capture_source_id,
                    stream_id,
                    state_constructor,
                    worker_frame_queue,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn packetizer worker thread")?;

        if started_receiver.recv().is_err() {
            join_packetizer_worker(worker_thread)?;
            eros::bail!("Packetizer worker stopped before startup completed");
        }

        Ok(PacketizerWorkerHandle {
            frame_queue,
            worker_thread,
        })
    }
}

impl<Frame> Drop for PacketizerWorkerExitGuard<Frame> {
    fn drop(&mut self) {
        self.frame_queue.close();
        let _ = self
            .app_message_sender
            .send(AppMessage::StreamPipelineWorkerExited {
                capture_source_id: self.capture_source_id,
                stream_id: self.stream_id,
            });
    }
}

impl<Buffer> PacketizerWorkerHandle<Buffer> {
    pub(crate) fn submit(&self, frame: EncodedVideoFrame<Buffer>) -> eros::Result<()> {
        if !self.frame_queue.push(frame) {
            eros::bail!("Packetizer worker stopped before receiving encoded frame");
        }

        Ok(())
    }

    pub(crate) fn shutdown(self) -> eros::Result<()> {
        let Self {
            frame_queue,
            worker_thread,
        } = self;

        frame_queue.close();
        join_packetizer_worker(worker_thread)
    }
}

fn run_packetizer_worker<State>(
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    state_constructor: impl FnOnce() -> eros::Result<State>,
    frame_queue: Arc<PacketizerFrameQueue<EncodedVideoFrame<EncodedBufferFor<State>>>>,
    started_sender: mpsc::SyncSender<()>,
) -> eros::Result<()>
where
    PacketizerContainer<State>: Packetizer + MetricsRecorder,
{
    let mut packetizer =
        PacketizerContainer::new(capture_source_id, stream_id, state_constructor()?);
    packetizer.register_metrics_target();
    packetizer.register_packetizer_queue_usage(frame_queue.usage());

    let result = (|| {
        started_sender
            .send(())
            .with_context(|| "Failed to report packetizer worker startup")?;

        while let Some(frame) = frame_queue.blocking_pop() {
            Packetizer::packetize(&mut packetizer, frame)?;
        }

        Ok(())
    })();

    packetizer.unregister_metrics_target();
    result
}

fn join_packetizer_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Packetizer worker thread panicked"),
    }
}
