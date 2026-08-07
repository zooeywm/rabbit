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
        let create_app = composition::create_app();

        app::run(create_app)
    }
}
