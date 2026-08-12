use crate::app::{
    container::{
        client::inbound_port::ClientApplication, host::inbound_port::HostApplication,
        network::inbound::EncodedUnitSender, root::AppContainer,
    },
    runtime::AppMessage,
};

impl<Host, Client, NetworkConstructorState> AppContainer<Host, Client, NetworkConstructorState>
where
    Host: HostApplication,
    Client: ClientApplication,
{
    pub(crate) async fn run(
        mut self,
        encoded_unit_sender: EncodedUnitSender<Host::EncodedBuffer>,
        app_message_sender: flume::Sender<AppMessage>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> eros::Result<()> {
        loop {
            match message_receiver.recv_async().await {
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
                    return failure;
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
                    return failure;
                }
                Ok(AppMessage::NetworkWorkerExited) => {
                    let _ = self.shutdown_applications().await;
                    eros::bail!("Network worker exited unexpectedly");
                }
                Ok(AppMessage::Shutdown) | Err(_) => break,
            }
        }

        self.shutdown_applications().await
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
