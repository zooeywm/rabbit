use crate::{
    app::{App, config::Config},
    infra::ConnectionEndpoint,
};

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState> AsRef<Config>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &Config {
        &self.config
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    AsRef<ConnectionEndpoint>
    for App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    fn as_ref(&self) -> &ConnectionEndpoint {
        &self.connection_endpoint
    }
}
