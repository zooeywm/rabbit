use std::future::Future;

use crate::{
    app::{App, config::Config, gui::VideoViewStack},
    infra::{ConnectionEndpoint, WorkerReaper, WorkerReaperHandle},
    kernel::{
        frame_pipeline::FramePipelineManager,
        input::RemoteInputInjector,
        screen_capture::ScreenCaptureManager,
        screen_manager::ScreenLayoutManager,
        session::ReceivedVideoFrame,
        video_decoder::{DecodedVideoFrame, VideoDecoder},
        video_encoder::VideoEncoder,
    },
};

pub(crate) trait RemoteVideoStack: 'static {
    type Decoder: VideoDecoder<Input = ReceivedVideoFrame, Frame = Self::Frame>;
    type Frame: DecodedVideoFrame + Send + 'static;

    fn run_decoder<Inputs, PresentFrame, PresentFuture>(
        inputs: Inputs,
        present_frame: PresentFrame,
        enable_probing: bool,
    ) -> impl Future<Output = eros::Result<()>>
    where
        Inputs: futures_core::Stream<Item = eros::Result<ReceivedVideoFrame>> + Unpin,
        PresentFrame: FnMut(Self::Frame) -> PresentFuture,
        PresentFuture: Future<Output = eros::Result<()>>;
}

pub(crate) trait RunnableApp: ScreenLayoutManager + AsRef<Config> {
    fn run_app(&mut self) -> impl Future<Output = eros::Result<()>>;
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState> RunnableApp
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
where
    Self: ScreenLayoutManager,
{
    async fn run_app(&mut self) -> eros::Result<()> {
        self.run().await
    }
}

pub(crate) trait ApplicationStack: 'static
where
    <Self::App as FramePipelineManager>::Subscription: Unpin,
    <Self::ScreenStreamEncoder as VideoEncoder>::Packet: Into<bytes::Bytes>,
{
    type App: RunnableApp
        + ScreenCaptureManager
        + FramePipelineManager
        + AsRef<Config>
        + AsRef<ConnectionEndpoint>
        + 'static;
    type RemoteVideo: RemoteVideoStack;
    type RemoteVideoViewStack: VideoViewStack<
        Frame = <Self::RemoteVideo as RemoteVideoStack>::Frame,
    >;
    type ScreenStreamEncoder: VideoEncoder<Input = <Self::App as FramePipelineManager>::Frame>;
    type RemoteInputInjector: RemoteInputInjector;

    fn name() -> &'static str;

    fn create_app(
        config: Config,
        connection_endpoint: ConnectionEndpoint,
        worker_reaper: WorkerReaper,
        worker_reaper_handle: WorkerReaperHandle,
    ) -> eros::Result<Self::App>;

    fn create_remote_input_injector() -> Self::RemoteInputInjector;
}
