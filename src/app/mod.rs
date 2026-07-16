pub(crate) mod config;

mod deps;
mod logging;

use tracing::{info, warn};

use crate::{app::config::Config, kernel::screen_manager::ScreenLayoutManager};

/// Root application state and dependency container.
pub struct App<ScreenLayoutManagerState> {
    config: Config,
    screen_layout_manager_state: ScreenLayoutManagerState,
}

impl<ScreenLayoutManagerState> App<ScreenLayoutManagerState> {
    /// Creates the application and all persistent application services.
    pub(crate) fn new(screen_layout_manager: ScreenLayoutManagerState) -> eros::Result<Self> {
        let config = Config::new()?;
        logging::init_logging(&config)?;

        Ok(Self {
            config,
            screen_layout_manager_state: screen_layout_manager,
        })
    }
}

impl<ScreenLayoutManagerState> App<ScreenLayoutManagerState>
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
