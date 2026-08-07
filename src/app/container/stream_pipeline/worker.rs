use std::{
    sync::{Arc, Weak},
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::app::{
    container::stream_pipeline::{
        LatestFrameSlot, StreamPipelineContainer,
        outbound_port::{EncoderFrameConverter, VideoEncoder},
    },
    runtime::AppCommand,
};
use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

pub(crate) struct StreamPipelineWorker;

pub(crate) struct StreamPipelineWorkerHandle<Frame> {
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct StreamPipelineWorkerExitGuard<Frame> {
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    app_command_sender: Weak<flume::Sender<AppCommand>>,
}

impl StreamPipelineWorker {
    pub(crate) async fn spawn<Frame, CvtSt, EcdSt>(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        stream_pipeline_states_constructor: impl FnOnce()
        -> eros::Result<(CvtSt, EcdSt)>
        + Send
        + 'static,
        app_command_sender: Weak<flume::Sender<AppCommand>>,
    ) -> eros::Result<StreamPipelineWorkerHandle<Frame>>
    where
        Frame: Send + 'static,
        StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter<CapturedFrame = Frame>
            + VideoEncoder<
                EncoderInput = <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput,
            >
            + 'static,
    {
        let frame_slot = Arc::new(LatestFrameSlot::new());
        let worker_frame_slot = Arc::clone(&frame_slot);
        let exit_frame_slot = Arc::clone(&frame_slot);
        let (started_sender, started_receiver) = flume::bounded(1);

        let worker_thread = thread::Builder::new()
            .name(format!("stream-pipeline-{}", stream_id.value()))
            .spawn(move || {
                let _exit_guard = StreamPipelineWorkerExitGuard {
                    capture_source_id,
                    stream_id,
                    frame_slot: exit_frame_slot,
                    app_command_sender,
                };

                run_stream_pipeline_worker(
                    stream_pipeline_states_constructor,
                    worker_frame_slot,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn stream pipeline worker thread")?;

        if started_receiver.recv_async().await.is_err() {
            join_stream_pipeline_worker(worker_thread)?;
            eros::bail!("Stream pipeline worker stopped before startup completed");
        }

        Ok(StreamPipelineWorkerHandle {
            frame_slot,
            worker_thread,
        })
    }
}

impl<Frame> Drop for StreamPipelineWorkerExitGuard<Frame> {
    fn drop(&mut self) {
        self.frame_slot.close();
        if let Some(app_command_sender) = self.app_command_sender.upgrade() {
            let _ = app_command_sender.send(AppCommand::StreamPipelineWorkerExited {
                capture_source_id: self.capture_source_id,
                stream_id: self.stream_id,
            });
        }
    }
}

impl<Frame> StreamPipelineWorkerHandle<Frame> {
    pub(crate) fn frame_slot(&self) -> Arc<LatestFrameSlot<Frame>> {
        Arc::clone(&self.frame_slot)
    }

    pub(crate) fn close(&self) {
        self.frame_slot.close();
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            frame_slot,
            worker_thread,
        } = self;

        frame_slot.close();

        match compio::runtime::spawn_blocking(move || join_stream_pipeline_worker(worker_thread))
            .await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Stream pipeline worker join task failed"),
        }
    }
}

fn run_stream_pipeline_worker<Frame, CvtSt, EcdSt>(
    stream_pipeline_states_constructor: impl FnOnce() -> eros::Result<(CvtSt, EcdSt)>,
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter<CapturedFrame = Frame>
        + VideoEncoder<
            EncoderInput = <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput,
        >,
{
    let (encoder_frame_converter_state, video_encoder_state) =
        stream_pipeline_states_constructor()?;
    let mut stream_pipeline =
        StreamPipelineContainer::new(encoder_frame_converter_state, video_encoder_state);

    started_sender
        .send(())
        .with_context(|| "Failed to report stream pipeline worker startup")?;

    while let Some(frame) = frame_slot.blocking_take() {
        let encoder_input = EncoderFrameConverter::convert(&mut stream_pipeline, frame)?;
        let _encoded_frame = VideoEncoder::encode(&mut stream_pipeline, encoder_input)?;
    }

    Ok(())
}

fn join_stream_pipeline_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Stream pipeline worker thread panicked"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        marker::PhantomData,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use super::*;
    use crate::app::container::stream_pipeline::EncodedVideoFrame;

    #[derive(Default)]
    struct NonSendConverterState(PhantomData<Rc<()>>);

    #[derive(Default)]
    struct NonSendEncoderState(PhantomData<Rc<()>>);

    impl EncoderFrameConverter for StreamPipelineContainer<NonSendConverterState, NonSendEncoderState> {
        type CapturedFrame = ();
        type EncoderInput = ();

        fn convert(&mut self, _frame: Self::CapturedFrame) -> eros::Result<Self::EncoderInput> {
            Ok(())
        }
    }

    impl VideoEncoder for StreamPipelineContainer<NonSendConverterState, NonSendEncoderState> {
        type EncoderInput = ();
        type EncodedBuffer = ();

        fn encode(
            &mut self,
            _input: Self::EncoderInput,
        ) -> eros::Result<EncodedVideoFrame<Self::EncodedBuffer>> {
            unreachable!("the test does not submit frames")
        }
    }

    #[test]
    fn creates_non_send_pipeline_states_on_worker_thread() {
        let runtime = compio::runtime::Runtime::new().expect("runtime should start");

        runtime.block_on(async {
            let caller_thread_id = thread::current().id();
            let created_on_worker = Arc::new(AtomicBool::new(false));
            let worker_flag = Arc::clone(&created_on_worker);
            let (app_command_sender, app_command_receiver) = flume::unbounded();
            let app_command_sender = Arc::new(app_command_sender);

            let worker = StreamPipelineWorker::spawn::<(), _, _>(
                CaptureSourceId::new(0),
                StreamId::new(0),
                move || {
                    worker_flag.store(
                        thread::current().id() != caller_thread_id,
                        Ordering::Relaxed,
                    );
                    Ok((
                        NonSendConverterState::default(),
                        NonSendEncoderState::default(),
                    ))
                },
                Arc::downgrade(&app_command_sender),
            )
            .await
            .expect("worker should start with non-Send pipeline states");

            assert!(created_on_worker.load(Ordering::Relaxed));

            worker.close();
            join_stream_pipeline_worker(worker.worker_thread).expect("worker should stop cleanly");

            assert!(matches!(
                app_command_receiver.recv(),
                Ok(AppCommand::StreamPipelineWorkerExited {
                    capture_source_id,
                    stream_id,
                }) if capture_source_id == CaptureSourceId::new(0)
                    && stream_id == StreamId::new(0)
            ));
        });
    }
}
