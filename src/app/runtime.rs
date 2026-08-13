use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use eros::{Context, bail};
use futures_util::FutureExt;

use crate::{
    app::container::network::inbound::NetworkWorker,
    domain::stream::models::{
        StreamRequest,
        vo::{CaptureSourceId, StreamId},
    },
};

pub(crate) enum AppMessage {
    User(UserMessage),
    Network(NetworkMessage),
}

pub(crate) enum UserMessage {
    Start {
        capture_source_id: CaptureSourceId,
        response_sender: flume::Sender<eros::Result<StreamId>>,
    },
    Remove {
        stream_id: StreamId,
        response_sender: flume::Sender<eros::Result<()>>,
    },
}

pub(crate) enum NetworkMessage {
    Request(StreamRequest),
}

pub(super) struct AppRuntime {
    app_handle: AppHandle,
    shutdown_sender: flume::Sender<()>,
    app_thread: JoinHandle<eros::Result<()>>,
}

#[derive(Clone)]
pub(crate) struct AppHandle {
    message_sender: flume::Sender<AppMessage>,
}

impl AppRuntime {
    pub(super) fn start() -> eros::Result<Self> {
        let (message_sender, message_receiver) = flume::unbounded();
        let (shutdown_sender, shutdown_receiver) = flume::bounded(1);
        let (started_sender, started_receiver) = mpsc::sync_channel(1);
        let container_app_message_sender = message_sender.clone();

        let app_thread = thread::Builder::new()
            .name("app".to_owned())
            .spawn(move || {
                let runtime = compio::runtime::Runtime::new()
                    .with_context(|| "Failed to create Compio runtime for app")?;
                let (mut host, mut client, networks) = runtime
                    .enter(|| crate::composition::compose_containers(container_app_message_sender))
                    .with_context(|| "Failed to construct application containers")?;
                let mut network_workers = Vec::with_capacity(networks.len());
                for network in networks {
                    match NetworkWorker::spawn(network) {
                        Ok(worker) => network_workers.push(worker),
                        Err(error) => {
                            for worker in network_workers {
                                let _ = runtime.block_on(worker.shutdown());
                            }
                            return Err(error);
                        }
                    }
                }

                started_sender
                    .send(())
                    .with_context(|| "Failed to report app startup")?;

                let app_result = runtime.block_on(async move {
                    loop {
                        let message = message_receiver.recv_async().fuse();
                        let shutdown = shutdown_receiver.recv_async().fuse();
                        futures_util::pin_mut!(message, shutdown);
                        let message = futures_util::select_biased! {
                            _ = shutdown => break,
                            message = message => message,
                        };

                        match message {
                            Ok(AppMessage::User(UserMessage::Start {
                                capture_source_id,
                                response_sender,
                            })) => {
                                let result = client.start_stream(capture_source_id).await;
                                response_sender
                                    .send(result)
                                    .with_context(|| "Failed response StartStream")?;
                            }
                            Ok(AppMessage::User(UserMessage::Remove {
                                stream_id,
                                response_sender,
                            })) => {
                                response_sender
                                    .send(client.remove_stream(stream_id).await)
                                    .with_context(|| "Failed response RemoveStream")?;
                            }
                            Ok(AppMessage::Network(NetworkMessage::Request(request))) => {
                                let result = match request {
                                    StreamRequest::Start {
                                        capture_source_id,
                                        stream_id,
                                    } => host.start_stream(capture_source_id, stream_id).await,
                                    StreamRequest::Remove { stream_id } => {
                                        host.remove_stream(stream_id).await
                                    }
                                };
                                if let Err(error) = result {
                                    tracing::error!(?error, "Failed to handle network request");
                                }
                            }
                            Err(e) => bail!("Failed recv app message: {:?}", e),
                        }
                    }

                    let host_result = host.shutdown().await;
                    let client_result = client.shutdown().await;

                    host_result?;
                    client_result
                });
                let mut network_result = Ok(());
                for worker in network_workers {
                    if let Err(error) = runtime.block_on(worker.shutdown())
                        && network_result.is_ok()
                    {
                        network_result = Err(error);
                    }
                }

                app_result?;
                network_result
            })
            .with_context(|| "Failed to spawn app thread")?;

        if started_receiver.recv().is_err() {
            join_app_thread(app_thread)?;
            eros::bail!("App thread stopped before startup completed");
        }

        Ok(Self {
            app_handle: AppHandle { message_sender },
            shutdown_sender,
            app_thread,
        })
    }

    pub(super) fn shutdown(self) -> eros::Result<()> {
        let Self {
            app_handle,
            shutdown_sender,
            app_thread,
        } = self;

        let send_result = shutdown_sender.send(());
        drop(app_handle);

        join_app_thread(app_thread)?;

        send_result.with_context(|| "App stopped before receiving shutdown")?;

        Ok(())
    }

    pub(super) fn handle(&self) -> AppHandle {
        self.app_handle.clone()
    }
}

impl AppHandle {
    pub(crate) async fn start_stream(
        &self,
        capture_source_id: CaptureSourceId,
    ) -> eros::Result<StreamId> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::User(UserMessage::Start {
                capture_source_id,
                response_sender,
            }))
            .with_context(|| "App stopped before stream could be started")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while starting stream")?
    }

    pub(crate) async fn remove_stream(&self, stream_id: StreamId) -> eros::Result<()> {
        let (response_sender, response_receiver) = flume::bounded(1);

        self.message_sender
            .send(AppMessage::User(UserMessage::Remove {
                stream_id,
                response_sender,
            }))
            .with_context(|| "App stopped before stream could be removed")?;

        response_receiver
            .recv_async()
            .await
            .with_context(|| "App stopped while removing stream")?
    }

    #[cfg(feature = "test-ui")]
    pub(crate) async fn simulate_remote_start_stream(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        self.simulate_remote_network_request(StreamRequest::Start {
            capture_source_id,
            stream_id,
        })
        .await
    }

    #[cfg(feature = "test-ui")]
    pub(crate) async fn simulate_remote_remove_stream(
        &self,
        stream_id: StreamId,
    ) -> eros::Result<()> {
        self.simulate_remote_network_request(StreamRequest::Remove { stream_id })
            .await
    }

    #[cfg(feature = "test-ui")]
    async fn simulate_remote_network_request(&self, request: StreamRequest) -> eros::Result<()> {
        self.message_sender
            .send_async(AppMessage::Network(NetworkMessage::Request(request)))
            .await
            .map_err(|_| {
                eros::error!("App stopped before simulated remote request could be handled")
            })
    }
}

fn join_app_thread(app_thread: JoinHandle<eros::Result<()>>) -> eros::Result<()> {
    match app_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("App thread panicked"),
    }
}
