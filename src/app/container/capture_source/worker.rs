use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    sync::{Arc, Weak},
    thread::{self, JoinHandle},
};

use eros::Context;

use crate::{
    app::container::{
        LatestFrameSlot,
        capture_source::outbound_port::{
            CaptureLoopAction, ScreenCapturer, ScreenCapturerControl,
        },
    },
    app::runtime::AppCommand,
    domain::stream::models::vo::{CaptureSourceId, StreamId},
};

struct CaptureWorkerState<Frame> {
    frame_slots: HashMap<StreamId, Arc<LatestFrameSlot<Frame>>>,
}

enum CaptureCommand<Frame> {
    AddStream {
        stream_id: StreamId,
        frame_slot: Arc<LatestFrameSlot<Frame>>,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    RemoveStream {
        stream_id: StreamId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
    Shutdown,
}

struct CaptureWorkerExitGuard<Frame> {
    capture_source_id: CaptureSourceId,
    state: Rc<RefCell<CaptureWorkerState<Frame>>>,
    app_command_sender: Weak<flume::Sender<AppCommand>>,
}

pub(crate) struct CaptureWorker;

pub(crate) struct CaptureWorkerHandle<Frame> {
    command_sender: flume::Sender<CaptureCommand<Frame>>,
    control: Arc<dyn ScreenCapturerControl>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

impl CaptureWorker {
    pub(crate) async fn spawn<Capturer, State>(
        capture_source_id: CaptureSourceId,
        screen_capturer_state_constructor: impl FnOnce() -> eros::Result<State> + Send + 'static,
        initial_stream_id: StreamId,
        initial_frame_slot: Arc<LatestFrameSlot<Capturer::CapturedFrame>>,
        app_command_sender: Weak<flume::Sender<AppCommand>>,
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
                    capture_source_id,
                    screen_capturer_state_constructor,
                    initial_stream_id,
                    initial_frame_slot,
                    command_receiver,
                    app_command_sender,
                    started_sender,
                )
            })
            .with_context(|| "Failed to spawn capture worker thread")?;

        let control = match started_receiver.recv_async().await {
            Ok(control) => control,
            Err(_) => {
                join_capture_worker(worker_thread)?;
                eros::bail!("Capture worker stopped before startup completed");
            }
        };

        Ok(CaptureWorkerHandle {
            command_sender,
            control,
            worker_thread,
        })
    }
}

impl<Frame> CaptureWorkerHandle<Frame> {
    pub(crate) async fn add_stream(
        &self,
        stream_id: StreamId,
        frame_slot: Arc<LatestFrameSlot<Frame>>,
    ) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        if self
            .command_sender
            .send(CaptureCommand::AddStream {
                stream_id,
                frame_slot,
                response_sender,
            })
            .is_err()
        {
            eros::bail!("Capture worker stopped before stream could be added");
        }

        self.control.wake()?;

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

        self.control.wake()?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "Capture worker stopped while removing stream")?
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            command_sender,
            control,
            worker_thread,
        } = self;

        let send_result = if command_sender.send(CaptureCommand::Shutdown).is_ok() {
            Ok(())
        } else {
            Err(eros::error!(
                "Capture worker stopped before receiving shutdown"
            ))
        };
        let wake_result = control.wake();

        let join_result = match compio::runtime::spawn_blocking(move || {
            join_capture_worker(worker_thread)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Capture worker join task failed"),
        };

        join_result?;
        send_result?;
        wake_result
    }

    pub(crate) async fn join(self) -> eros::Result<()> {
        match compio::runtime::spawn_blocking(move || join_capture_worker(self.worker_thread)).await {
            Ok(result) => result,
            Err(_) => eros::bail!("Capture worker join task failed"),
        }
    }
}

impl<Frame> Drop for CaptureWorkerExitGuard<Frame> {
    fn drop(&mut self) {
        if let Some(app_command_sender) = self.app_command_sender.upgrade() {
            let _ = app_command_sender.send(AppCommand::CaptureWorkerExited {
                capture_source_id: self.capture_source_id,
            });
        }
        self.state.borrow().close_frame_slots();
    }
}

fn run_capture_worker<Capturer, State>(
    capture_source_id: CaptureSourceId,
    screen_capturer_state_constructor: impl FnOnce() -> eros::Result<State>,
    initial_stream_id: StreamId,
    initial_frame_slot: Arc<LatestFrameSlot<Capturer::CapturedFrame>>,
    command_receiver: flume::Receiver<CaptureCommand<Capturer::CapturedFrame>>,
    app_command_sender: Weak<flume::Sender<AppCommand>>,
    started_sender: flume::Sender<Arc<dyn ScreenCapturerControl>>,
) -> eros::Result<()>
where
    Capturer: ScreenCapturer + From<State> + 'static,
{
    let state = Rc::new(RefCell::new(CaptureWorkerState::new(
        initial_stream_id,
        initial_frame_slot,
    )));
    let _exit_guard = CaptureWorkerExitGuard {
        capture_source_id,
        state: Rc::clone(&state),
        app_command_sender,
    };
    let screen_capturer_state = screen_capturer_state_constructor()?;
    let mut screen_capturer = Capturer::from(screen_capturer_state);
    let control = screen_capturer.control()?;
    let control_state = Rc::clone(&state);
    let frame_state = Rc::clone(&state);

    screen_capturer.run(
        state.borrow().consumer_count(),
        move || {
            started_sender
                .send(control)
                .with_context(|| "Failed to report capture worker startup")?;
            Ok(())
        },
        move || {
            Ok(process_commands(
                &command_receiver,
                &mut control_state.borrow_mut(),
            ))
        },
        move |frame| Ok(frame_state.borrow_mut().deliver_frame(frame)),
    )
}

impl<Frame> CaptureWorkerState<Frame> {
    fn new(
        initial_stream_id: StreamId,
        initial_frame_slot: Arc<LatestFrameSlot<Frame>>,
    ) -> Self {
        Self {
            frame_slots: HashMap::from([(initial_stream_id, initial_frame_slot)]),
        }
    }

    fn consumer_count(&self) -> usize {
        self.frame_slots.len()
    }

    fn close_frame_slots(&self) {
        for frame_slot in self.frame_slots.values() {
            frame_slot.close();
        }
    }
}

impl<Frame: Clone> CaptureWorkerState<Frame> {
    fn deliver_frame(&mut self, frame: Frame) -> CaptureLoopAction {
        self.frame_slots
            .retain(|_, frame_slot| frame_slot.replace(frame.clone()));

        CaptureLoopAction::Continue {
            consumer_count: self.frame_slots.len(),
        }
    }
}

fn process_commands<Frame>(
    command_receiver: &flume::Receiver<CaptureCommand<Frame>>,
    state: &mut CaptureWorkerState<Frame>,
) -> CaptureLoopAction {
    loop {
        match command_receiver.try_recv() {
            Ok(CaptureCommand::AddStream {
                stream_id,
                frame_slot,
                response_sender,
            }) => {
                let result = match state.frame_slots.entry(stream_id) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(frame_slot);
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
                let result = if state.frame_slots.remove(&stream_id).is_some() {
                    Ok(())
                } else {
                    Err(eros::error!("Stream does not exist in capture worker"))
                };
                let _ = response_sender.send(result);
            }
            Ok(CaptureCommand::Shutdown) | Err(flume::TryRecvError::Disconnected) => {
                return CaptureLoopAction::Stop;
            }
            Err(flume::TryRecvError::Empty) => {
                return CaptureLoopAction::Continue {
                    consumer_count: state.frame_slots.len(),
                };
            }
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
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use super::*;

    struct NonSendCapturerState {
        _not_send: Rc<()>,
        control_sender: flume::Sender<()>,
        control_receiver: flume::Receiver<()>,
    }

    impl Default for NonSendCapturerState {
        fn default() -> Self {
            let (control_sender, control_receiver) = flume::unbounded();

            Self {
                _not_send: Rc::new(()),
                control_sender,
                control_receiver,
            }
        }
    }

    struct TestCapturer(NonSendCapturerState);

    struct TestScreenCapturerControl(flume::Sender<()>);

    impl ScreenCapturerControl for TestScreenCapturerControl {
        fn wake(&self) -> eros::Result<()> {
            Ok(self.0
                .send(())
                .with_context(|| "Test screen capturer stopped before control wakeup")?)
        }
    }

    impl From<NonSendCapturerState> for TestCapturer {
        fn from(state: NonSendCapturerState) -> Self {
            Self(state)
        }
    }

    impl ScreenCapturer for TestCapturer {
        type CapturedFrame = ();

        fn control(&self) -> eros::Result<Arc<dyn ScreenCapturerControl>> {
            Ok(Arc::new(TestScreenCapturerControl(
                self.0.control_sender.clone(),
            )))
        }

        fn run<OnStarted, OnControl, OnFrame>(
            &mut self,
            _initial_consumer_count: usize,
            on_started: OnStarted,
            mut on_control: OnControl,
            _on_frame: OnFrame,
        ) -> eros::Result<()>
        where
            OnStarted: FnOnce() -> eros::Result<()>,
            OnControl: FnMut() -> eros::Result<CaptureLoopAction>,
            OnFrame: FnMut(Self::CapturedFrame) -> eros::Result<CaptureLoopAction>,
        {
            on_started()?;
            self.0
                .control_receiver
                .recv()
                .with_context(|| "Test capture control disconnected")?;

            match on_control()? {
                CaptureLoopAction::Stop => Ok(()),
                CaptureLoopAction::Continue { .. } => {
                    eros::bail!("Test capturer expected shutdown control")
                }
            }
        }
    }

    #[test]
    fn creates_non_send_capturer_state_on_worker_thread() {
        let runtime = compio::runtime::Runtime::new().expect("runtime should start");

        runtime.block_on(async {
            let caller_thread_id = thread::current().id();
            let created_on_worker = Arc::new(AtomicBool::new(false));
            let worker_flag = Arc::clone(&created_on_worker);
            let (app_command_sender, app_command_receiver) = flume::unbounded();
            let app_command_sender = Arc::new(app_command_sender);
            let frame_slot = Arc::new(LatestFrameSlot::new());

            let worker = CaptureWorker::spawn::<TestCapturer, _>(
                CaptureSourceId::new(0),
                move || {
                    worker_flag.store(
                        thread::current().id() != caller_thread_id,
                        Ordering::Relaxed,
                    );
                    Ok(NonSendCapturerState::default())
                },
                StreamId::new(0),
                Arc::clone(&frame_slot),
                Arc::downgrade(&app_command_sender),
            )
            .await
            .expect("worker should start with a non-Send capturer state");

            assert!(created_on_worker.load(Ordering::Relaxed));

            worker.shutdown().await.expect("worker should stop cleanly");

            assert!(frame_slot.blocking_take().is_none());
            assert!(matches!(
                app_command_receiver.recv(),
                Ok(AppCommand::CaptureWorkerExited { capture_source_id })
                    if capture_source_id == CaptureSourceId::new(0)
            ));
        });
    }
}
