mod app;
mod composition;
mod domain;
mod infrastructure;
mod presentation;

#[derive(Default)]
pub struct RabbitApp;

impl RabbitApp {
    pub fn new() -> Self {
        Self
    }

    pub fn run(self) -> eros::Result<()> {
        let app_constructor = composition::compose_app();

        app::run(app_constructor, |_app_handle| Ok(()))
    }

    #[cfg(feature = "test-ui")]
    pub fn run_test_ui(self) -> eros::Result<()> {
        let app_constructor = composition::compose_app();

        app::run(app_constructor, presentation::test_ui::run)
    }
}
