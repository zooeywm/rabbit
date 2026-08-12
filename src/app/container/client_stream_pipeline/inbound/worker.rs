use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

use eros::Context;

use super::{DecodeUnitSender, LatestDecodedFrameSlot, decode_unit_queue::DecodeUnitReceiver};
use crate::{
    app::container::{
        client::outbound_port::ClientEventReporter,
        client_stream_pipeline::{
            ClientStreamPipelineContainer,
            outbound_port::{DecodedVideoFrame, VideoDecodeUnit, VideoDecoder},
        },
    },
    domain::stream::models::vo::StreamId,
};

type DecoderInputFor<DcdSt> = <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecoderInput;
type DecodedBufferFor<DcdSt> =
    <ClientStreamPipelineContainer<DcdSt> as VideoDecoder>::DecodedBuffer;
type DecodedFrameFor<DcdSt> = DecodedVideoFrame<DecodedBufferFor<DcdSt>>;

pub(crate) struct ClientStreamPipelineWorker;

pub(crate) struct ClientStreamPipelineWorkerHandle<Input, DecodedFrame> {
    input_sender: DecodeUnitSender<Input>,
    decoded_frame_slot: Arc<LatestDecodedFrameSlot<DecodedFrame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct ClientStreamPipelineWorkerExitGuard<Input, DecodedFrame, EventReporter>
where
    EventReporter: ClientEventReporter,
{
    stream_id: StreamId,
    input_sender: DecodeUnitSender<Input>,
    decoded_frame_slot: Arc<LatestDecodedFrameSlot<DecodedFrame>>,
    event_reporter: EventReporter,
}

impl ClientStreamPipelineWorker {
    pub(crate) async fn spawn<DcdSt, EventReporter>(
        stream_id: StreamId,
        decoder_constructor: impl FnOnce() -> eros::Result<DcdSt> + Send + 'static,
        event_reporter: EventReporter,
    ) -> eros::Result<
        ClientStreamPipelineWorkerHandle<DecoderInputFor<DcdSt>, DecodedFrameFor<DcdSt>>,
    >
    where
        DecoderInputFor<DcdSt>: VideoDecodeUnit + Send + 'static,
        DecodedBufferFor<DcdSt>: Send + 'static,
        ClientStreamPipelineContainer<DcdSt>: VideoDecoder + 'static,
        EventReporter: ClientEventReporter,
    {
        let (input_sender, input_receiver) = DecodeUnitSender::channel();
        let decoded_frame_slot = Arc::new(LatestDecodedFrameSlot::new());
        let worker_decoded_frame_slot = Arc::clone(&decoded_frame_slot);
        let exit_decoded_frame_slot = Arc::clone(&decoded_frame_slot);
        let exit_input_sender = input_sender.clone();
        let (started_sender, started_receiver) = flume::bounded(1);

        let worker_thread = thread::Builder::new()
            .name(format!("client-stream-pipeline-{}", stream_id.value()))
            .spawn(move || {
                let _exit_guard = ClientStreamPipelineWorkerExitGuard {
                    stream_id,
                    input_sender: exit_input_sender,
                    decoded_frame_slot: exit_decoded_frame_slot,
                    event_reporter,
                };

                run_client_stream_pipeline_worker(
                    decoder_constructor,
                    input_receiver,
                    worker_decoded_frame_slot,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn Client stream pipeline worker thread")?;

        if started_receiver.recv_async().await.is_err() {
            join_client_stream_pipeline_worker(worker_thread)?;
            eros::bail!("Client stream pipeline worker stopped before startup completed");
        }

        Ok(ClientStreamPipelineWorkerHandle {
            input_sender,
            decoded_frame_slot,
            worker_thread,
        })
    }
}

impl<Input, DecodedFrame, EventReporter> Drop
    for ClientStreamPipelineWorkerExitGuard<Input, DecodedFrame, EventReporter>
where
    EventReporter: ClientEventReporter,
{
    fn drop(&mut self) {
        self.input_sender.close();
        self.decoded_frame_slot.close();
        self.event_reporter
            .report_client_stream_pipeline_worker_exited(self.stream_id);
    }
}

impl<Input, DecodedFrame> ClientStreamPipelineWorkerHandle<Input, DecodedFrame> {
    pub(crate) fn input_sender(&self) -> DecodeUnitSender<Input> {
        self.input_sender.clone()
    }

    pub(crate) fn decoded_frame_slot(&self) -> Arc<LatestDecodedFrameSlot<DecodedFrame>> {
        Arc::clone(&self.decoded_frame_slot)
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            input_sender,
            decoded_frame_slot,
            worker_thread,
        } = self;

        input_sender.close();
        let result = match compio::runtime::spawn_blocking(move || {
            join_client_stream_pipeline_worker(worker_thread)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Client stream pipeline worker join task failed"),
        };
        decoded_frame_slot.close();
        result
    }
}

fn run_client_stream_pipeline_worker<DcdSt>(
    decoder_constructor: impl FnOnce() -> eros::Result<DcdSt>,
    input_receiver: DecodeUnitReceiver<DecoderInputFor<DcdSt>>,
    decoded_frame_slot: Arc<LatestDecodedFrameSlot<DecodedFrameFor<DcdSt>>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    let mut pipeline = ClientStreamPipelineContainer::new(decoder_constructor()?);
    started_sender
        .send(())
        .with_context(|| "Failed to report Client stream pipeline worker startup")?;

    let mut decoder_generation = 0;
    while let Some((generation, input)) = input_receiver.blocking_receive() {
        if generation != decoder_generation {
            pipeline.reset()?;
            decoder_generation = generation;
        }

        pipeline.submit(input)?;
        while let Some(frame) = pipeline.try_receive()? {
            if input_receiver.generation() != generation {
                break;
            }
            if !decoded_frame_slot.replace(frame) {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn join_client_stream_pipeline_worker(
    worker_thread: JoinHandle<eros::Result<()>>,
) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Client stream pipeline worker thread panicked"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::{
        app::container::client_stream_pipeline::{
            inbound::DecodeUnitPushOutcome, outbound_port::VideoDecodeUnit,
        },
        domain::stream::models::vo::{CaptureSourceId, FrameId},
    };

    struct TestInput {
        frame_id: FrameId,
        value: u64,
    }

    impl VideoDecodeUnit for TestInput {
        fn is_recovery_point(&self) -> bool {
            true
        }
    }

    struct NonSendDecoderState {
        _not_send: Rc<()>,
        pending: Option<DecodedVideoFrame<u64>>,
        caller_thread_id: thread::ThreadId,
        dropped_on_worker: Arc<AtomicBool>,
    }

    impl Drop for NonSendDecoderState {
        fn drop(&mut self) {
            self.dropped_on_worker.store(
                thread::current().id() != self.caller_thread_id,
                Ordering::Relaxed,
            );
        }
    }

    impl VideoDecoder for ClientStreamPipelineContainer<NonSendDecoderState> {
        type DecoderInput = TestInput;
        type DecodedBuffer = u64;

        fn reset(&mut self) -> eros::Result<()> {
            self.video_decoder_state_mut().pending = None;
            Ok(())
        }

        fn submit(&mut self, input: Self::DecoderInput) -> eros::Result<()> {
            self.video_decoder_state_mut().pending =
                Some(DecodedVideoFrame::new(input.frame_id, input.value));
            Ok(())
        }

        fn try_receive(&mut self) -> eros::Result<Option<DecodedVideoFrame<Self::DecodedBuffer>>> {
            Ok(self.video_decoder_state_mut().pending.take())
        }
    }

    #[derive(Clone)]
    struct TestEventReporter;

    impl ClientEventReporter for TestEventReporter {
        fn report_client_stream_pipeline_worker_exited(&self, _stream_id: StreamId) {}
    }

    #[test]
    fn constructs_decodes_and_drops_non_send_decoder_on_its_worker_thread() -> eros::Result<()> {
        let caller_thread_id = thread::current().id();
        let constructed_on_worker = Arc::new(AtomicBool::new(false));
        let constructed_flag = Arc::clone(&constructed_on_worker);
        let dropped_on_worker = Arc::new(AtomicBool::new(false));
        let dropped_flag = Arc::clone(&dropped_on_worker);
        let frame_id = FrameId::new(CaptureSourceId::new(4), 8);
        let runtime = compio::runtime::Runtime::new()?;

        let handle = runtime.block_on(ClientStreamPipelineWorker::spawn(
            StreamId::new(3),
            move || {
                constructed_flag.store(
                    thread::current().id() != caller_thread_id,
                    Ordering::Relaxed,
                );
                Ok(NonSendDecoderState {
                    _not_send: Rc::new(()),
                    pending: None,
                    caller_thread_id,
                    dropped_on_worker: dropped_flag,
                })
            },
            TestEventReporter,
        ))?;
        let input_sender = handle.input_sender();
        let decoded_frame_slot = handle.decoded_frame_slot();
        assert!(matches!(
            input_sender.push(TestInput {
                frame_id,
                value: 42
            }),
            DecodeUnitPushOutcome::Enqueued
        ));

        let deadline = Instant::now() + Duration::from_secs(1);
        let decoded = loop {
            if let Some(decoded) = decoded_frame_slot.take_latest() {
                break decoded;
            }
            if Instant::now() >= deadline {
                eros::bail!("Client stream pipeline worker did not decode the input");
            }
            thread::yield_now();
        };
        assert!(decoded.frame_id == frame_id);
        assert_eq!(decoded.buffer, 42);

        runtime.block_on(handle.shutdown())?;
        assert!(constructed_on_worker.load(Ordering::Relaxed));
        assert!(dropped_on_worker.load(Ordering::Relaxed));
        Ok(())
    }
}
