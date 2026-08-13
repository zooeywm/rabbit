use std::{
    collections::HashMap,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

use eros::Context;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};

use super::{
    client_stream_control::{
        ClientStreamControl, ClientStreamControlReceiver, ClientStreamControlSender,
    },
    unit_queue::EncodedUnitReceiver,
};
use crate::app::{
    container::network::{
        NetworkContainer, NetworkMetricsHandle,
        inbound::EncodedUnitSender,
        outbound_port::{
            NetworkMetricsRecorder, SentBytes, TransporterClientSide, TransporterHostSide,
        },
    },
    runtime::AppMessage,
};

pub(crate) struct NetworkWorker;

pub(crate) struct NetworkWorkerHandle<Buffer, ClientInput> {
    sender: EncodedUnitSender<Buffer>,
    client_stream_control_sender: ClientStreamControlSender<ClientInput>,
    shutdown_sender: flume::Sender<()>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct NetworkWorkerExitGuard {
    app_message_sender: flume::Sender<AppMessage>,
}

struct NetworkQueueMetricsGuard;

impl NetworkWorker {
    pub(crate) fn spawn<State, Constructor>(
        transporter_constructor: Constructor,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<
        NetworkWorkerHandle<
            <NetworkContainer<State> as TransporterHostSide>::EncodedBuffer,
            <NetworkContainer<State> as TransporterClientSide>::Depacketized,
        >,
    >
    where
        State: 'static,
        Constructor: FnOnce() -> eros::Result<State> + Send + 'static,
        NetworkContainer<State>:
            TransporterHostSide + TransporterClientSide + NetworkMetricsRecorder,
        <NetworkContainer<State> as TransporterHostSide>::EncodedBuffer: Send + 'static,
        <NetworkContainer<State> as TransporterHostSide>::Packetized: 'static,
        <NetworkContainer<State> as TransporterClientSide>::Received: 'static,
        <NetworkContainer<State> as TransporterClientSide>::Depacketized:
            crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit,
    {
        let (sender, receiver) = EncodedUnitReceiver::channel();
        let (client_stream_control_sender, client_stream_control_receiver) =
            ClientStreamControlSender::channel();
        let (shutdown_sender, shutdown_receiver) = flume::bounded(1);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let worker_thread = thread::Builder::new()
            .name("network".to_owned())
            .spawn(move || {
                let _exit_guard = NetworkWorkerExitGuard { app_message_sender };
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for network worker")?;

                runtime.block_on(run_network_worker(
                    transporter_constructor,
                    receiver,
                    client_stream_control_receiver,
                    shutdown_receiver,
                    started_sender,
                ))
            })
            .with_context(|| "Failed to spawn network worker thread")?;

        if started_receiver.recv().is_err() {
            join_network_worker(worker_thread)?;
            eros::bail!("Network worker stopped before startup completed");
        }

        Ok(NetworkWorkerHandle {
            sender,
            client_stream_control_sender,
            shutdown_sender,
            worker_thread,
        })
    }
}

impl Drop for NetworkWorkerExitGuard {
    fn drop(&mut self) {
        let _ = self
            .app_message_sender
            .send(AppMessage::NetworkWorkerExited);
    }
}

impl<Buffer, ClientInput> NetworkWorkerHandle<Buffer, ClientInput> {
    pub(crate) fn sender(&self) -> EncodedUnitSender<Buffer> {
        self.sender.clone()
    }

    pub(crate) fn client_stream_control_sender(&self) -> ClientStreamControlSender<ClientInput> {
        self.client_stream_control_sender.clone()
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            sender: _sender,
            client_stream_control_sender: _client_stream_control_sender,
            shutdown_sender,
            worker_thread,
        } = self;
        let _ = shutdown_sender.try_send(());

        match compio::runtime::spawn_blocking(move || join_network_worker(worker_thread)).await {
            Ok(result) => result,
            Err(_) => eros::bail!("Network worker join task failed"),
        }
    }
}

async fn run_network_worker<State, Constructor>(
    transporter_constructor: Constructor,
    encoded_receiver: EncodedUnitReceiver<
        <NetworkContainer<State> as TransporterHostSide>::EncodedBuffer,
    >,
    client_stream_control_receiver: ClientStreamControlReceiver<
        <NetworkContainer<State> as TransporterClientSide>::Depacketized,
    >,
    worker_shutdown_receiver: flume::Receiver<()>,
    started_sender: mpsc::SyncSender<()>,
) -> eros::Result<()>
where
    State: 'static,
    Constructor: FnOnce() -> eros::Result<State>,
    NetworkContainer<State>:
        TransporterHostSide + TransporterClientSide + NetworkMetricsRecorder + 'static,
    <NetworkContainer<State> as TransporterHostSide>::Packetized: 'static,
    <NetworkContainer<State> as TransporterClientSide>::Received: 'static,
    <NetworkContainer<State> as TransporterClientSide>::Depacketized:
        crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit,
{
    let mut network = NetworkContainer::new(transporter_constructor()?);
    let host = TransporterHostSide::take_host(&mut network)?;
    let receiver = TransporterClientSide::take_receiver(&mut network)?;
    let (received_sender, received_receiver) = flume::bounded(1);
    let (event_shutdown_sender, event_shutdown_receiver) = flume::bounded(1);
    let (host_shutdown_sender, host_shutdown_receiver) = flume::bounded(1);
    let (receive_shutdown_sender, receive_shutdown_receiver) = flume::bounded(1);

    network.register_network_queue_usage(encoded_receiver.usage());
    let _metrics_guard = NetworkQueueMetricsGuard;
    started_sender
        .send(())
        .with_context(|| "Failed to report network worker startup")?;

    let mut tasks = FuturesUnordered::new();
    tasks.push(compio::runtime::spawn(async move {
        run_network_event_loop(
            network,
            received_receiver,
            client_stream_control_receiver,
            event_shutdown_receiver,
        )
        .await
    }));
    tasks.push(compio::runtime::spawn(async move {
        run_host_loop::<NetworkContainer<State>>(
            host,
            encoded_receiver,
            host_shutdown_receiver,
            NetworkMetricsHandle::new(),
        )
        .await
    }));
    tasks.push(compio::runtime::spawn(async move {
        run_receive_loop::<NetworkContainer<State>>(
            receiver,
            received_sender,
            receive_shutdown_receiver,
        )
        .await
    }));

    let mut first_error = None;
    let mut shutdown_started = false;
    while !tasks.is_empty() {
        let joined = if shutdown_started {
            tasks.next().await
        } else {
            let worker_shutdown = worker_shutdown_receiver.recv_async().fuse();
            let task_exit = tasks.next().fuse();
            futures_util::pin_mut!(worker_shutdown, task_exit);

            futures_util::select_biased! {
                joined = task_exit => joined,
                _ = worker_shutdown => {
                    shutdown_started = true;
                    let _ = event_shutdown_sender.try_send(());
                    let _ = host_shutdown_sender.try_send(());
                    let _ = receive_shutdown_sender.try_send(());
                    None
                },
            }
        };
        let Some(joined) = joined else {
            continue;
        };
        let result = match joined {
            Ok(result) => result,
            Err(error) => {
                tracing::error!(%error, "Network task failed to join");
                if !shutdown_started && first_error.is_none() {
                    first_error = Some(eros::error!("Network task failed to join"));
                }
                let _ = event_shutdown_sender.try_send(());
                let _ = host_shutdown_sender.try_send(());
                let _ = receive_shutdown_sender.try_send(());
                shutdown_started = true;
                continue;
            }
        };

        if let Err(error) = result {
            if !shutdown_started && first_error.is_none() {
                first_error = Some(error);
            }
        }
        shutdown_started = true;
        let _ = event_shutdown_sender.try_send(());
        let _ = host_shutdown_sender.try_send(());
        let _ = receive_shutdown_sender.try_send(());
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn run_network_event_loop<Network>(
    mut network: Network,
    received_receiver: flume::Receiver<Network::Received>,
    client_stream_control_receiver: ClientStreamControlReceiver<Network::Depacketized>,
    shutdown_receiver: flume::Receiver<()>,
) -> eros::Result<()>
where
    Network: TransporterClientSide,
    Network::Depacketized:
        crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit,
{
    let mut receive_loop_alive = true;
    let mut client_stream_control_alive = true;
    let mut client_streams = HashMap::new();

    loop {
        let receive_network = receive_loop_alive;
        let receive_client_stream_control = client_stream_control_alive;
        let client_stream_control = async {
            if receive_client_stream_control {
                client_stream_control_receiver.receive().await
            } else {
                futures_util::future::pending().await
            }
        }
        .fuse();
        let received = async {
            if receive_network {
                received_receiver.recv_async().await.ok()
            } else {
                futures_util::future::pending().await
            }
        }
        .fuse();
        let normal_event = async {
            futures_util::pin_mut!(client_stream_control, received);

            futures_util::select! {
                command = client_stream_control => {
                    let Some(command) = command else {
                        client_stream_control_alive = false;
                        return eros::Result::Ok(());
                    };
                    match command {
                        ClientStreamControl::Register {
                            stream_id,
                            input_sender,
                            response_sender,
                        } => {
                            let result = if client_streams.contains_key(&stream_id) {
                                Err(eros::error!("Client stream is already registered"))
                            } else {
                                network.request_video_refresh(stream_id)?;
                                client_streams.insert(stream_id, input_sender);
                                Ok(())
                            };
                            let _ = response_sender.send(result);
                        }
                        ClientStreamControl::Unregister {
                            stream_id,
                            response_sender,
                        } => {
                            let result = if client_streams.remove(&stream_id).is_some() {
                                Ok(())
                            } else {
                                Err(eros::error!("Client stream is not registered"))
                            };
                            let _ = response_sender.send(result);
                        }
                    }
                    eros::Result::Ok(())
                },
                item = received => {
                    let Some(item) = item else {
                        receive_loop_alive = false;
                        return eros::Result::Ok(());
                    };
                    let (stream_id, input) = network.depacketize(item)?;
                    let Some(input_sender) = client_streams.get(&stream_id) else {
                        return eros::Result::Ok(());
                    };
                    use crate::app::container::client_stream_pipeline::inbound::DecodeUnitPushOutcome;
                    match input_sender.push(input) {
                        DecodeUnitPushOutcome::Enqueued
                        | DecodeUnitPushOutcome::DroppedAwaitingRecovery => {}
                        DecodeUnitPushOutcome::Overflowed => {
                            network.request_video_refresh(stream_id)?;
                        }
                        DecodeUnitPushOutcome::Closed => {
                            client_streams.remove(&stream_id);
                        }
                    }
                    eros::Result::Ok(())
                },
            }
        }
        .fuse();
        let shutdown = shutdown_receiver.recv_async().fuse();
        futures_util::pin_mut!(shutdown, normal_event);

        futures_util::select_biased! {
            _ = shutdown => break,
            result = normal_event => result?,
        }
    }

    Ok(())
}

impl Drop for NetworkQueueMetricsGuard {
    fn drop(&mut self) {
        crate::app::container::network::NetworkMetricsHandle::new()
            .unregister_network_queue_usage();
    }
}

async fn run_host_loop<Network>(
    mut host: Network::Host,
    encoded_receiver: EncodedUnitReceiver<Network::EncodedBuffer>,
    shutdown_receiver: flume::Receiver<()>,
    metrics: impl NetworkMetricsRecorder,
) -> eros::Result<()>
where
    Network: TransporterHostSide,
{
    let shutdown = shutdown_receiver.recv_async().fuse();
    futures_util::pin_mut!(shutdown);

    loop {
        let process_one = async {
            let Some(item) = encoded_receiver.receive().await else {
                return eros::Result::Ok(false);
            };

            let frame_id = item.unit.source_frame_id;
            let packetize_started_at = Instant::now();
            let packetized = Network::packetize(&mut host, item.stream_id, item.unit)?;
            metrics.record_packetized_frame(
                frame_id.capture_source_id(),
                item.stream_id,
                frame_id,
                packetize_started_at.elapsed(),
            );

            let SentBytes {
                capture_source_id,
                stream_id,
                bytes,
            } = Network::send(&mut host, packetized).await?;
            metrics.record_sent_bytes(capture_source_id, stream_id, bytes);
            eros::Result::Ok(true)
        }
        .fuse();
        futures_util::pin_mut!(process_one);

        let keep_running = futures_util::select_biased! {
            _ = shutdown => break,
            result = process_one => result?,
        };
        if !keep_running {
            break;
        }
    }
    Ok(())
}

async fn run_receive_loop<Network>(
    mut receiver: Network::Receiver,
    received_sender: flume::Sender<Network::Received>,
    shutdown_receiver: flume::Receiver<()>,
) -> eros::Result<()>
where
    Network: TransporterClientSide,
{
    loop {
        let shutdown = shutdown_receiver.recv_async().fuse();
        let receive = Network::receive(&mut receiver).fuse();
        futures_util::pin_mut!(shutdown, receive);

        let received = futures_util::select_biased! {
            _ = shutdown => break,
            received = receive => received?,
        };
        let Some(received) = received else {
            break;
        };
        let shutdown = shutdown_receiver.recv_async().fuse();
        let forward = received_sender.send_async(received).fuse();
        futures_util::pin_mut!(shutdown, forward);
        futures_util::select_biased! {
            _ = shutdown => break,
            result = forward => {
                if result.is_err() {
                    break;
                }
            },
        }
    }
    Ok(())
}

fn join_network_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Network worker thread panicked"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        marker::PhantomData,
        rc::Rc,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Duration,
    };

    use super::*;
    use crate::{
        app::container::{
            client_stream_pipeline::outbound_port::VideoDecodeUnit,
            host_stream_pipeline::outbound_port::{EncodedVideoUnit, UnitNumber},
        },
        domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
    };

    impl VideoDecodeUnit for () {
        fn is_recovery_point(&self) -> bool {
            true
        }
    }

    struct NonSendTransporterState {
        _not_send: PhantomData<Rc<()>>,
        caller_thread_id: thread::ThreadId,
        dropped_on_network_thread: Arc<AtomicBool>,
    }

    struct CancellationTransporterState {
        host: Option<CancellationHost>,
        receiver: Option<CancellationReceiver>,
    }

    struct CancellationHost {
        packetized_sender: flume::Sender<u64>,
        gate_receiver: flume::Receiver<()>,
        send_started_sender: flume::Sender<u64>,
        sent: Arc<Mutex<Vec<u64>>>,
    }

    enum CancellationReceiver {
        Pending(flume::Sender<()>, flume::Receiver<()>),
        Fail,
    }

    impl Drop for NonSendTransporterState {
        fn drop(&mut self) {
            self.dropped_on_network_thread.store(
                thread::current().id() != self.caller_thread_id,
                Ordering::Relaxed,
            );
        }
    }

    impl TransporterHostSide
        for crate::app::container::network::NetworkContainer<CancellationTransporterState>
    {
        type EncodedBuffer = u64;
        type Packetized = u64;
        type Host = CancellationHost;

        fn take_host(&mut self) -> eros::Result<Self::Host> {
            Ok(self
                .state_mut()
                .host
                .take()
                .with_context(|| "Cancellation test host half has already been taken")?)
        }

        fn packetize(
            host: &mut Self::Host,
            _stream_id: StreamId,
            unit: EncodedVideoUnit<Self::EncodedBuffer>,
        ) -> eros::Result<Self::Packetized> {
            host.packetized_sender
                .send(unit.data)
                .with_context(|| "Cancellation test stopped observing packetized units")?;
            Ok(unit.data)
        }

        async fn send(
            host: &mut Self::Host,
            packetized: Self::Packetized,
        ) -> eros::Result<SentBytes> {
            host.send_started_sender
                .send(packetized)
                .with_context(|| "Cancellation test stopped observing started sends")?;
            host.gate_receiver
                .recv_async()
                .await
                .with_context(|| "Cancellation test send gate closed")?;
            host.sent
                .lock()
                .expect("cancellation test sent mutex should not be poisoned")
                .push(packetized);
            Ok(SentBytes::new(
                CaptureSourceId::new(0),
                StreamId::new(0),
                std::mem::size_of::<u64>(),
            ))
        }
    }

    impl TransporterClientSide
        for crate::app::container::network::NetworkContainer<CancellationTransporterState>
    {
        type Receiver = CancellationReceiver;
        type Received = ();
        type Depacketized = ();

        fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
            Ok(self
                .state_mut()
                .receiver
                .take()
                .with_context(|| "Cancellation test receiver has already been taken")?)
        }

        async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
            match receiver {
                CancellationReceiver::Pending(_sender, receiver) => {
                    Ok(receiver.recv_async().await.ok())
                }
                CancellationReceiver::Fail => {
                    eros::bail!("Intentional receive failure")
                }
            }
        }

        fn depacketize(
            &mut self,
            _received: Self::Received,
        ) -> eros::Result<(StreamId, Self::Depacketized)> {
            Ok((StreamId::new(0), ()))
        }

        fn request_video_refresh(&mut self, _stream_id: StreamId) -> eros::Result<()> {
            Ok(())
        }
    }

    fn encoded_unit(value: u64) -> EncodedVideoUnit<u64> {
        EncodedVideoUnit::new(
            FrameId::new(CaptureSourceId::new(0), value),
            UnitNumber::new(value),
            false,
            value,
        )
    }

    fn pending_cancellation_receiver() -> CancellationReceiver {
        let (sender, receiver) = flume::unbounded();
        CancellationReceiver::Pending(sender, receiver)
    }

    fn shutdown_with_timeout(worker: NetworkWorkerHandle<u64, ()>) -> eros::Result<()> {
        let (result_sender, result_receiver) = mpsc::sync_channel(1);
        let shutdown_thread = thread::spawn(move || {
            let result = match compio::runtime::Runtime::new() {
                Ok(runtime) => runtime.block_on(worker.shutdown()),
                Err(_) => Err(eros::error!("Failed to create shutdown test runtime")),
            };
            let _ = result_sender.send(result);
        });
        let result = result_receiver
            .recv_timeout(Duration::from_secs(2))
            .with_context(|| "Network worker shutdown did not complete promptly")?;
        shutdown_thread
            .join()
            .map_err(|_| eros::error!("Network worker shutdown test thread panicked"))?;
        result
    }

    impl TransporterHostSide
        for crate::app::container::network::NetworkContainer<NonSendTransporterState>
    {
        type EncodedBuffer = ();
        type Packetized = ();
        type Host = ();

        fn take_host(&mut self) -> eros::Result<Self::Host> {
            Ok(())
        }

        fn packetize(
            _host: &mut Self::Host,
            _stream_id: StreamId,
            _unit: EncodedVideoUnit<Self::EncodedBuffer>,
        ) -> eros::Result<Self::Packetized> {
            Ok(())
        }

        async fn send(
            _host: &mut Self::Host,
            _packetized: Self::Packetized,
        ) -> eros::Result<SentBytes> {
            Ok(SentBytes::new(CaptureSourceId::new(0), StreamId::new(0), 0))
        }
    }

    impl TransporterClientSide
        for crate::app::container::network::NetworkContainer<NonSendTransporterState>
    {
        type Receiver = (flume::Sender<()>, flume::Receiver<()>);
        type Received = ();
        type Depacketized = ();

        fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
            Ok(flume::unbounded())
        }

        async fn receive(receiver: &mut Self::Receiver) -> eros::Result<Option<Self::Received>> {
            Ok(receiver.1.recv_async().await.ok())
        }

        fn depacketize(
            &mut self,
            _received: Self::Received,
        ) -> eros::Result<(StreamId, Self::Depacketized)> {
            Ok((StreamId::new(0), ()))
        }

        fn request_video_refresh(&mut self, _stream_id: StreamId) -> eros::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn constructs_and_drops_non_send_transporter_on_the_network_thread() {
        let caller_thread_id = thread::current().id();
        let constructed_on_network_thread = Arc::new(AtomicBool::new(false));
        let constructed_flag = Arc::clone(&constructed_on_network_thread);
        let dropped_on_network_thread = Arc::new(AtomicBool::new(false));
        let dropped_flag = Arc::clone(&dropped_on_network_thread);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();

        let worker = NetworkWorker::spawn(
            move || {
                constructed_flag.store(
                    thread::current().id() != caller_thread_id,
                    Ordering::Relaxed,
                );
                Ok(NonSendTransporterState {
                    _not_send: PhantomData,
                    caller_thread_id,
                    dropped_on_network_thread: dropped_flag,
                })
            },
            app_message_sender,
        )
        .expect("network worker should start");

        let runtime = compio::runtime::Runtime::new().expect("test runtime should start");
        runtime
            .block_on(worker.shutdown())
            .expect("network worker should stop");
        assert!(constructed_on_network_thread.load(Ordering::Relaxed));
        assert!(dropped_on_network_thread.load(Ordering::Relaxed));
    }

    #[test]
    fn shutdown_cancels_a_blocked_in_flight_send() -> eros::Result<()> {
        let (gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, packetized_receiver) = flume::unbounded();
        let (send_started_sender, send_started_receiver) = flume::unbounded();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sent_from_network = Arc::clone(&sent);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    host: Some(CancellationHost {
                        packetized_sender,
                        gate_receiver,
                        send_started_sender,
                        sent: sent_from_network,
                    }),
                    receiver: Some(pending_cancellation_receiver()),
                })
            },
            app_message_sender,
        )?;
        let encoded_sender = worker.sender();
        encoded_sender.send(StreamId::new(0), encoded_unit(1))?;

        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 1);
        assert_eq!(
            send_started_receiver.recv_timeout(Duration::from_secs(1))?,
            1
        );
        drop(encoded_sender);
        shutdown_with_timeout(worker)?;

        assert!(
            sent.lock()
                .expect("cancellation test sent mutex should not be poisoned")
                .is_empty()
        );
        assert!(gate_sender.is_disconnected());
        Ok(())
    }

    #[test]
    fn blocked_send_does_not_packetize_the_next_encoded_unit() -> eros::Result<()> {
        let (gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, packetized_receiver) = flume::unbounded();
        let (send_started_sender, send_started_receiver) = flume::unbounded();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sent_from_network = Arc::clone(&sent);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    host: Some(CancellationHost {
                        packetized_sender,
                        gate_receiver,
                        send_started_sender,
                        sent: sent_from_network,
                    }),
                    receiver: Some(pending_cancellation_receiver()),
                })
            },
            app_message_sender,
        )?;
        let encoded_sender = worker.sender();
        encoded_sender.send(StreamId::new(0), encoded_unit(1))?;

        assert_eq!(
            send_started_receiver.recv_timeout(Duration::from_secs(1))?,
            1
        );
        encoded_sender.send(StreamId::new(0), encoded_unit(2))?;
        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 1);
        assert!(
            matches!(
                packetized_receiver.try_recv(),
                Err(flume::TryRecvError::Empty)
            ),
            "blocked host send should keep the next encoded unit before packetization",
        );
        drop(encoded_sender);

        shutdown_with_timeout(worker)?;

        assert!(
            sent.lock()
                .expect("cancellation test sent mutex should not be poisoned")
                .is_empty()
        );
        assert!(gate_sender.is_disconnected());
        Ok(())
    }

    #[test]
    fn send_failure_cancels_the_other_network_tasks_and_reaches_shutdown() -> eros::Result<()> {
        let (gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, packetized_receiver) = flume::unbounded();
        let (send_started_sender, send_started_receiver) = flume::unbounded();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    host: Some(CancellationHost {
                        packetized_sender,
                        gate_receiver,
                        send_started_sender,
                        sent,
                    }),
                    receiver: Some(pending_cancellation_receiver()),
                })
            },
            app_message_sender,
        )?;
        let encoded_sender = worker.sender();
        encoded_sender.send(StreamId::new(0), encoded_unit(1))?;

        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 1);
        assert_eq!(
            send_started_receiver.recv_timeout(Duration::from_secs(1))?,
            1
        );
        drop(gate_sender);

        let error = shutdown_with_timeout(worker).expect_err("send should fail");
        let error_debug = format!("{error:?}");
        assert!(
            error_debug.contains("Cancellation test send gate closed"),
            "unexpected send error: {error:?}"
        );
        Ok(())
    }

    #[test]
    fn receive_failure_cancels_the_other_network_tasks_and_reaches_shutdown() -> eros::Result<()> {
        let (_gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, _packetized_receiver) = flume::unbounded();
        let (send_started_sender, _send_started_receiver) = flume::unbounded();
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    host: Some(CancellationHost {
                        packetized_sender,
                        gate_receiver,
                        send_started_sender,
                        sent: Arc::new(Mutex::new(Vec::new())),
                    }),
                    receiver: Some(CancellationReceiver::Fail),
                })
            },
            app_message_sender,
        )?;

        let error = shutdown_with_timeout(worker).expect_err("receive should fail");
        assert!(format!("{error:?}").contains("Intentional receive failure"));
        Ok(())
    }

    #[test]
    fn constructor_failure_is_reported_without_leaving_a_worker_thread() {
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let result = NetworkWorker::spawn(
            || -> eros::Result<NonSendTransporterState> {
                eros::bail!("Intentional constructor failure")
            },
            app_message_sender,
        );

        assert!(result.is_err());
    }
}
