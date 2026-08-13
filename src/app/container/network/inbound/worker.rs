use std::{
    collections::HashMap,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Instant,
};

use eros::Context;
use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};

use super::{
    client_stream_control::{ClientStreamControl, ClientStreamControlReceiver},
    network_request::NetworkRequestReceiver,
    unit_queue::EncodedUnitReceiver,
};
use crate::app::{
    container::network::{
        ConfiguredNetworkContainer, NetworkContainer, NetworkMetricsHandle,
        outbound_port::{
            NetworkMetricsRecorder, NetworkState, SentBytes, TransporterClientSide,
            TransporterHostSide,
        },
    },
    runtime::{AppMessage, NetworkMessage},
};

pub(crate) struct NetworkWorker;

pub(crate) struct NetworkWorkerHandle {
    shutdown_sender: flume::Sender<()>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct NetworkQueueMetricsGuard;

impl NetworkWorker {
    pub(crate) fn spawn<State>(
        network: ConfiguredNetworkContainer<State>,
    ) -> eros::Result<NetworkWorkerHandle>
    where
        State: NetworkState,
        NetworkContainer<State>: TransporterHostSide<EncodedBuffer = State::EncodedBuffer>
            + TransporterClientSide<Depacketized = State::ClientInput>
            + NetworkMetricsRecorder,
        <NetworkContainer<State> as TransporterHostSide>::EncodedBuffer: Send + 'static,
        <NetworkContainer<State> as TransporterHostSide>::Packetized: 'static,
        <NetworkContainer<State> as TransporterClientSide>::Received: 'static,
        <NetworkContainer<State> as TransporterClientSide>::Depacketized:
            crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit,
    {
        let (shutdown_sender, shutdown_receiver) = flume::bounded(1);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let worker_thread = thread::Builder::new()
            .name("network".to_owned())
            .spawn(move || {
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for network worker")?;
                let network = network.initialize()?;

                runtime.block_on(run_network_worker(
                    network,
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
            shutdown_sender,
            worker_thread,
        })
    }
}

impl NetworkWorkerHandle {
    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            shutdown_sender,
            worker_thread,
            ..
        } = self;
        let _ = shutdown_sender.try_send(());

        match compio::runtime::spawn_blocking(move || join_network_worker(worker_thread)).await {
            Ok(result) => result,
            Err(_) => eros::bail!("Network worker join task failed"),
        }
    }
}

async fn run_network_worker<State>(
    mut network: NetworkContainer<State>,
    worker_shutdown_receiver: flume::Receiver<()>,
    started_sender: mpsc::SyncSender<()>,
) -> eros::Result<()>
where
    State: NetworkState,
    NetworkContainer<State>: TransporterHostSide<EncodedBuffer = State::EncodedBuffer>
        + TransporterClientSide<Depacketized = State::ClientInput>
        + NetworkMetricsRecorder
        + 'static,
    <NetworkContainer<State> as TransporterHostSide>::Packetized: 'static,
    <NetworkContainer<State> as TransporterClientSide>::Received: 'static,
    <NetworkContainer<State> as TransporterClientSide>::Depacketized:
        crate::app::container::client_stream_pipeline::outbound_port::VideoDecodeUnit,
{
    let encoded_receiver = network.take_encoded_unit_receiver();
    let client_stream_control_receiver = network.take_client_stream_control_receiver();
    let network_request_receiver = network.take_network_request_receiver();
    let app_message_sender = network.app_message_sender();
    let host = TransporterHostSide::take_host(&mut network)?;
    let request_receiver = TransporterHostSide::take_request_receiver(&mut network)?;
    let receiver = TransporterClientSide::take_receiver(&mut network)?;
    let request_sender = TransporterClientSide::take_request_sender(&mut network)?;
    let (received_sender, received_receiver) = flume::bounded(1);
    let (event_shutdown_sender, event_shutdown_receiver) = flume::bounded(1);
    let (host_shutdown_sender, host_shutdown_receiver) = flume::bounded(1);
    let (receive_shutdown_sender, receive_shutdown_receiver) = flume::bounded(1);
    let (request_send_shutdown_sender, request_send_shutdown_receiver) = flume::bounded(1);
    let (request_receive_shutdown_sender, request_receive_shutdown_receiver) = flume::bounded(1);

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
    tasks.push(compio::runtime::spawn(async move {
        run_request_send_loop::<NetworkContainer<State>>(
            request_sender,
            network_request_receiver,
            request_send_shutdown_receiver,
        )
        .await
    }));
    tasks.push(compio::runtime::spawn(async move {
        run_request_receive_loop::<NetworkContainer<State>>(
            request_receiver,
            app_message_sender,
            request_receive_shutdown_receiver,
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
                    let _ = request_send_shutdown_sender.try_send(());
                    let _ = request_receive_shutdown_sender.try_send(());
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
                let _ = request_send_shutdown_sender.try_send(());
                let _ = request_receive_shutdown_sender.try_send(());
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
        let _ = request_send_shutdown_sender.try_send(());
        let _ = request_receive_shutdown_sender.try_send(());
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn run_request_send_loop<Network>(
    mut sender: Network::RequestSender,
    request_receiver: NetworkRequestReceiver,
    shutdown_receiver: flume::Receiver<()>,
) -> eros::Result<()>
where
    Network: TransporterClientSide,
{
    loop {
        let request = {
            let request = request_receiver.receive().fuse();
            let shutdown = shutdown_receiver.recv_async().fuse();
            futures_util::pin_mut!(request, shutdown);
            futures_util::select_biased! {
                _ = shutdown => return Ok(()),
                request = request => request,
            }
        };
        let Some(request) = request else {
            let _ = shutdown_receiver.recv_async().await;
            return Ok(());
        };

        Network::send_request(&mut sender, request).await?;
    }
}

async fn run_request_receive_loop<Network>(
    mut receiver: Network::RequestReceiver,
    app_message_sender: flume::Sender<AppMessage>,
    shutdown_receiver: flume::Receiver<()>,
) -> eros::Result<()>
where
    Network: TransporterHostSide,
{
    loop {
        let request = Network::receive_request(&mut receiver).fuse();
        let shutdown = shutdown_receiver.recv_async().fuse();
        futures_util::pin_mut!(request, shutdown);

        let Some(request) = (futures_util::select_biased! {
            _ = shutdown => break,
            request = request => request?,
        }) else {
            break;
        };

        app_message_sender
            .send_async(AppMessage::Network(NetworkMessage::Request(request)))
            .await
            .map_err(|_| eros::error!("App stopped before handling remote network request"))?;
    }

    Ok(())
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
    loop {
        let item = {
            let receive = encoded_receiver.receive().fuse();
            let shutdown = shutdown_receiver.recv_async().fuse();
            futures_util::pin_mut!(receive, shutdown);
            futures_util::select_biased! {
                _ = shutdown => return Ok(()),
                item = receive => item,
            }
        };
        let Some(item) = item else {
            let _ = shutdown_receiver.recv_async().await;
            return Ok(());
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
    }
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
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use super::*;
    use crate::{
        app::container::{
            client_stream_pipeline::outbound_port::VideoDecodeUnit,
            host_stream_pipeline::outbound_port::{EncodedVideoUnit, UnitNumber},
            network::inbound::{ClientStreamControlSender, EncodedUnitSender},
        },
        domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
    };

    impl VideoDecodeUnit for () {
        fn is_recovery_point(&self) -> bool {
            true
        }
    }

    struct CancellationTransporterState {
        host: Option<CancellationHost>,
        receiver: Option<CancellationReceiver>,
    }

    impl NetworkState for CancellationTransporterState {
        type Config = Self;
        type EncodedBuffer = u64;
        type ClientInput = ();

        fn new(config: Self::Config) -> eros::Result<Self> {
            Ok(config)
        }
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

    struct CancellationRequestSender;
    struct CancellationRequestReceiver;

    impl TransporterHostSide
        for crate::app::container::network::NetworkContainer<CancellationTransporterState>
    {
        type EncodedBuffer = u64;
        type Packetized = u64;
        type Host = CancellationHost;
        type RequestReceiver = CancellationRequestReceiver;

        fn take_host(&mut self) -> eros::Result<Self::Host> {
            Ok(self
                .state_mut()
                .host
                .take()
                .with_context(|| "Cancellation test host half has already been taken")?)
        }

        fn take_request_receiver(&mut self) -> eros::Result<Self::RequestReceiver> {
            Ok(CancellationRequestReceiver)
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

        async fn receive_request(
            _receiver: &mut Self::RequestReceiver,
        ) -> eros::Result<Option<crate::domain::stream::models::StreamRequest>> {
            futures_util::future::pending().await
        }
    }

    impl TransporterClientSide
        for crate::app::container::network::NetworkContainer<CancellationTransporterState>
    {
        type Receiver = CancellationReceiver;
        type RequestSender = CancellationRequestSender;
        type Received = ();
        type Depacketized = ();

        fn take_receiver(&mut self) -> eros::Result<Self::Receiver> {
            Ok(self
                .state_mut()
                .receiver
                .take()
                .with_context(|| "Cancellation test receiver has already been taken")?)
        }

        fn take_request_sender(&mut self) -> eros::Result<Self::RequestSender> {
            Ok(CancellationRequestSender)
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

        async fn send_request(
            _sender: &mut Self::RequestSender,
            _request: crate::domain::stream::models::StreamRequest,
        ) -> eros::Result<()> {
            Ok(())
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

    fn spawn_cancellation_worker(
        state: CancellationTransporterState,
    ) -> eros::Result<(
        NetworkWorkerHandle,
        EncodedUnitSender<u64>,
        ClientStreamControlSender<()>,
        crate::app::container::network::inbound::NetworkRequestSender,
    )> {
        let (encoded_unit_sender, encoded_unit_receiver) = EncodedUnitSender::channel();
        let (client_stream_control_sender, client_stream_control_receiver) =
            ClientStreamControlSender::channel();
        let (network_request_sender, network_request_receiver) =
            crate::app::container::network::inbound::NetworkRequestSender::channel();
        let (app_message_sender, _app_message_receiver) = flume::unbounded();
        let worker = NetworkWorker::spawn(ConfiguredNetworkContainer::<
            CancellationTransporterState,
        >::new(
            state,
            encoded_unit_receiver,
            client_stream_control_receiver,
            network_request_receiver,
            app_message_sender,
        ))?;

        Ok((
            worker,
            encoded_unit_sender,
            client_stream_control_sender,
            network_request_sender,
        ))
    }

    fn shutdown_with_timeout(worker: NetworkWorkerHandle) -> eros::Result<()> {
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

    #[test]
    fn shutdown_cancels_a_blocked_in_flight_send() -> eros::Result<()> {
        let (gate_sender, gate_receiver) = flume::unbounded();
        let (packetized_sender, packetized_receiver) = flume::unbounded();
        let (send_started_sender, send_started_receiver) = flume::unbounded();
        let sent = Arc::new(Mutex::new(Vec::new()));
        let sent_from_network = Arc::clone(&sent);
        let (worker, encoded_sender, _client_stream_control_sender, _network_request_sender) =
            spawn_cancellation_worker(CancellationTransporterState {
                host: Some(CancellationHost {
                    packetized_sender,
                    gate_receiver,
                    send_started_sender,
                    sent: sent_from_network,
                }),
                receiver: Some(pending_cancellation_receiver()),
            })?;
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
        let (worker, encoded_sender, _client_stream_control_sender, _network_request_sender) =
            spawn_cancellation_worker(CancellationTransporterState {
                host: Some(CancellationHost {
                    packetized_sender,
                    gate_receiver,
                    send_started_sender,
                    sent: sent_from_network,
                }),
                receiver: Some(pending_cancellation_receiver()),
            })?;
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
        let (worker, encoded_sender, _client_stream_control_sender, _network_request_sender) =
            spawn_cancellation_worker(CancellationTransporterState {
                host: Some(CancellationHost {
                    packetized_sender,
                    gate_receiver,
                    send_started_sender,
                    sent,
                }),
                receiver: Some(pending_cancellation_receiver()),
            })?;
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
        let (worker, _encoded_sender, _client_stream_control_sender, _network_request_sender) =
            spawn_cancellation_worker(CancellationTransporterState {
                host: Some(CancellationHost {
                    packetized_sender,
                    gate_receiver,
                    send_started_sender,
                    sent: Arc::new(Mutex::new(Vec::new())),
                }),
                receiver: Some(CancellationReceiver::Fail),
            })?;

        let error = shutdown_with_timeout(worker).expect_err("receive should fail");
        assert!(format!("{error:?}").contains("Intentional receive failure"));
        Ok(())
    }
}
