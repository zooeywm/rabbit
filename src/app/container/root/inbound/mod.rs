use crate::app::{
    container::{
        client::inbound_port::ClientApplication,
        host::inbound_port::HostApplication,
        network::inbound::{ClientStreamControlSender, EncodedUnitSender},
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
        client_stream_control_sender: ClientStreamControlSender<Client::NetworkInput>,
        app_message_sender: flume::Sender<AppMessage>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> AppRunExit {
        loop {
            let message = message_receiver.recv_async().await;

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
                    let result = self
                        .start_stream(
                            capture_source_id,
                            encoded_unit_sender.clone(),
                            &client_stream_control_sender,
                            app_message_sender.clone(),
                        )
                        .await;
                    let _ = response_sender.send(result);
                }
                Ok(AppMessage::RemoveStream {
                    stream_id,
                    response_sender,
                }) => {
                    let _ = response_sender.send(
                        self.remove_stream(stream_id, &client_stream_control_sender)
                            .await,
                    );
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
                    return AppRunExit::Application(Err(failure));
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
                    return AppRunExit::Application(Err(failure));
                }
                Ok(AppMessage::ClientStreamPipelineWorkerExited { stream_id }) => {
                    let Some(failure) = self
                        .client
                        .handle_client_stream_pipeline_worker_exit(stream_id)
                        .await
                    else {
                        continue;
                    };

                    let _ = self.shutdown_applications().await;
                    return AppRunExit::Application(Err(failure));
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

    async fn start_stream(
        &mut self,
        capture_source_id: crate::domain::stream::models::vo::CaptureSourceId,
        encoded_unit_sender: EncodedUnitSender<Host::EncodedBuffer>,
        client_stream_control_sender: &ClientStreamControlSender<Client::NetworkInput>,
        app_message_sender: flume::Sender<AppMessage>,
    ) -> eros::Result<crate::domain::stream::models::vo::StreamId> {
        let stream_id = self
            .host
            .start_stream(
                capture_source_id,
                encoded_unit_sender,
                app_message_sender.clone(),
            )
            .await?;

        let input_sender = match self
            .client
            .start_stream(stream_id, app_message_sender)
            .await
        {
            Ok(sender) => sender,
            Err(error) => {
                let _ = self.host.remove_stream(stream_id).await;
                return Err(error);
            }
        };

        if let Err(error) = client_stream_control_sender
            .register(stream_id, input_sender)
            .await
        {
            let _ = self.client.remove_stream(stream_id).await;
            let _ = self.host.remove_stream(stream_id).await;
            return Err(error);
        }

        Ok(stream_id)
    }

    async fn remove_stream(
        &mut self,
        stream_id: crate::domain::stream::models::vo::StreamId,
        client_stream_control_sender: &ClientStreamControlSender<Client::NetworkInput>,
    ) -> eros::Result<()> {
        self.host.remove_stream(stream_id).await?;

        let unregister_result = client_stream_control_sender.unregister(stream_id).await;
        let client_result = self.client.remove_stream(stream_id).await;
        unregister_result?;
        client_result
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
