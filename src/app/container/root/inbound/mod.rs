use crate::app::{
    container::{
        client::inbound_port::ClientApplication,
        host::inbound_port::HostApplication,
        network::inbound::{EncodedUnitSender, NetworkClientEventReceiver},
        root::AppContainer,
    },
    runtime::AppMessage,
};

pub(crate) enum AppRunExit {
    Application(eros::Result<()>),
    NetworkWorkerExited,
}

impl<Host, Client, NetworkConstructorState> AppContainer<Host, Client, NetworkConstructorState>
where
    Host: HostApplication,
    Client: ClientApplication,
{
    pub(crate) async fn run(
        mut self,
        encoded_unit_sender: EncodedUnitSender<Host::EncodedBuffer>,
        network_client_event_receiver: NetworkClientEventReceiver<Client::NetworkInput>,
        app_message_sender: flume::Sender<AppMessage>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> AppRunExit {
        use futures_util::{FutureExt, select_biased};

        loop {
            let app_message = message_receiver.recv_async().fuse();
            let network_client_event = network_client_event_receiver.receive().fuse();
            futures_util::pin_mut!(app_message, network_client_event);

            let message = select_biased! {
                message = app_message => Some(message),
                event = network_client_event => {
                    let Some(event) = event else {
                        let _ = self.shutdown_applications().await;
                        return AppRunExit::NetworkWorkerExited;
                    };
                    if let Err(failure) = self
                        .client
                        .handle_network_input(event.stream_id, event.input)
                    {
                        let _ = self.shutdown_applications().await;
                        return AppRunExit::Application(Err(failure));
                    }
                    None
                },
            };

            let Some(message) = message else {
                continue;
            };

            match message {
                #[cfg(feature = "test-ui")]
                Ok(AppMessage::CaptureOnly(message)) => {
                    use crate::app::runtime::capture_only::CaptureOnlyMessage;

                    match message {
                        CaptureOnlyMessage::Start {
                            capture_source_id,
                            response_sender,
                        } => {
                            let _ = response_sender.send(
                                self.host
                                    .start_capture_only(
                                        capture_source_id,
                                        app_message_sender.clone(),
                                    )
                                    .await,
                            );
                        }
                        CaptureOnlyMessage::Stop {
                            capture_source_id,
                            response_sender,
                        } => {
                            let _ = response_sender
                                .send(self.host.stop_capture_only(capture_source_id).await);
                        }
                    }
                }
                Ok(AppMessage::StartStream {
                    capture_source_id,
                    response_sender,
                }) => {
                    let _ = response_sender.send(
                        self.host
                            .start_stream(
                                capture_source_id,
                                encoded_unit_sender.clone(),
                                app_message_sender.clone(),
                            )
                            .await,
                    );
                }
                Ok(AppMessage::RemoveStream {
                    stream_id,
                    response_sender,
                }) => {
                    let _ = response_sender.send(self.host.remove_stream(stream_id).await);
                }
                Ok(AppMessage::CaptureWorkerExited { capture_source_id }) => {
                    let Some(failure) = self
                        .host
                        .handle_capture_worker_exit(capture_source_id)
                        .await
                    else {
                        continue;
                    };

                    let _ = self.shutdown_applications().await;
                    return AppRunExit::Application(failure);
                }
                Ok(AppMessage::HostStreamPipelineWorkerExited {
                    capture_source_id,
                    stream_id,
                }) => {
                    let Some(failure) = self
                        .host
                        .handle_host_stream_pipeline_worker_exit(capture_source_id, stream_id)
                        .await
                    else {
                        continue;
                    };

                    let _ = self.shutdown_applications().await;
                    return AppRunExit::Application(failure);
                }
                Ok(AppMessage::NetworkWorkerExited) => {
                    let _ = self.shutdown_applications().await;
                    return AppRunExit::NetworkWorkerExited;
                }
                Ok(AppMessage::Shutdown) | Err(_) => break,
            }
        }

        AppRunExit::Application(self.shutdown_applications().await)
    }

    async fn shutdown_applications(self) -> eros::Result<()> {
        let Self {
            host,
            client,
            network_constructor_state: _,
        } = self;

        let host_result = host.shutdown().await;
        let client_result = client.shutdown().await;

        host_result?;
        client_result
    }
}
