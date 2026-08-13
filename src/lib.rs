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
        let _app = app::run()?;

        std::thread::park();
        Ok(())
    }

    #[cfg(feature = "test-ui")]
    pub fn run_test_ui(self) -> eros::Result<()> {
        presentation::test_ui::run(app::run()?)
    }
}
