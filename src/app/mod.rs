pub(crate) mod config;

mod deps;
mod gui;
pub(crate) mod headless;
mod logging;
mod model;
mod platform;
mod screen_stream;
pub(crate) mod services;

use tracing::{info, warn};

use crate::{
    app::config::Config,
    infra::{ConnectionEndpoint, WorkerReaper},
    kernel::screen_manager::ScreenLayoutManager,
};

pub(crate) use logging::{LoggerGuard, init_logging};
pub(crate) use platform::run as run_gui;
pub(crate) use platform::run_headless;

/// Root application state and dependency container.
pub struct App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState> {
    config: Config,
    screen_layout_manager_state: ScreenLayoutManagerState,
    screen_capture_manager_state: ScreenCaptureManagerState,
    frame_pipeline_manager_state: FramePipelineManagerState,
    connection_endpoint: ConnectionEndpoint,
    _worker_reaper: WorkerReaper,
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
{
    /// Creates the application and all persistent application services.
    pub(crate) fn new(
        config: Config,
        screen_layout_manager_state: ScreenLayoutManagerState,
        screen_capture_manager_state: ScreenCaptureManagerState,
        frame_pipeline_manager_state: FramePipelineManagerState,
        connection_endpoint: ConnectionEndpoint,
        worker_reaper: WorkerReaper,
    ) -> Self {
        Self {
            config,
            screen_layout_manager_state,
            screen_capture_manager_state,
            frame_pipeline_manager_state,
            connection_endpoint,
            _worker_reaper: worker_reaper,
        }
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
    App<ScreenLayoutManagerState, ScreenCaptureManagerState, FramePipelineManagerState>
where
    Self: ScreenLayoutManager,
{
    /// Runs platform service startup for the selected application stack.
    ///
    /// Refreshes and logs the local screen layout. Session orchestration, GUI
    /// messaging, and media streaming run in `app::gui::application` after this
    /// bootstrap returns.
    pub(crate) async fn run(&mut self) -> eros::Result<()> {
        let screens = self.screens();

        if screens.is_empty() {
            warn!("No screens detected");
            return Ok(());
        }

        info!("Detected screens:{:?}", screens);

        let primary_screen = self.primary_screen()?;

        info!("Selected primary screen:{:?}", primary_screen);

        Ok(())
    }
}
