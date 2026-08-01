mod app;
mod domain;
mod infrastructure;
mod presentation;

pub fn run() -> eros::Result<()> {
    app::run()
}
