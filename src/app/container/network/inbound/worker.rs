use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};

use super::{
    client_event_queue::{NetworkClientEventReceiver, NetworkClientEventSender},
    packetized_send_queue::{
        PacketizePermitReceiver, PacketizedSendReceiver, PacketizedSendSender,
    },
    unit_queue::EncodedUnitReceiver,
};
use crate::app::{
    container::network::{
        NetworkContainer, NetworkMetricsHandle,
        inbound::{EncodedUnitSender, NetworkClientEvent},
        outbound_port::{
            NetworkMetricsRecorder, SentBytes, TransporterClientSide, TransporterHostSide,
        },
    },
    runtime::AppMessage,
};

pub(crate) struct NetworkWorker;

pub(crate) struct NetworkWorkerHandle<Buffer, ClientInput> {
    sender: EncodedUnitSender<Buffer>,
    client_event_receiver: Option<NetworkClientEventReceiver<ClientInput>>,
    shutdown_sender: flume::Sender<()>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct NetworkWorkerExitGuard {
    app_message_sender: flume::Sender<AppMessage>,
}

struct NetworkQueueMetricsGuard;

enum NetworkEventLoopStep {
    Continue,
    Stop,
}

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
    {
        let (sender, receiver) = EncodedUnitReceiver::channel();
        let (client_event_sender, client_event_receiver) = NetworkClientEventSender::channel();
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
                    client_event_sender,
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
            client_event_receiver: Some(client_event_receiver),
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

    pub(crate) fn take_client_event_receiver(
        &mut self,
    ) -> eros::Result<NetworkClientEventReceiver<ClientInput>> {
        Ok(self
            .client_event_receiver
            .take()
            .with_context(|| "Network client event receiver has already been taken")?)
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            sender: _sender,
            client_event_receiver: _client_event_receiver,
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
    client_event_sender: NetworkClientEventSender<
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
{
    let mut network = NetworkContainer::new(transporter_constructor()?);
    let sender = TransporterHostSide::take_sender(&mut network)?;
    let receiver = TransporterClientSide::take_receiver(&mut network)?;
    let (packetized_sender, packetized_receiver, packetize_permits) =
        PacketizedSendSender::channel();
    let (received_sender, received_receiver) = flume::bounded(1);
    let (event_shutdown_sender, event_shutdown_receiver) = flume::bounded(1);
    let (send_shutdown_sender, send_shutdown_receiver) = flume::bounded(1);
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
            encoded_receiver,
            received_receiver,
            packetized_sender,
            packetize_permits,
            client_event_sender,
            event_shutdown_receiver,
        )
        .await
    }));
    tasks.push(compio::runtime::spawn(async move {
        run_send_loop::<NetworkContainer<State>>(
            sender,
            packetized_receiver,
            send_shutdown_receiver,
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
                    let _ = send_shutdown_sender.try_send(());
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
                let _ = send_shutdown_sender.try_send(());
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
        let _ = send_shutdown_sender.try_send(());
        let _ = receive_shutdown_sender.try_send(());
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn run_network_event_loop<Network>(
    mut network: Network,
    encoded_receiver: EncodedUnitReceiver<Network::EncodedBuffer>,
    received_receiver: flume::Receiver<Network::Received>,
    packetized_sender: PacketizedSendSender<Network::Packetized>,
    packetize_permits: PacketizePermitReceiver,
    client_event_sender: NetworkClientEventSender<Network::Depacketized>,
    shutdown_receiver: flume::Receiver<()>,
) -> eros::Result<()>
where
    Network: TransporterHostSide + TransporterClientSide,
{
    let mut may_packetize = false;
    let mut may_route_client_event = false;
    let mut send_loop_alive = true;
    let mut receive_loop_alive = true;
    let mut client_event_sink_alive = true;

    loop {
        let wait_for_packetize_permit = send_loop_alive && !may_packetize;
        let wait_for_client_event_permit = client_event_sink_alive && !may_route_client_event;
        let receive_encoded = send_loop_alive && may_packetize;
        let receive_network =
            receive_loop_alive && client_event_sink_alive && may_route_client_event;
        let packetize_permit = async {
            if wait_for_packetize_permit {
                packetize_permits.acquire().await
            } else {
                futures_util::future::pending().await
            }
        }
        .fuse();
        let client_event_permit = async {
            if wait_for_client_event_permit {
                client_event_sender.acquire_permit().await
            } else {
                futures_util::future::pending().await
            }
        }
        .fuse();
        let encoded = async {
            if receive_encoded {
                encoded_receiver.receive().await
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
            futures_util::pin_mut!(packetize_permit, client_event_permit, encoded, received);

            futures_util::select! {
                permit = packetize_permit => {
                    match permit {
                        Ok(()) => may_packetize = true,
                        Err(_) => {
                            send_loop_alive = false;
                            may_packetize = false;
                        }
                    }
                    eros::Result::Ok(NetworkEventLoopStep::Continue)
                },
                permit = client_event_permit => {
                    match permit {
                        Ok(()) => may_route_client_event = true,
                        Err(_) => {
                            client_event_sink_alive = false;
                            may_route_client_event = false;
                        }
                    }
                    eros::Result::Ok(NetworkEventLoopStep::Continue)
                },
                item = encoded => {
                    let Some(item) = item else {
                        return eros::Result::Ok(NetworkEventLoopStep::Stop);
                    };
                    let packetized = network.packetize(item.stream_id, item.unit)?;
                    packetized_sender.send(packetized)?;
                    may_packetize = false;
                    eros::Result::Ok(NetworkEventLoopStep::Continue)
                },
                item = received => {
                    let Some(item) = item else {
                        receive_loop_alive = false;
                        return eros::Result::Ok(NetworkEventLoopStep::Continue);
                    };
                    let (stream_id, input) = network.depacketize(item)?;
                    client_event_sender.send(NetworkClientEvent { stream_id, input })?;
                    may_route_client_event = false;
                    eros::Result::Ok(NetworkEventLoopStep::Continue)
                },
            }
        }
        .fuse();
        let shutdown = shutdown_receiver.recv_async().fuse();
        futures_util::pin_mut!(shutdown, normal_event);

        let step = futures_util::select_biased! {
            _ = shutdown => break,
            step = normal_event => step?,
        };
        if matches!(step, NetworkEventLoopStep::Stop) {
            break;
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

async fn run_send_loop<Network>(
    mut sender: Network::Sender,
    packetized_receiver: PacketizedSendReceiver<Network::Packetized>,
    shutdown_receiver: flume::Receiver<()>,
    metrics: impl NetworkMetricsRecorder,
) -> eros::Result<()>
where
    Network: TransporterHostSide,
{
    loop {
        let shutdown = shutdown_receiver.recv_async().fuse();
        let receive = packetized_receiver.receive().fuse();
        futures_util::pin_mut!(shutdown, receive);

        let packetized = futures_util::select_biased! {
            _ = shutdown => break,
            packetized = receive => packetized,
        };
        let Some(packetized) = packetized else {
            break;
        };

        let shutdown = shutdown_receiver.recv_async().fuse();
        let send = Network::send(&mut sender, packetized).fuse();
        futures_util::pin_mut!(shutdown, send);
        let sent = futures_util::select_biased! {
            _ = shutdown => break,
            sent = send => sent?,
        };
        let SentBytes {
            capture_source_id,
            stream_id,
            bytes,
        } = sent;
        metrics.record_sent_bytes(capture_source_id, stream_id, bytes);
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
        app::container::host_stream_pipeline::outbound_port::{EncodedVideoUnit, UnitNumber},
        domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
    };

    struct NonSendTransporterState {
        _not_send: PhantomData<Rc<()>>,
        caller_thread_id: thread::ThreadId,
        dropped_on_network_thread: Arc<AtomicBool>,
    }

    struct CancellationTransporterState {
        packetized_sender: flume::Sender<u64>,
        sender: Option<CancellationSender>,
        receiver: Option<CancellationReceiver>,
    }

    struct CancellationSender {
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
        type Sender = CancellationSender;

        fn take_sender(&mut self) -> eros::Result<Self::Sender> {
            Ok(self
                .state_mut()
                .sender
                .take()
                .with_context(|| "Cancellation test sender has already been taken")?)
        }

        fn packetize(
            &mut self,
            _stream_id: StreamId,
            unit: EncodedVideoUnit<Self::EncodedBuffer>,
        ) -> eros::Result<Self::Packetized> {
            self.state_mut()
                .packetized_sender
                .send(unit.data)
                .with_context(|| "Cancellation test stopped observing packetized units")?;
            Ok(unit.data)
        }

        async fn send(
            sender: &mut Self::Sender,
            packetized: Self::Packetized,
        ) -> eros::Result<SentBytes> {
            sender
                .send_started_sender
                .send(packetized)
                .with_context(|| "Cancellation test stopped observing started sends")?;
            sender
                .gate_receiver
                .recv_async()
                .await
                .with_context(|| "Cancellation test send gate closed")?;
            sender
                .sent
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
        type Sender = ();

        fn take_sender(&mut self) -> eros::Result<Self::Sender> {
            Ok(())
        }

        fn packetize(
            &mut self,
            _stream_id: StreamId,
            _unit: EncodedVideoUnit<Self::EncodedBuffer>,
        ) -> eros::Result<Self::Packetized> {
            Ok(())
        }

        async fn send(
            _sender: &mut Self::Sender,
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
        let mut worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    packetized_sender,
                    sender: Some(CancellationSender {
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
        let client_event_receiver = worker.take_client_event_receiver()?;
        encoded_sender.send(StreamId::new(0), encoded_unit(1))?;

        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 1);
        assert_eq!(
            send_started_receiver.recv_timeout(Duration::from_secs(1))?,
            1
        );
        drop(encoded_sender);
        drop(client_event_receiver);

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
    fn shutdown_drops_a_queued_packetized_unit() -> eros::Result<()> {
        let (gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, packetized_receiver) = flume::unbounded();
        let (send_started_sender, send_started_receiver) = flume::unbounded();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sent_from_network = Arc::clone(&sent);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let mut worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    packetized_sender,
                    sender: Some(CancellationSender {
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
        let _client_event_receiver = worker.take_client_event_receiver()?;
        encoded_sender.send(StreamId::new(0), encoded_unit(1))?;

        assert_eq!(
            send_started_receiver.recv_timeout(Duration::from_secs(1))?,
            1
        );
        encoded_sender.send(StreamId::new(0), encoded_unit(2))?;
        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 1);
        assert_eq!(packetized_receiver.recv_timeout(Duration::from_secs(1))?, 2);
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
        let mut worker = NetworkWorker::spawn(
            move || {
                Ok(CancellationTransporterState {
                    packetized_sender,
                    sender: Some(CancellationSender {
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
        let _client_event_receiver = worker.take_client_event_receiver()?;
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
                    packetized_sender,
                    sender: Some(CancellationSender {
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
