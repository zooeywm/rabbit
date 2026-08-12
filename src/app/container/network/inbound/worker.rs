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
        NetworkContainer,
        inbound::{EncodedUnitSender, NetworkClientEvent},
        outbound_port::{NetworkMetricsRecorder, TransporterClientSide, TransporterHostSide},
    },
    runtime::AppMessage,
};

pub(crate) struct NetworkWorker;

pub(crate) struct NetworkWorkerHandle<Buffer, ClientInput> {
    sender: EncodedUnitSender<Buffer>,
    client_event_receiver: Option<NetworkClientEventReceiver<ClientInput>>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct NetworkWorkerExitGuard {
    app_message_sender: flume::Sender<AppMessage>,
}

enum NetworkTaskExit {
    EventLoop(eros::Result<()>),
    SendLoop(eros::Result<()>),
    ReceiveLoop(eros::Result<()>),
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
            sender,
            client_event_receiver,
            worker_thread,
        } = self;
        drop(sender);
        drop(client_event_receiver);

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
    let (receive_shutdown_sender, receive_shutdown_receiver) = flume::bounded(1);

    network.register_network_queue_usage(encoded_receiver.usage());
    started_sender
        .send(())
        .with_context(|| "Failed to report network worker startup")?;

    let mut tasks = FuturesUnordered::new();
    tasks.push(compio::runtime::spawn(async move {
        NetworkTaskExit::EventLoop(
            run_network_event_loop(
                network,
                encoded_receiver,
                received_receiver,
                packetized_sender,
                packetize_permits,
                client_event_sender,
                event_shutdown_receiver,
            )
            .await,
        )
    }));
    tasks.push(compio::runtime::spawn(async move {
        NetworkTaskExit::SendLoop(
            run_send_loop::<NetworkContainer<State>>(sender, packetized_receiver).await,
        )
    }));
    tasks.push(compio::runtime::spawn(async move {
        NetworkTaskExit::ReceiveLoop(
            run_receive_loop::<NetworkContainer<State>>(
                receiver,
                received_sender,
                receive_shutdown_receiver,
            )
            .await,
        )
    }));

    let mut first_error = None;
    while let Some(joined) = tasks.next().await {
        let exit = match joined {
            Ok(exit) => exit,
            Err(error) => {
                tracing::error!(%error, "Network task failed to join");
                if first_error.is_none() {
                    first_error = Some(eros::error!("Network task failed to join"));
                }
                let _ = event_shutdown_sender.try_send(());
                let _ = receive_shutdown_sender.try_send(());
                continue;
            }
        };

        let (is_event_loop, result) = match exit {
            NetworkTaskExit::EventLoop(result) => (true, result),
            NetworkTaskExit::SendLoop(result) | NetworkTaskExit::ReceiveLoop(result) => {
                (false, result)
            }
        };

        if let Err(error) = result {
            if first_error.is_none() {
                first_error = Some(error);
            }
            let _ = event_shutdown_sender.try_send(());
            let _ = receive_shutdown_sender.try_send(());
        } else if is_event_loop {
            let _ = receive_shutdown_sender.try_send(());
        }
    }

    // All task-owned transporter parts have been dropped at this point.
    crate::app::container::network::NetworkMetricsHandle::new().unregister_network_queue_usage();

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

    loop {
        let wait_for_packetize_permit = !may_packetize;
        let wait_for_client_event_permit = !may_route_client_event;
        let receive_encoded = may_packetize;
        let receive_network = may_route_client_event;
        let shutdown = shutdown_receiver.recv_async().fuse();
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
        futures_util::pin_mut!(
            shutdown,
            packetize_permit,
            client_event_permit,
            encoded,
            received
        );

        futures_util::select_biased! {
            _ = shutdown => break,
            permit = packetize_permit => {
                permit?;
                may_packetize = true;
            },
            permit = client_event_permit => {
                permit?;
                may_route_client_event = true;
            },
            item = encoded => {
                let Some(item) = item else {
                    break;
                };
                let packetized = network.packetize(item.stream_id, item.unit)?;
                packetized_sender.send(packetized)?;
                may_packetize = false;
            },
            item = received => {
                let Some(item) = item else {
                    eros::bail!("Network receive loop stopped unexpectedly");
                };
                let (stream_id, input) = network.depacketize(item)?;
                client_event_sender.send(NetworkClientEvent { stream_id, input })?;
                may_route_client_event = false;
            },
        }
    }

    Ok(())
}

async fn run_send_loop<Network>(
    mut sender: Network::Sender,
    packetized_receiver: PacketizedSendReceiver<Network::Packetized>,
) -> eros::Result<()>
where
    Network: TransporterHostSide,
{
    while let Some(packetized) = packetized_receiver.receive().await {
        Network::send(&mut sender, packetized).await?;
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
        if received_sender.send_async(received).await.is_err() {
            break;
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
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
    };

    use super::*;
    use crate::{
        app::container::host_stream_pipeline::outbound_port::EncodedVideoUnit,
        domain::stream::models::vo::StreamId,
    };

    struct NonSendTransporterState(PhantomData<Rc<()>>);

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
        ) -> eros::Result<()> {
            Ok(())
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
    fn constructs_non_send_transporter_on_the_network_thread() {
        let caller_thread_id = thread::current().id();
        let constructed_on_network_thread = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&constructed_on_network_thread);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();

        let worker = NetworkWorker::spawn(
            move || {
                flag.store(
                    thread::current().id() != caller_thread_id,
                    Ordering::Relaxed,
                );
                Ok(NonSendTransporterState(PhantomData))
            },
            app_message_sender,
        )
        .expect("network worker should start");

        let runtime = compio::runtime::Runtime::new().expect("test runtime should start");
        runtime
            .block_on(worker.shutdown())
            .expect("network worker should stop");
        assert!(constructed_on_network_thread.load(Ordering::Relaxed));
    }
}
