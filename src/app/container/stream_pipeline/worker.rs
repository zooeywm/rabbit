use std::{
    sync::{Arc, mpsc::SyncSender},
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::app::container::stream_pipeline::{
    LatestFrameSlot, StreamPipelineContainer,
    outbound_port::{EncoderFrameConverter, VideoEncoder},
};
use crate::domain::stream::models::vo::StreamId;

pub(crate) struct StreamPipelineWorker;

pub(crate) struct StreamPipelineWorkerHandle<Frame> {
    slot: Arc<LatestFrameSlot<Frame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

impl StreamPipelineWorker {
    pub(crate) fn spawn<Frame, CvtSt, EcdSt>(
        stream_id: StreamId,
        create_pipeline_states: impl FnOnce() -> eros::Result<(CvtSt, EcdSt)> + Send + 'static,
    ) -> eros::Result<StreamPipelineWorkerHandle<Frame>>
    where
        Frame: Send + 'static,
        StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter<CapturedFrame = Frame>
            + VideoEncoder<
                EncoderInput = <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput,
            >
            + 'static,
    {
        let slot = Arc::new(LatestFrameSlot::new());
        let worker_slot = Arc::clone(&slot);
        let (started_sender, started_receiver) = std::sync::mpsc::sync_channel(1);

        let worker_thread = thread::Builder::new()
            .name(format!("stream-pipeline-{}", stream_id.value()))
            .spawn(move || {
                run_stream_pipeline_worker(create_pipeline_states, worker_slot, started_sender)
            })
            .with_context(|| "Failed to spawn stream pipeline worker thread")?;

        if started_receiver.recv().is_err() {
            join_stream_pipeline_worker(worker_thread)?;
            eros::bail!("Stream pipeline worker stopped before startup completed");
        }

        Ok(StreamPipelineWorkerHandle {
            slot,
            worker_thread,
        })
    }
}

impl<Frame> StreamPipelineWorkerHandle<Frame> {
    pub(crate) fn slot(&self) -> Arc<LatestFrameSlot<Frame>> {
        Arc::clone(&self.slot)
    }

    pub(crate) fn close(&self) {
        self.slot.close();
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            slot,
            worker_thread,
        } = self;

        slot.close();

        match compio::runtime::spawn_blocking(move || join_stream_pipeline_worker(worker_thread))
            .await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Stream pipeline worker join task failed"),
        }
    }
}

fn run_stream_pipeline_worker<Frame, CvtSt, EcdSt>(
    create_pipeline_states: impl FnOnce() -> eros::Result<(CvtSt, EcdSt)>,
    slot: Arc<LatestFrameSlot<Frame>>,
    started_sender: SyncSender<()>,
) -> eros::Result<()>
where
    StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter<CapturedFrame = Frame>
        + VideoEncoder<
            EncoderInput = <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput,
        >,
{
    let (converter_state, encoder_state) = create_pipeline_states()?;
    let mut pipeline = StreamPipelineContainer::new(converter_state, encoder_state);

    started_sender
        .send(())
        .with_context(|| "Failed to report stream pipeline worker startup")?;

    while let Some(frame) = slot.blocking_take() {
        let encoder_input = EncoderFrameConverter::convert(&mut pipeline, frame)?;
        let _encoded_frame = VideoEncoder::encode(&mut pipeline, encoder_input)?;
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
        let caller_thread_id = thread::current().id();
        let created_on_worker = Arc::new(AtomicBool::new(false));
        let worker_flag = Arc::clone(&created_on_worker);

        let worker = StreamPipelineWorker::spawn::<(), _, _>(StreamId::new(0), move || {
            worker_flag.store(
                thread::current().id() != caller_thread_id,
                Ordering::Relaxed,
            );
            Ok((
                NonSendConverterState::default(),
                NonSendEncoderState::default(),
            ))
        })
        .expect("worker should start with non-Send pipeline states");

        assert!(created_on_worker.load(Ordering::Relaxed));

        worker.close();
        join_stream_pipeline_worker(worker.worker_thread).expect("worker should stop cleanly");
    }
}
