use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
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
    shutdown_requested: bool,
}

struct CaptureWorkerShared<Frame> {
    state: Mutex<CaptureWorkerState<Frame>>,
}

struct CaptureWorkerExitGuard<Frame> {
    capture_source_id: CaptureSourceId,
    shared: Arc<CaptureWorkerShared<Frame>>,
    app_command_sender: Weak<flume::Sender<AppCommand>>,
}

pub(crate) struct CaptureWorker;

pub(crate) struct CaptureWorkerHandle<Frame> {
    shared: Arc<CaptureWorkerShared<Frame>>,
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
        let shared = Arc::new(CaptureWorkerShared::new(initial_stream_id, initial_frame_slot));
        let worker_shared = Arc::clone(&shared);
        let exit_shared = Arc::clone(&shared);
        let (started_sender, started_receiver) = flume::bounded(1);

        let worker_thread = thread::Builder::new()
            .name("capture".to_owned())
            .spawn(move || {
                let _exit_guard = CaptureWorkerExitGuard {
                    capture_source_id,
                    shared: exit_shared,
                    app_command_sender,
                };

                run_capture_worker::<Capturer, State>(
                    screen_capturer_state_constructor,
                    worker_shared,
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
            shared,
            control,
            worker_thread,
        })
    }
}

impl<Frame> CaptureWorkerHandle<Frame> {
    pub(crate) fn add_stream(
        &self,
        stream_id: StreamId,
        frame_slot: Arc<LatestFrameSlot<Frame>>,
    ) -> eros::Result<()> {
        self.shared.add_stream(stream_id, frame_slot)?;

        if let Err(error) = self.control.wake() {
            let _ = self.shared.remove_stream(stream_id);
            return Err(error);
        }

        Ok(())
    }

    pub(crate) fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        self.shared.remove_stream(stream_id)?;
        self.control.wake()
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            shared,
            control,
            worker_thread,
        } = self;

        shared.request_shutdown();
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
        self.shared.close_frame_slots();
        if let Some(app_command_sender) = self.app_command_sender.upgrade() {
            let _ = app_command_sender.send(AppCommand::CaptureWorkerExited {
                capture_source_id: self.capture_source_id,
            });
        }
    }
}

fn run_capture_worker<Capturer, State>(
    screen_capturer_state_constructor: impl FnOnce() -> eros::Result<State>,
    shared: Arc<CaptureWorkerShared<Capturer::CapturedFrame>>,
    started_sender: flume::Sender<Arc<dyn ScreenCapturerControl>>,
) -> eros::Result<()>
where
    Capturer: ScreenCapturer + From<State> + 'static,
{
    let screen_capturer_state = screen_capturer_state_constructor()?;
    let mut screen_capturer = Capturer::from(screen_capturer_state);
    let control = screen_capturer.control()?;
    let control_shared = Arc::clone(&shared);
    let frame_shared = Arc::clone(&shared);

    screen_capturer.run(
        shared.consumer_count(),
        move || {
            started_sender
                .send(control)
                .with_context(|| "Failed to report capture worker startup")?;
            Ok(())
        },
        move || Ok(control_shared.capture_loop_action()),
        move |frame| Ok(frame_shared.deliver_frame(frame)),
    )
}

impl<Frame> CaptureWorkerShared<Frame> {
    fn new(
        initial_stream_id: StreamId,
        initial_frame_slot: Arc<LatestFrameSlot<Frame>>,
    ) -> Self {
        Self {
            state: Mutex::new(CaptureWorkerState {
                frame_slots: HashMap::from([(initial_stream_id, initial_frame_slot)]),
                shutdown_requested: false,
            }),
        }
    }

    fn consumer_count(&self) -> usize {
        self.state
            .lock()
            .expect("capture worker state mutex poisoned")
            .frame_slots
            .len()
    }

    fn add_stream(
        &self,
        stream_id: StreamId,
        frame_slot: Arc<LatestFrameSlot<Frame>>,
    ) -> eros::Result<()> {
        let mut state = self
            .state
            .lock()
            .expect("capture worker state mutex poisoned");

        if state.shutdown_requested {
            eros::bail!("Capture worker is shutting down");
        }

        match state.frame_slots.entry(stream_id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(frame_slot);
                Ok(())
            }
            std::collections::hash_map::Entry::Occupied(_) => {
                eros::bail!("Stream already exists in capture worker")
            }
        }
    }

    fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let removed = self
            .state
            .lock()
            .expect("capture worker state mutex poisoned")
            .frame_slots
            .remove(&stream_id);

        if removed.is_some() {
            Ok(())
        } else {
            eros::bail!("Stream does not exist in capture worker")
        }
    }

    fn request_shutdown(&self) {
        self.state
            .lock()
            .expect("capture worker state mutex poisoned")
            .shutdown_requested = true;
    }

    fn capture_loop_action(&self) -> CaptureLoopAction {
        let state = self
            .state
            .lock()
            .expect("capture worker state mutex poisoned");

        if state.shutdown_requested {
            CaptureLoopAction::Stop
        } else {
            CaptureLoopAction::Continue {
                consumer_count: state.frame_slots.len(),
            }
        }
    }

    fn close_frame_slots(&self) {
        let frame_slots = self
            .state
            .lock()
            .expect("capture worker state mutex poisoned")
            .frame_slots
            .values()
            .cloned()
            .collect::<Vec<_>>();

        for frame_slot in frame_slots {
            frame_slot.close();
        }
    }
}

impl<Frame: Clone> CaptureWorkerShared<Frame> {
    fn deliver_frame(&self, frame: Frame) -> CaptureLoopAction {
        let mut state = self
            .state
            .lock()
            .expect("capture worker state mutex poisoned");

        state
            .frame_slots
            .retain(|_, frame_slot| frame_slot.replace(frame.clone()));

        if state.shutdown_requested {
            CaptureLoopAction::Stop
        } else {
            CaptureLoopAction::Continue {
                consumer_count: state.frame_slots.len(),
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
