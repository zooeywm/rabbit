mod app;
mod composition;
mod domain;
mod infrastructure;
mod presentation;

pub fn run() -> eros::Result<()> {
    composition::run()
}
