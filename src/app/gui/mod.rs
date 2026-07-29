//! GUI shell, view bindings, and application message loop.

mod application;
mod input;
mod state;
mod video_view;
mod view;

use std::time::Duration;

use eros::Context as _;

use crate::app::{
    config::Config,
    gui::{application::run_root, view::Gui},
    platform::ApplicationStack,
};

pub(crate) use video_view::VideoViewStack;
pub(crate) use view::RabbitWindow;

pub(crate) fn run<Stack>(config: Config) -> eros::Result<()>
where
    Stack: ApplicationStack,
{
    let probe_interval = Duration::from_millis(config.video.probe_interval_ms);
    let (gui, publisher, intents) =
        Gui::<Stack::RemoteVideoViewStack>::new(probe_interval, config.input.pointer_mode)?;
    let thread_publisher = publisher.clone();
    let application_thread = std::thread::Builder::new()
        .name("rabbit-app".to_string())
        .spawn(move || {
            let result = (|| {
                let runtime = compio::runtime::Runtime::new()
                    .context("Failed to create the Rabbit Compio runtime")?;
                runtime.block_on(run_root::<Stack>(config, thread_publisher.clone(), intents))
            })();

            if result.is_err()
                && let Err(error) = thread_publisher.quit()
            {
                eprintln!("Failed to stop the Slint event loop after an App error: {error}");
            }
            result
        })
        .context("Failed to start the Rabbit App thread")?;

    let gui_result = gui.run();
    gui.request_close();
    let application_result = match application_thread.join() {
        Ok(result) => result,
        Err(_) => eros::bail!("Rabbit App thread terminated unexpectedly"),
    };

    gui_result?;
    application_result
}
