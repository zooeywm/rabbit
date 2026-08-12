use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use super::unit_queue::EncodedUnitReceiver;
use crate::app::{
    container::{
        network::{NetworkContainer, inbound::EncodedUnitSender, outbound_port::Transporter},
        root::outbound_port::NetworkMetricsRecorder,
    },
    runtime::AppMessage,
};

pub(crate) struct NetworkWorker;

pub(crate) struct NetworkWorkerHandle<Buffer> {
    sender: EncodedUnitSender<Buffer>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct NetworkWorkerExitGuard {
    app_message_sender: flume::Sender<AppMessage>,
}

impl NetworkWorker {
    pub(crate) fn spawn<State, Constructor>(
        transporter_constructor: Constructor,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<NetworkWorkerHandle<<NetworkContainer<State> as Transporter>::EncodedBuffer>>
    where
        State: 'static,
        Constructor: FnOnce() -> eros::Result<State> + Send + 'static,
        NetworkContainer<State>: Transporter + NetworkMetricsRecorder,
        <NetworkContainer<State> as Transporter>::EncodedBuffer: Send + 'static,
    {
        let (sender, receiver) = EncodedUnitReceiver::channel();
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

impl<Buffer> NetworkWorkerHandle<Buffer> {
    pub(crate) fn sender(&self) -> EncodedUnitSender<Buffer> {
        self.sender.clone()
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            sender,
            worker_thread,
        } = self;
        drop(sender);

        match compio::runtime::spawn_blocking(move || join_network_worker(worker_thread)).await {
            Ok(result) => result,
            Err(_) => eros::bail!("Network worker join task failed"),
        }
    }
}

async fn run_network_worker<State, Constructor>(
    transporter_constructor: Constructor,
    receiver: EncodedUnitReceiver<<NetworkContainer<State> as Transporter>::EncodedBuffer>,
    started_sender: mpsc::SyncSender<()>,
) -> eros::Result<()>
where
    Constructor: FnOnce() -> eros::Result<State>,
    NetworkContainer<State>: Transporter + NetworkMetricsRecorder,
{
    let mut network = NetworkContainer::new(transporter_constructor()?);
    network.register_network_queue_usage(receiver.usage());
    started_sender
        .send(())
        .with_context(|| "Failed to report network worker startup")?;

    let result = async {
        while let Some(item) = receiver.receive().await {
            let packetized = Transporter::packetize(&mut network, item.stream_id, item.unit)?;
            Transporter::send(&mut network, packetized).await?;
        }
        Ok(())
    }
    .await;

    network.unregister_network_queue_usage();
    result
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

    impl Transporter for crate::app::container::network::NetworkContainer<NonSendTransporterState> {
        type EncodedBuffer = ();
        type Packetized = ();

        fn packetize(
            &mut self,
            _stream_id: StreamId,
            _unit: EncodedVideoUnit<Self::EncodedBuffer>,
        ) -> eros::Result<Self::Packetized> {
            Ok(())
        }

        async fn send(&mut self, _packetized: Self::Packetized) -> eros::Result<()> {
            Ok(())
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
