use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::app::{
    container::{
        root::outbound_port::MetricsRecorder,
        stream_pipeline::{
            StreamPipelineContainer,
            inbound::LatestFrameSlot,
            outbound_port::{EncoderFrameConverter, VideoEncoder},
        },
        transporter::inbound::EncodedUnitSender,
    },
    runtime::AppMessage,
};
use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

type PipelineFrameFor<CvtSt, EcdSt> =
    <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::CapturedFrame;

type EncoderInputFor<CvtSt, EcdSt> =
    <StreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput;

type EncodedBufferFor<CvtSt, EcdSt> =
    <StreamPipelineContainer<CvtSt, EcdSt> as VideoEncoder>::EncodedBuffer;

pub(crate) struct StreamPipelineWorker;

pub(crate) struct StreamPipelineWorkerHandle<Frame> {
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct StreamPipelineWorkerExitGuard<Frame> {
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    app_message_sender: flume::Sender<AppMessage>,
}

impl StreamPipelineWorker {
    pub(crate) async fn spawn<CvtSt, EcdSt>(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        stream_pipeline_states_constructor: impl FnOnce() -> eros::Result<(CvtSt, EcdSt)>
        + Send
        + 'static,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<StreamPipelineWorkerHandle<PipelineFrameFor<CvtSt, EcdSt>>>
    where
        PipelineFrameFor<CvtSt, EcdSt>: Send + 'static,
        EncodedBufferFor<CvtSt, EcdSt>: Send + 'static,
        StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter
            + VideoEncoder<EncoderInput = EncoderInputFor<CvtSt, EcdSt>>
            + MetricsRecorder
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
                    app_message_sender,
                };

                run_stream_pipeline_worker(
                    capture_source_id,
                    stream_id,
                    stream_pipeline_states_constructor,
                    encoded_unit_sender,
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
        let _ = self
            .app_message_sender
            .send(AppMessage::StreamPipelineWorkerExited {
                capture_source_id: self.capture_source_id,
                stream_id: self.stream_id,
            });
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

fn run_stream_pipeline_worker<CvtSt, EcdSt>(
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    stream_pipeline_states_constructor: impl FnOnce() -> eros::Result<(CvtSt, EcdSt)>,
    encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
    frame_slot: Arc<LatestFrameSlot<PipelineFrameFor<CvtSt, EcdSt>>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    StreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtSt, EcdSt>>
        + MetricsRecorder,
{
    let (encoder_frame_converter_state, video_encoder_state) =
        stream_pipeline_states_constructor()?;
    let mut stream_pipeline = StreamPipelineContainer::new(
        capture_source_id,
        stream_id,
        encoder_frame_converter_state,
        video_encoder_state,
    );
    stream_pipeline.register_metrics_target();

    let result = (|| {
        started_sender
            .send(())
            .with_context(|| "Failed to report stream pipeline worker startup")?;

        while let Some(frame) = frame_slot.blocking_take() {
            let encoder_input = EncoderFrameConverter::convert(&mut stream_pipeline, frame)?;
            let encoded_unit = VideoEncoder::encode(&mut stream_pipeline, encoder_input)?;
            encoded_unit_sender.send(stream_id, encoded_unit)?;
        }

        Ok(())
    })();

    stream_pipeline.unregister_metrics_target();
    result
}

fn join_stream_pipeline_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Stream pipeline worker thread panicked"),
    }
}
