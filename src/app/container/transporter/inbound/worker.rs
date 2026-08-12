use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::Context;

use super::unit_queue::EncodedUnitReceiver;
use crate::app::{
    container::{
        root::outbound_port::TransporterMetricsRecorder,
        transporter::{
            TransporterContainer, inbound::EncodedUnitSender, outbound_port::Transporter,
        },
    },
    runtime::AppMessage,
};

pub(crate) struct TransporterWorker;

pub(crate) struct TransporterWorkerHandle<Buffer> {
    sender: EncodedUnitSender<Buffer>,
    worker_thread: JoinHandle<eros::Result<()>>,
}

struct TransporterWorkerExitGuard {
    app_message_sender: flume::Sender<AppMessage>,
}

impl TransporterWorker {
    pub(crate) fn spawn<State, Constructor>(
        transporter_constructor: Constructor,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<
        TransporterWorkerHandle<<TransporterContainer<State> as Transporter>::EncodedBuffer>,
    >
    where
        State: 'static,
        Constructor: FnOnce() -> eros::Result<State> + Send + 'static,
        TransporterContainer<State>: Transporter + TransporterMetricsRecorder,
        <TransporterContainer<State> as Transporter>::EncodedBuffer: Send + 'static,
    {
        let (sender, receiver) = EncodedUnitReceiver::channel();
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let worker_thread = thread::Builder::new()
            .name("transporter".to_owned())
            .spawn(move || {
                let _exit_guard = TransporterWorkerExitGuard { app_message_sender };
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for transporter worker")?;

                runtime.block_on(run_transporter_worker(
                    transporter_constructor,
                    receiver,
                    started_sender,
                ))
            })
            .with_context(|| "Failed to spawn transporter worker thread")?;

        if started_receiver.recv().is_err() {
            join_transporter_worker(worker_thread)?;
            eros::bail!("Transporter worker stopped before startup completed");
        }

        Ok(TransporterWorkerHandle {
            sender,
            worker_thread,
        })
    }
}

impl Drop for TransporterWorkerExitGuard {
    fn drop(&mut self) {
        let _ = self
            .app_message_sender
            .send(AppMessage::TransporterWorkerExited);
    }
}

impl<Buffer> TransporterWorkerHandle<Buffer> {
    pub(crate) fn sender(&self) -> EncodedUnitSender<Buffer> {
        self.sender.clone()
    }

    pub(crate) async fn shutdown(self) -> eros::Result<()> {
        let Self {
            sender,
            worker_thread,
        } = self;
        drop(sender);

        match compio::runtime::spawn_blocking(move || join_transporter_worker(worker_thread)).await
        {
            Ok(result) => result,
            Err(_) => eros::bail!("Transporter worker join task failed"),
        }
    }
}

async fn run_transporter_worker<State, Constructor>(
    transporter_constructor: Constructor,
    receiver: EncodedUnitReceiver<<TransporterContainer<State> as Transporter>::EncodedBuffer>,
    started_sender: mpsc::SyncSender<()>,
) -> eros::Result<()>
where
    Constructor: FnOnce() -> eros::Result<State>,
    TransporterContainer<State>: Transporter + TransporterMetricsRecorder,
{
    let mut transporter = TransporterContainer::new(transporter_constructor()?);
    transporter.register_transporter_queue_usage(receiver.usage());
    started_sender
        .send(())
        .with_context(|| "Failed to report transporter worker startup")?;

    let result = async {
        while let Some(item) = receiver.receive().await {
            let packetized = Transporter::packetize(&mut transporter, item.stream_id, item.unit)?;
            Transporter::send(&mut transporter, packetized).await?;
        }
        Ok(())
    }
    .await;

    transporter.unregister_transporter_queue_usage();
    result
}

fn join_transporter_worker(worker_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match worker_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Transporter worker thread panicked"),
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
        app::container::stream_pipeline::outbound_port::EncodedVideoUnit,
        domain::stream::models::vo::StreamId,
    };

    struct NonSendTransporterState(PhantomData<Rc<()>>);

    impl Transporter
        for crate::app::container::transporter::TransporterContainer<NonSendTransporterState>
    {
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
    fn constructs_non_send_transporter_on_the_transporter_thread() {
        let caller_thread_id = thread::current().id();
        let constructed_on_transporter_thread = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&constructed_on_transporter_thread);
        let (app_message_sender, _app_message_receiver) = flume::unbounded();

        let worker = TransporterWorker::spawn(
            move || {
                flag.store(
                    thread::current().id() != caller_thread_id,
                    Ordering::Relaxed,
                );
                Ok(NonSendTransporterState(PhantomData))
            },
            app_message_sender,
        )
        .expect("transporter worker should start");

        let runtime = compio::runtime::Runtime::new().expect("test runtime should start");
        runtime
            .block_on(worker.shutdown())
            .expect("transporter worker should stop");
        assert!(constructed_on_transporter_thread.load(Ordering::Relaxed));
    }
}
