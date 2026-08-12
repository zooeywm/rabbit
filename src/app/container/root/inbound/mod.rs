use crate::app::{
    container::{
        host::{
            CapturedFrameFor, EncodedBufferFor, EncoderInputFor, HostContainer,
            HostStreamPipelineFor,
            outbound_port::{
                CapturerManager, CapturerManagerStateSpec, ConverterManager,
                ConverterManagerStateSpec, EncoderManager, EncoderManagerStateSpec,
                MetricsRecorder,
            },
        },
        host_stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
        network::inbound::EncodedUnitSender,
        root::AppContainer,
    },
    runtime::AppMessage,
};

impl<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>
    AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, NetworkConstructorState>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    HostStreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>
        + MetricsRecorder,
    EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
{
    pub(crate) async fn run(
        mut self,
        encoded_unit_sender: EncodedUnitSender<EncodedBufferFor<CvtMgrSt, EcdMgrSt>>,
        app_message_sender: flume::Sender<AppMessage>,
        message_receiver: flume::Receiver<AppMessage>,
    ) -> eros::Result<()> {
        loop {
            match message_receiver.recv_async().await {
                #[cfg(feature = "test-ui")]
                Ok(AppMessage::CaptureOnly(message)) => {
                    self.host
                        .handle_capture_only_message(message, &app_message_sender)
                        .await;
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
                                &app_message_sender,
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

                    let _ = self.host.shutdown().await;
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

                    let _ = self.host.shutdown().await;
                    return failure;
                }
                Ok(AppMessage::NetworkWorkerExited) => {
                    let _ = self.host.shutdown().await;
                    eros::bail!("Network worker exited unexpectedly");
                }
                Ok(AppMessage::Shutdown) | Err(_) => break,
            }
        }

        self.host.shutdown().await
    }
}
