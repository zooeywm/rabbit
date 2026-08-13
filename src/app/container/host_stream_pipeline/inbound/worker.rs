use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::app::container::{
    host::outbound_port::{EncoderFrameConverterState, MetricsRecorder, VideoEncoderState},
    host_stream_pipeline::{
        HostStreamPipelineContainer,
        inbound::LatestFrameSlot,
        outbound_port::{EncoderFrameConverter, VideoEncoder},
    },
    network::inbound::EncodedUnitSender,
};
use crate::domain::stream::models::vo::{CaptureSourceId, StreamId};

type HostPipelineFrameFor<CvtSt, EcdSt> =
    <HostStreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::CapturedFrame;

type EncoderInputFor<CvtSt, EcdSt> =
    <HostStreamPipelineContainer<CvtSt, EcdSt> as EncoderFrameConverter>::EncoderInput;

type EncodedBufferFor<CvtSt, EcdSt> =
    <HostStreamPipelineContainer<CvtSt, EcdSt> as VideoEncoder>::EncodedBuffer;

pub(crate) struct HostStreamPipelineWorker;

pub(crate) struct HostStreamPipelineWorkerHandle<Frame> {
    frame_slot: Arc<LatestFrameSlot<Frame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct FrameSlotGuard<Frame> {
    frame_slot: Arc<LatestFrameSlot<Frame>>,
}

impl HostStreamPipelineWorker {
    pub(crate) async fn spawn<CvtSt, EcdSt>(
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
    ) -> eros::Result<HostStreamPipelineWorkerHandle<HostPipelineFrameFor<CvtSt, EcdSt>>>
    where
        CvtSt: EncoderFrameConverterState,
        EcdSt: VideoEncoderState,
        HostPipelineFrameFor<CvtSt, EcdSt>: Send + 'static,
        EncodedBufferFor<CvtSt, EcdSt>: Send + 'static,
        HostStreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter
            + VideoEncoder<EncoderInput = EncoderInputFor<CvtSt, EcdSt>>
            + MetricsRecorder
            + 'static,
    {
        let frame_slot = Arc::new(LatestFrameSlot::new());
        let worker_frame_slot = Arc::clone(&frame_slot);
        let exit_frame_slot = Arc::clone(&frame_slot);
        let (started_sender, started_receiver) = flume::bounded(1);

        let worker_thread = thread::Builder::new()
            .name(format!("host-stream-pipeline-{}", stream_id.value()))
            .spawn(move || {
                let _frame_slot_guard = FrameSlotGuard {
                    frame_slot: exit_frame_slot,
                };

                run_host_stream_pipeline_worker(
                    capture_source_id,
                    stream_id,
                    encoded_unit_sender,
                    worker_frame_slot,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn Host stream pipeline worker thread")?;

        if started_receiver.recv_async().await.is_err() {
            join_host_stream_pipeline_worker(worker_thread)?;
            eros::bail!("Host stream pipeline worker stopped before startup completed");
        }

        Ok(HostStreamPipelineWorkerHandle {
            frame_slot,
            worker_thread,
        })
    }
}

impl<Frame> Drop for FrameSlotGuard<Frame> {
    fn drop(&mut self) {
        self.frame_slot.close();
    }
}

impl<Frame> HostStreamPipelineWorkerHandle<Frame> {
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

        match compio::runtime::spawn_blocking(move || {
            join_host_stream_pipeline_worker(worker_thread)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Host stream pipeline worker join task failed"),
        }
    }
}

fn run_host_stream_pipeline_worker<CvtSt, EcdSt>(
    capture_source_id: CaptureSourceId,
    stream_id: StreamId,
    encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtSt, EcdSt>>,
    frame_slot: Arc<LatestFrameSlot<HostPipelineFrameFor<CvtSt, EcdSt>>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    CvtSt: EncoderFrameConverterState,
    EcdSt: VideoEncoderState,
    HostStreamPipelineContainer<CvtSt, EcdSt>: EncoderFrameConverter
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtSt, EcdSt>>
        + MetricsRecorder,
{
    let mut host_stream_pipeline = HostStreamPipelineContainer::new(
        capture_source_id,
        stream_id,
        CvtSt::new()?,
        EcdSt::new()?,
    );
    host_stream_pipeline.register_metrics_target();

    let result = (|| {
        started_sender
            .send(())
            .with_context(|| "Failed to report Host stream pipeline worker startup")?;

        while let Some(frame) = frame_slot.blocking_take() {
            let encoder_input = EncoderFrameConverter::convert(&mut host_stream_pipeline, frame)?;
            let encoded_unit = VideoEncoder::encode(&mut host_stream_pipeline, encoder_input)?;
            encoded_unit_sender.send(stream_id, encoded_unit)?;
        }

        Ok(())
    })();

    host_stream_pipeline.unregister_metrics_target();
    result
}

fn join_host_stream_pipeline_worker(
    worker_thread: JoinHandle<eros::Result<()>>,
) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Host stream pipeline worker thread panicked"),
    }
}
