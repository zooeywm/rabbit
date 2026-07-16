pub(crate) mod config;

mod deps;
mod gui;
mod logging;

use tracing::{info, warn};

use crate::{
    app::config::Config,
    infra::{QuicEndpoint, RayonThreadPoolState},
    kernel::screen_manager::ScreenLayoutManager,
};

pub(crate) use gui::RootComponent;
pub(crate) use logging::init_logging;

/// Root application state and dependency container.
pub struct App<ScreenLayoutManagerState, ScreenCaptureManagerState> {
    config: Config,
    screen_layout_manager_state: ScreenLayoutManagerState,
    screen_capture_manager_state: ScreenCaptureManagerState,
    rayon_thread_pool_state: RayonThreadPoolState,
    quic_endpoint: QuicEndpoint,
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState>
    App<ScreenLayoutManagerState, ScreenCaptureManagerState>
{
    /// Creates the application and all persistent application services.
    pub(crate) fn new(
        config: Config,
        screen_layout_manager_state: ScreenLayoutManagerState,
        screen_capture_manager_state: ScreenCaptureManagerState,
        rayon_thread_pool_state: RayonThreadPoolState,
        quic_endpoint: QuicEndpoint,
    ) -> Self {
        Self {
            config,
            screen_layout_manager_state,
            screen_capture_manager_state,
            rayon_thread_pool_state,
            quic_endpoint,
        }
    }
}

impl<ScreenLayoutManagerState, ScreenCaptureManagerState>
    App<ScreenLayoutManagerState, ScreenCaptureManagerState>
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
