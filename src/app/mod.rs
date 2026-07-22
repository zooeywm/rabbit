pub(crate) mod config;

mod deps;
mod gui;
mod logging;
mod model;
mod screen_stream;

use tracing::{info, warn};

use crate::{
    app::config::Config,
    infra::{ConnectionEndpoint, WorkerReaper},
    kernel::screen_manager::ScreenLayoutManager,
};

pub(crate) use gui::run as run_gui;
pub(crate) use logging::{LoggerGuard, init_logging};

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
    /// Runs the current application lifecycle.
    ///
    /// The MVP currently reports the detected screen layout. The persistent
    /// rendering and application event loop will be added here later.
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
