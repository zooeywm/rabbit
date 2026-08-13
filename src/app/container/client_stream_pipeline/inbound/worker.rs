use std::{
    sync::Arc,
    thread::{self, JoinHandle},
};

use eros::Context;

use super::{DecodeUnitSender, LatestDecodedFrameSlot, decode_unit_queue::DecodeUnitReceiver};
use crate::{
    app::container::{
        client::outbound_port::VideoDecoderState,
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

struct PipelineResourcesGuard<Input, DecodedFrame> {
    input_sender: DecodeUnitSender<Input>,
    decoded_frame_slot: Arc<LatestDecodedFrameSlot<DecodedFrame>>,
}

impl ClientStreamPipelineWorker {
    pub(crate) async fn spawn<DcdSt>(
        stream_id: StreamId,
    ) -> eros::Result<
        ClientStreamPipelineWorkerHandle<DecoderInputFor<DcdSt>, DecodedFrameFor<DcdSt>>,
    >
    where
        DcdSt: VideoDecoderState,
        DecoderInputFor<DcdSt>: VideoDecodeUnit + Send + 'static,
        DecodedBufferFor<DcdSt>: Send + 'static,
        ClientStreamPipelineContainer<DcdSt>: VideoDecoder + 'static,
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
                let _resources_guard = PipelineResourcesGuard {
                    input_sender: exit_input_sender,
                    decoded_frame_slot: exit_decoded_frame_slot,
                };

                run_client_stream_pipeline_worker(
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

impl<Input, DecodedFrame> Drop for PipelineResourcesGuard<Input, DecodedFrame> {
    fn drop(&mut self) {
        self.input_sender.close();
        self.decoded_frame_slot.close();
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
    input_receiver: DecodeUnitReceiver<DecoderInputFor<DcdSt>>,
    decoded_frame_slot: Arc<LatestDecodedFrameSlot<DecodedFrameFor<DcdSt>>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    DcdSt: VideoDecoderState,
    ClientStreamPipelineContainer<DcdSt>: VideoDecoder,
{
    let mut pipeline = ClientStreamPipelineContainer::new(DcdSt::new()?);
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
            Mutex,
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
    }

    static CALLER_THREAD_ID: Mutex<Option<thread::ThreadId>> = Mutex::new(None);
    static CONSTRUCTED_ON_WORKER: AtomicBool = AtomicBool::new(false);
    static DROPPED_ON_WORKER: AtomicBool = AtomicBool::new(false);

    impl VideoDecoderState for NonSendDecoderState {
        fn new() -> eros::Result<Self> {
            let caller_thread_id = CALLER_THREAD_ID
                .lock()
                .expect("decoder test caller thread mutex should not be poisoned")
                .expect("decoder test caller thread should be registered");
            CONSTRUCTED_ON_WORKER.store(
                thread::current().id() != caller_thread_id,
                Ordering::Relaxed,
            );
            Ok(Self {
                _not_send: Rc::new(()),
                pending: None,
            })
        }
    }

    impl Drop for NonSendDecoderState {
        fn drop(&mut self) {
            DROPPED_ON_WORKER.store(true, Ordering::Relaxed);
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

    #[test]
    fn constructs_decodes_and_drops_non_send_decoder_on_its_worker_thread() -> eros::Result<()> {
        let caller_thread_id = thread::current().id();
        *CALLER_THREAD_ID
            .lock()
            .expect("decoder test caller thread mutex should not be poisoned") =
            Some(caller_thread_id);
        CONSTRUCTED_ON_WORKER.store(false, Ordering::Relaxed);
        DROPPED_ON_WORKER.store(false, Ordering::Relaxed);
        let frame_id = FrameId::new(CaptureSourceId::new(4), 8);
        let runtime = compio::runtime::Runtime::new()?;

        let handle = runtime.block_on(ClientStreamPipelineWorker::spawn::<NonSendDecoderState>(
            StreamId::new(3),
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
        assert!(CONSTRUCTED_ON_WORKER.load(Ordering::Relaxed));
        assert!(DROPPED_ON_WORKER.load(Ordering::Relaxed));
        Ok(())
    }
}
