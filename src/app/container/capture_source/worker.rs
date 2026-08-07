use std::{
    collections::HashMap,
    sync::Arc,
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        LatestFrameSlot,
        capture_source::outbound_port::{CaptureLoopAction, ScreenCapturer},
    },
    domain::stream::models::vo::StreamId,
};

enum CaptureCommand<Frame> {
    AddStream {
        stream_id: StreamId,
        slot: Arc<LatestFrameSlot<Frame>>,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    RemoveStream {
        stream_id: StreamId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    Stop,
}

pub(crate) struct CaptureWorker;

pub(crate) struct CaptureWorkerHandle<Frame> {
    command_sender: flume::Sender<CaptureCommand<Frame>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

impl CaptureWorker {
    pub(crate) async fn spawn<Capturer, State>(
        create_state: impl FnOnce() -> eros::Result<State> + Send + 'static,
        initial_stream_id: StreamId,
        initial_slot: Arc<LatestFrameSlot<Capturer::CapturedFrame>>,
    ) -> eros::Result<CaptureWorkerHandle<Capturer::CapturedFrame>>
    where
        Capturer: ScreenCapturer + From<State> + 'static,
    {
        let (command_sender, command_receiver) = flume::unbounded();
        let (started_sender, started_receiver) = flume::bounded(1);

        let worker_thread = thread::Builder::new()
            .name("capture".to_owned())
            .spawn(move || {
                run_capture_worker::<Capturer, State>(
                    create_state,
                    initial_stream_id,
                    initial_slot,
                    command_receiver,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn capture worker thread")?;

        if started_receiver.recv_async().await.is_err() {
            join_capture_worker(worker_thread)?;
            eros::bail!("Capture worker stopped before startup completed");
        }

        Ok(CaptureWorkerHandle {
            command_sender,
            worker_thread,
        })
    }
}

impl<Frame> CaptureWorkerHandle<Frame> {
    pub(crate) async fn add_stream(
        &self,
        stream_id: StreamId,
        slot: Arc<LatestFrameSlot<Frame>>,
    ) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        if self
            .command_sender
            .send(CaptureCommand::AddStream {
                stream_id,
                slot,
                response_sender,
            })
            .is_err()
        {
            eros::bail!("Capture worker stopped before stream could be added");
        }

        response_receiver
            .recv_async()
            .await
            .with_context(|| "Capture worker stopped while adding stream")?
    }

    pub(crate) async fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        if self
            .command_sender
            .send(CaptureCommand::RemoveStream {
                stream_id,
                response_sender,
            })
            .is_err()
        {
            eros::bail!("Capture worker stopped before stream could be removed");
        }

        response_receiver
            .recv_async()
            .await
            .with_context(|| "Capture worker stopped while removing stream")?
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            command_sender,
            worker_thread,
        } = self;

        let _ = command_sender.send(CaptureCommand::Stop);

        match compio::runtime::spawn_blocking(move || join_capture_worker(worker_thread)).await {
            Ok(result) => result,
            Err(_) => eros::bail!("Capture worker join task failed"),
        }
    }
}

fn run_capture_worker<Capturer, State>(
    create_state: impl FnOnce() -> eros::Result<State>,
    initial_stream_id: StreamId,
    initial_slot: Arc<LatestFrameSlot<Capturer::CapturedFrame>>,
    command_receiver: flume::Receiver<CaptureCommand<Capturer::CapturedFrame>>,
    started_sender: flume::Sender<()>,
) -> eros::Result<()>
where
    Capturer: ScreenCapturer + From<State> + 'static,
{
    let state = create_state()?;
    let mut capturer = Capturer::from(state);
    let mut slots = HashMap::from([(initial_stream_id, initial_slot)]);

    capturer.run(
        slots.len(),
        move || {
            started_sender
                .send(())
                .with_context(|| "Failed to report capture worker startup")?;
            Ok(())
        },
        move |frame| {
            if process_commands(&command_receiver, &mut slots) {
                return Ok(CaptureLoopAction::Stop);
            }

            let consumer_count = slots.len();
            let mut slot_iter = slots.values().peekable();

            while let Some(slot) = slot_iter.next() {
                if slot_iter.peek().is_some() {
                    slot.replace(frame.clone());
                } else {
                    slot.replace(frame);
                    break;
                }
            }

            if consumer_count == 0 {
                Ok(CaptureLoopAction::Stop)
            } else {
                Ok(CaptureLoopAction::Continue { consumer_count })
            }
        },
    )
}

fn process_commands<Frame>(
    command_receiver: &flume::Receiver<CaptureCommand<Frame>>,
    slots: &mut HashMap<StreamId, Arc<LatestFrameSlot<Frame>>>,
) -> bool {
    loop {
        match command_receiver.try_recv() {
            Ok(CaptureCommand::AddStream {
                stream_id,
                slot,
                response_sender,
            }) => {
                let result = match slots.entry(stream_id) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(slot);
                        Ok(())
                    }
                    std::collections::hash_map::Entry::Occupied(_) => {
                        Err(eros::error!("Stream already exists in capture worker"))
                    }
                };
                let _ = response_sender.send(result);
            }
            Ok(CaptureCommand::RemoveStream {
                stream_id,
                response_sender,
            }) => {
                let result = if slots.remove(&stream_id).is_some() {
                    Ok(())
                } else {
                    Err(eros::error!("Stream does not exist in capture worker"))
                };
                let _ = response_sender.send(result);
            }
            Ok(CaptureCommand::Stop) | Err(flume::TryRecvError::Disconnected) => return true,
            Err(flume::TryRecvError::Empty) => return false,
        }
    }
}

fn join_capture_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Capture worker thread panicked"),
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

    #[derive(Default)]
    struct NonSendCapturerState(PhantomData<Rc<()>>);

    struct TestCapturer(NonSendCapturerState);

    impl From<NonSendCapturerState> for TestCapturer {
        fn from(state: NonSendCapturerState) -> Self {
            Self(state)
        }
    }

    impl ScreenCapturer for TestCapturer {
        type CapturedFrame = ();

        fn run<OnStarted, OnFrame>(
            &mut self,
            _initial_consumer_count: usize,
            on_started: OnStarted,
            _on_frame: OnFrame,
        ) -> eros::Result<()>
        where
            OnStarted: FnOnce() -> eros::Result<()>,
            OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
        {
            on_started()
        }
    }

    #[test]
    fn creates_non_send_capturer_state_on_worker_thread() {
        let runtime = compio::runtime::Runtime::new().expect("runtime should start");

        runtime.block_on(async {
            let caller_thread_id = thread::current().id();
            let created_on_worker = Arc::new(AtomicBool::new(false));
            let worker_flag = Arc::clone(&created_on_worker);

            let worker = CaptureWorker::spawn::<TestCapturer, _>(
                move || {
                    worker_flag.store(
                        thread::current().id() != caller_thread_id,
                        Ordering::Relaxed,
                    );
                    Ok(NonSendCapturerState::default())
                },
                StreamId::new(0),
                Arc::new(LatestFrameSlot::new()),
            )
            .await
            .expect("worker should start with a non-Send capturer state");

            assert!(created_on_worker.load(Ordering::Relaxed));

            worker.shutdown().await.expect("worker should stop cleanly");
        });
    }
}
