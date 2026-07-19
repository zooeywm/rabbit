use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    pin::Pin,
    rc::{Rc, Weak},
    task::{Context, Poll, Waker},
};

use eros::Context as _;
use futures_core::Stream as _;

use crate::{
    infra::platform::{
        dma_buf::DmaBufFrame,
        frame_pipeline::worker::{
            FramePipelineId, GpuPipelineRegistration, GpuPipelineSource, GpuScreenRegistration,
            GpuWorker, GpuWorkerNotification,
        },
        screen_capture::{KmsCaptureLease, KmsFrameReceiver},
    },
    kernel::{
        frame_pipeline::{FramePipelineManager, FramePipelineParameters},
        screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
        screen_manager::ScreenId,
    },
};

mod worker;

#[derive(Debug, kudi::DepInj)]
#[target(GbmFramePipelineManager)]
pub(crate) struct GbmFramePipelineManagerState {
    sources: Rc<RefCell<HashMap<FramePipelineSourceKey, Weak<FramePipelineSource>>>>,
    captured_screens: Rc<RefCell<HashMap<ScreenId, Rc<CapturedScreenSource>>>>,
    worker: Rc<RefCell<Option<GpuWorker>>>,
    next_pipeline_id: Cell<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FramePipelineSourceKey {
    screen_id: ScreenId,
    parameters: FramePipelineParameters,
}

#[derive(Debug)]
struct FramePipelineSource {
    frames: RefCell<LatestFramePublisher<GbmFramePipelineFrame>>,
    _captured_screen: Rc<CapturedScreenSource>,
    _gpu_registration: GpuPipelineRegistration,
}

#[derive(Debug)]
struct CapturedScreenSource {
    gpu_registration: RefCell<Option<GpuScreenRegistration>>,
    lease: RefCell<Option<KmsCaptureLease>>,
}

#[derive(Debug)]
struct LatestFramePublisher<Frame> {
    subscribers: Vec<Weak<RefCell<LatestFrameSubscriptionState<Frame>>>>,
    failure: Option<Rc<str>>,
    closed: bool,
}

#[derive(Debug)]
struct LatestFrameSubscription<Frame> {
    state: Rc<RefCell<LatestFrameSubscriptionState<Frame>>>,
}

#[derive(Debug)]
struct LatestFrameSubscriptionState<Frame> {
    latest: Option<Rc<Frame>>,
    failure: Option<Rc<str>>,
    waker: Option<Waker>,
    closed: bool,
}

#[derive(Debug)]
pub(crate) struct GbmFramePipelineFrame {
    pub(crate) buffer: DmaBufFrame,
}

#[derive(Debug)]
pub(crate) struct GbmFramePipelineSubscription {
    key: FramePipelineSourceKey,
    source: Rc<FramePipelineSource>,
    frames: LatestFrameSubscription<GbmFramePipelineFrame>,
    sources: Weak<RefCell<HashMap<FramePipelineSourceKey, Weak<FramePipelineSource>>>>,
    captured_screens: Weak<RefCell<HashMap<ScreenId, Rc<CapturedScreenSource>>>>,
    worker: Weak<RefCell<Option<GpuWorker>>>,
}

impl futures_core::Stream for GbmFramePipelineSubscription {
    type Item = eros::Result<Rc<GbmFramePipelineFrame>>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.frames).poll_next(context)
    }
}

impl<Frame> Default for LatestFramePublisher<Frame> {
    fn default() -> Self {
        Self {
            subscribers: Vec::new(),
            failure: None,
            closed: false,
        }
    }
}

impl<Frame> LatestFramePublisher<Frame> {
    fn subscribe(&mut self) -> LatestFrameSubscription<Frame> {
        let state = Rc::new(RefCell::new(LatestFrameSubscriptionState {
            latest: None,
            failure: self.failure.clone(),
            waker: None,
            closed: self.closed,
        }));
        self.subscribers.push(Rc::downgrade(&state));

        LatestFrameSubscription { state }
    }

    fn publish(&mut self, frame: Frame) {
        let frame = Rc::new(frame);

        self.subscribers.retain(|subscriber| {
            let Some(state) = subscriber.upgrade() else {
                return false;
            };
            let waker = {
                let mut state = state.borrow_mut();
                state.latest = Some(Rc::clone(&frame));
                state.waker.take()
            };

            if let Some(waker) = waker {
                waker.wake();
            }

            true
        });
    }

    fn close(&mut self) {
        self.closed = true;

        for subscriber in self.subscribers.drain(..) {
            let Some(state) = subscriber.upgrade() else {
                continue;
            };
            let waker = {
                let mut state = state.borrow_mut();
                state.closed = true;
                state.waker.take()
            };

            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn fail(&mut self, failure: Rc<str>) {
        self.failure = Some(Rc::clone(&failure));
        self.closed = true;

        for subscriber in self.subscribers.drain(..) {
            let Some(state) = subscriber.upgrade() else {
                continue;
            };
            let waker = {
                let mut state = state.borrow_mut();
                state.latest = None;
                state.failure = Some(Rc::clone(&failure));
                state.closed = true;
                state.waker.take()
            };

            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

impl<Frame> futures_core::Stream for LatestFrameSubscription<Frame> {
    type Item = eros::Result<Rc<Frame>>;

    fn poll_next(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut state = self.state.borrow_mut();

        if let Some(failure) = state.failure.take() {
            return Poll::Ready(Some(failed_frame(&failure)));
        }

        if let Some(frame) = state.latest.take() {
            return Poll::Ready(Some(Ok(frame)));
        }

        if state.closed {
            return Poll::Ready(None);
        }

        match &state.waker {
            Some(waker) if waker.will_wake(context.waker()) => {}
            _ => state.waker = Some(context.waker().clone()),
        }

        Poll::Pending
    }
}

fn failed_frame<Frame>(failure: &str) -> eros::Result<Rc<Frame>> {
    Err(eros::error!("{}", failure))
}

impl CapturedScreenSource {
    fn stop(&self) {
        self.gpu_registration.borrow_mut().take();
        self.lease.borrow_mut().take();
    }
}

impl FramePipelineSource {
    fn new(captured_screen: Rc<CapturedScreenSource>, gpu_source: GpuPipelineSource) -> Rc<Self> {
        let GpuPipelineSource {
            registration,
            frames,
        } = gpu_source;
        let source = Rc::new(Self {
            frames: RefCell::new(LatestFramePublisher::default()),
            _captured_screen: captured_screen,
            _gpu_registration: registration,
        });
        let weak_source = Rc::downgrade(&source);

        compio::runtime::spawn(async move {
            while let Ok(frame) = frames.recv_async().await {
                let Some(source) = weak_source.upgrade() else {
                    return;
                };

                match frame {
                    Ok(frame) => source.frames.borrow_mut().publish(frame),
                    Err(error) => {
                        tracing::error!(%error, "GPU frame pipeline failed");
                        source
                            .frames
                            .borrow_mut()
                            .fail(format!("GPU frame pipeline failed: {error}").into());
                        return;
                    }
                }
            }

            if let Some(source) = weak_source.upgrade() {
                source.frames.borrow_mut().close();
            }
        })
        .detach();

        source
    }

    fn subscribe(
        self: &Rc<Self>,
        key: FramePipelineSourceKey,
        sources: &Rc<RefCell<HashMap<FramePipelineSourceKey, Weak<FramePipelineSource>>>>,
        captured_screens: &Rc<RefCell<HashMap<ScreenId, Rc<CapturedScreenSource>>>>,
        worker: &Rc<RefCell<Option<GpuWorker>>>,
    ) -> GbmFramePipelineSubscription {
        GbmFramePipelineSubscription {
            key,
            source: Rc::clone(self),
            frames: self.frames.borrow_mut().subscribe(),
            sources: Rc::downgrade(sources),
            captured_screens: Rc::downgrade(captured_screens),
            worker: Rc::downgrade(worker),
        }
    }
}

impl GbmFramePipelineManagerState {
    pub(crate) fn new() -> Self {
        Self {
            sources: Rc::new(RefCell::new(HashMap::new())),
            captured_screens: Rc::new(RefCell::new(HashMap::new())),
            worker: Rc::new(RefCell::new(None)),
            next_pipeline_id: Cell::new(0),
        }
    }

    fn ensure_worker(&self) -> eros::Result<()> {
        if self.worker.borrow().is_none() {
            let (worker, notifications) = GpuWorker::new()
                .with_context(|| "Failed to start the GPU frame-pipeline worker")?;
            *self.worker.borrow_mut() = Some(worker);
            self.monitor_worker(notifications);
        }

        Ok(())
    }

    fn monitor_worker(&self, notifications: flume::Receiver<GpuWorkerNotification>) {
        let sources = Rc::downgrade(&self.sources);
        let captured_screens = Rc::downgrade(&self.captured_screens);
        let worker = Rc::downgrade(&self.worker);

        compio::runtime::spawn(async move {
            while let Ok(notification) = notifications.recv_async().await {
                let (Some(sources), Some(captured_screens), Some(worker)) = (
                    sources.upgrade(),
                    captured_screens.upgrade(),
                    worker.upgrade(),
                ) else {
                    return;
                };

                handle_worker_notification(notification, &sources, &captured_screens, &worker);
            }
        })
        .detach();
    }

    fn register_screen(
        &self,
        screen_id: ScreenId,
        frames: KmsFrameReceiver,
    ) -> eros::Result<GpuScreenRegistration> {
        self.ensure_worker()?;
        let worker = self.worker.borrow();
        let Some(worker) = worker.as_ref() else {
            eros::bail!("GPU frame-pipeline worker is not available");
        };

        worker.register_screen(screen_id, frames)
    }

    fn register_pipeline(
        &self,
        screen_id: ScreenId,
        parameters: FramePipelineParameters,
    ) -> eros::Result<GpuPipelineSource> {
        self.ensure_worker()?;
        let worker = self.worker.borrow();
        let Some(worker) = worker.as_ref() else {
            eros::bail!("GPU frame-pipeline worker is not available");
        };

        let id = FramePipelineId(self.next_pipeline_id.get());
        let next_id =
            id.0.checked_add(1)
                .with_context(|| "Failed to allocate a GPU frame-pipeline ID")?;
        self.next_pipeline_id.set(next_id);

        worker.register_pipeline(id, screen_id, parameters)
    }

    #[cfg(test)]
    fn worker_thread_id(&self) -> Option<std::thread::ThreadId> {
        self.worker.borrow().as_ref().map(GpuWorker::thread_id)
    }

    fn existing_subscription(
        &self,
        key: FramePipelineSourceKey,
    ) -> Option<GbmFramePipelineSubscription> {
        let source = self.sources.borrow().get(&key)?.upgrade()?;

        Some(source.subscribe(key, &self.sources, &self.captured_screens, &self.worker))
    }

    fn insert_source(
        &self,
        key: FramePipelineSourceKey,
        source: Rc<FramePipelineSource>,
    ) -> GbmFramePipelineSubscription {
        self.sources
            .borrow_mut()
            .insert(key, Rc::downgrade(&source));

        source.subscribe(key, &self.sources, &self.captured_screens, &self.worker)
    }
}

fn handle_worker_notification(
    notification: GpuWorkerNotification,
    sources: &Rc<RefCell<HashMap<FramePipelineSourceKey, Weak<FramePipelineSource>>>>,
    captured_screens: &Rc<RefCell<HashMap<ScreenId, Rc<CapturedScreenSource>>>>,
    worker: &Rc<RefCell<Option<GpuWorker>>>,
) {
    match notification {
        GpuWorkerNotification::ScreenFailed { screen_id, error } => {
            tracing::error!(screen_id = screen_id.0, %error, "Screen capture source failed");
            let failure: Rc<str> =
                format!("Screen {} capture source failed: {error}", screen_id.0).into();
            let failed_sources = {
                let mut sources = sources.borrow_mut();
                let keys = sources
                    .keys()
                    .filter(|key| key.screen_id == screen_id)
                    .copied()
                    .collect::<Vec<_>>();

                keys.into_iter()
                    .filter_map(|key| sources.remove(&key)?.upgrade())
                    .collect::<Vec<_>>()
            };

            for source in failed_sources {
                source.frames.borrow_mut().fail(Rc::clone(&failure));
            }

            if let Some(captured_screen) = captured_screens.borrow_mut().remove(&screen_id) {
                captured_screen.stop();
            }

            if sources.borrow().is_empty() {
                let worker = worker.borrow_mut().take();
                drop(worker);
            }
        }
    }
}

impl<Deps> FramePipelineManager for GbmFramePipelineManager<Deps>
where
    Deps: AsRef<GbmFramePipelineManagerState>
        + AsMut<GbmFramePipelineManagerState>
        + ScreenCaptureManager<Lease = KmsCaptureLease, Receiver = KmsFrameReceiver>,
{
    type Frame = GbmFramePipelineFrame;
    type Subscription = GbmFramePipelineSubscription;

    fn subscribe(
        &mut self,
        screen_id: &ScreenId,
        parameters: FramePipelineParameters,
    ) -> eros::Result<Self::Subscription> {
        let key = FramePipelineSourceKey {
            screen_id: *screen_id,
            parameters,
        };

        if let Some(subscription) = self.prj_ref().existing_subscription(key) {
            return Ok(subscription);
        }

        let captured_screen = match self
            .prj_ref()
            .captured_screens
            .borrow()
            .get(screen_id)
            .cloned()
        {
            Some(captured_screen) => captured_screen,
            None => {
                let ScreenCaptureSource { lease, receiver } =
                    ScreenCaptureManager::acquire(self.prj_ref_mut(), screen_id)?;
                let gpu_registration = self.prj_ref().register_screen(*screen_id, receiver)?;

                Rc::new(CapturedScreenSource {
                    gpu_registration: RefCell::new(Some(gpu_registration)),
                    lease: RefCell::new(Some(lease)),
                })
            }
        };
        let gpu_source = self.prj_ref().register_pipeline(*screen_id, parameters)?;
        let captured_screen = self
            .prj_ref()
            .captured_screens
            .borrow_mut()
            .entry(*screen_id)
            .or_insert(captured_screen)
            .clone();
        let source = FramePipelineSource::new(captured_screen, gpu_source);

        Ok(self.prj_ref().insert_source(key, source))
    }
}

impl Drop for GbmFramePipelineSubscription {
    fn drop(&mut self) {
        if Rc::strong_count(&self.source) != 1 {
            return;
        }

        let Some(sources) = self.sources.upgrade() else {
            return;
        };
        let source = Rc::downgrade(&self.source);
        let (screen_still_used, is_empty) = {
            let mut sources = sources.borrow_mut();

            if sources
                .get(&self.key)
                .is_some_and(|registered| Weak::ptr_eq(registered, &source))
            {
                sources.remove(&self.key);
            }

            (
                sources
                    .keys()
                    .any(|key| key.screen_id == self.key.screen_id),
                sources.is_empty(),
            )
        };

        if !screen_still_used {
            if let Some(captured_screens) = self.captured_screens.upgrade() {
                captured_screens.borrow_mut().remove(&self.key.screen_id);
            }
        }

        if !is_empty {
            return;
        }

        let worker = self
            .worker
            .upgrade()
            .and_then(|worker| worker.borrow_mut().take());
        drop(worker);
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::poll_fn,
        pin::Pin,
        rc::Rc,
        task::{Context, Poll, Waker},
    };

    use futures_core::Stream;

    use crate::{
        infra::platform::{
            frame_pipeline::{
                GbmFramePipelineManager, GbmFramePipelineManagerState, LatestFramePublisher,
            },
            screen_capture::{KmsCaptureLease, KmsCapturedFrame, KmsFrameReceiver},
        },
        kernel::{
            frame_pipeline::{FramePipelineManager, FramePipelineParameters},
            geometry::PixelSize,
            screen_capture::{ScreenCaptureManager, ScreenCaptureSource},
            screen_manager::ScreenId,
        },
    };

    struct TestDeps {
        frame_pipeline: GbmFramePipelineManagerState,
        capture_acquisitions: usize,
        capture_senders: Vec<flume::Sender<eros::Result<KmsCapturedFrame>>>,
    }

    impl AsRef<GbmFramePipelineManagerState> for TestDeps {
        fn as_ref(&self) -> &GbmFramePipelineManagerState {
            &self.frame_pipeline
        }
    }

    impl AsMut<GbmFramePipelineManagerState> for TestDeps {
        fn as_mut(&mut self) -> &mut GbmFramePipelineManagerState {
            &mut self.frame_pipeline
        }
    }

    impl ScreenCaptureManager for TestDeps {
        type Lease = KmsCaptureLease;
        type Receiver = KmsFrameReceiver;

        fn acquire(
            &mut self,
            _screen_id: &ScreenId,
        ) -> eros::Result<ScreenCaptureSource<Self::Lease, Self::Receiver>> {
            self.capture_acquisitions += 1;
            let (sender, receiver) = KmsFrameReceiver::channel();
            self.capture_senders.push(sender);

            Ok(ScreenCaptureSource {
                lease: KmsCaptureLease::empty(),
                receiver,
            })
        }
    }

    #[test]
    fn subscriptions_reuse_only_an_identical_frame_pipeline_source() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let mut deps = test_deps();
            let parameters = parameters(1920, 1080);
            let mut manager = GbmFramePipelineManager::inj_ref_mut(&mut deps);
            let first = manager
                .subscribe(&ScreenId(1), parameters)
                .expect("First frame pipeline subscription should be created");
            let second = manager
                .subscribe(&ScreenId(1), parameters)
                .expect("Second frame pipeline subscription should be created");
            let different = manager
                .subscribe(&ScreenId(1), parameters(1280, 720))
                .expect("Different frame pipeline subscription should be created");

            assert!(Rc::ptr_eq(&first.source, &second.source));
            assert!(!Rc::ptr_eq(&first.source, &different.source));
            assert_eq!(manager.prj_ref().frame_pipeline.sources.borrow().len(), 2);
            assert_eq!(manager.prj_ref().capture_acquisitions, 1);

            drop(first);
            assert_eq!(manager.prj_ref().frame_pipeline.sources.borrow().len(), 2);
            drop(second);
            assert_eq!(manager.prj_ref().frame_pipeline.sources.borrow().len(), 1);
            drop(different);
            assert!(manager.prj_ref().frame_pipeline.sources.borrow().is_empty());
            assert!(
                manager
                    .prj_ref()
                    .frame_pipeline
                    .captured_screens
                    .borrow()
                    .is_empty()
            );
        });
    }

    #[test]
    fn shared_source_publishes_one_frame_to_all_subscribers() {
        let mut publisher = LatestFramePublisher::default();
        let mut first = publisher.subscribe();
        let mut second = publisher.subscribe();

        publisher.publish(7_u8);

        let first_frame = ready_frame(&mut first);
        let second_frame = ready_frame(&mut second);

        assert!(Rc::ptr_eq(&first_frame, &second_frame));
    }

    #[test]
    fn distinct_pipeline_sources_share_one_gpu_worker() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let mut deps = test_deps();
            let first = {
                let mut manager = GbmFramePipelineManager::inj_ref_mut(&mut deps);
                manager
                    .subscribe(&ScreenId(1), parameters(1920, 1080))
                    .expect("First frame pipeline subscription should be created")
            };
            let first_worker = deps
                .frame_pipeline
                .worker_thread_id()
                .expect("First frame pipeline should start the GPU worker");
            let second = {
                let mut manager = GbmFramePipelineManager::inj_ref_mut(&mut deps);
                manager
                    .subscribe(&ScreenId(1), parameters(1280, 720))
                    .expect("Second frame pipeline subscription should be created")
            };
            let second_worker = deps
                .frame_pipeline
                .worker_thread_id()
                .expect("Second frame pipeline should reuse the GPU worker");

            assert_eq!(first_worker, second_worker);
            assert_eq!(deps.capture_acquisitions, 1);

            drop(first);
            drop(second);

            assert!(deps.frame_pipeline.worker_thread_id().is_none());
        });
    }

    #[test]
    fn capture_failure_fails_every_pipeline_for_the_screen() {
        let runtime = compio::runtime::Runtime::new().expect("Compio test runtime should start");

        runtime.block_on(async {
            let mut deps = test_deps();
            let mut first = {
                let mut manager = GbmFramePipelineManager::inj_ref_mut(&mut deps);
                manager
                    .subscribe(&ScreenId(1), parameters(1920, 1080))
                    .expect("First frame pipeline subscription should be created")
            };
            let mut second = {
                let mut manager = GbmFramePipelineManager::inj_ref_mut(&mut deps);
                manager
                    .subscribe(&ScreenId(1), parameters(1280, 720))
                    .expect("Second frame pipeline subscription should be created")
            };
            let sender = deps
                .capture_senders
                .pop()
                .expect("Test capture sender should exist");

            sender
                .send(Err(eros::error!("test capture failure")))
                .expect("Capture failure should be sent");

            let first_error = poll_fn(|context| Pin::new(&mut first).poll_next(context))
                .await
                .expect("First pipeline should report its failure")
                .expect_err("First pipeline should fail");
            let second_error = poll_fn(|context| Pin::new(&mut second).poll_next(context))
                .await
                .expect("Second pipeline should report its failure")
                .expect_err("Second pipeline should fail");

            assert!(first_error.to_string().contains("test capture failure"));
            assert!(second_error.to_string().contains("test capture failure"));
            assert!(deps.frame_pipeline.sources.borrow().is_empty());
            assert!(deps.frame_pipeline.captured_screens.borrow().is_empty());
            assert!(deps.frame_pipeline.worker_thread_id().is_none());
        });
    }

    fn test_deps() -> TestDeps {
        TestDeps {
            frame_pipeline: GbmFramePipelineManagerState::new(),
            capture_acquisitions: 0,
            capture_senders: Vec::new(),
        }
    }

    fn parameters(width: u32, height: u32) -> FramePipelineParameters {
        FramePipelineParameters {
            frame_size: PixelSize { width, height },
        }
    }

    fn ready_frame<Frame>(
        subscription: &mut (impl Stream<Item = eros::Result<Rc<Frame>>> + Unpin),
    ) -> Rc<Frame> {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);

        match Pin::new(subscription).poll_next(&mut context) {
            Poll::Ready(Some(frame)) => frame.expect("Frame pipeline should publish a valid frame"),
            Poll::Ready(None) => panic!("Frame pipeline subscription should remain open"),
            Poll::Pending => panic!("Frame pipeline should have a published frame"),
        }
    }
}
