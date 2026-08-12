mod config;
pub(crate) mod container;
mod logging;
mod runtime;

use config::Config;
use container::{
    client::inbound_port::ClientApplication,
    host::inbound_port::HostApplication,
    network::{NetworkContainer, outbound_port::Transporter},
    root::{
        AppContainer, TransporterStateFor,
        outbound_port::{
            NetworkMetricsRecorder, TransporterConstructor, TransporterConstructorStateSpec,
        },
    },
};
use directories::ProjectDirs;
use eros::Context;
use runtime::AppRuntime;

pub(crate) use runtime::AppHandle;

pub(crate) fn run<Host, Client, NetworkConstructorState, AppRuntimeGuard>(
    app_constructor: impl FnOnce() -> eros::Result<(
        AppContainer<Host, Client, NetworkConstructorState>,
        AppRuntimeGuard,
    )> + Send
    + 'static,
    run_presentation: impl FnOnce(AppHandle) -> eros::Result<()>,
) -> eros::Result<()>
where
    Host: HostApplication,
    Client: ClientApplication,
    NetworkConstructorState: TransporterConstructorStateSpec,
    NetworkContainer<TransporterStateFor<NetworkConstructorState>>:
        Transporter<EncodedBuffer = Host::EncodedBuffer> + NetworkMetricsRecorder,
    AppContainer<Host, Client, NetworkConstructorState>:
        TransporterConstructor<State = NetworkConstructorState>,
{
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;
    let app_runtime = AppRuntime::start(app_constructor)?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    let presentation_result = run_presentation(app_runtime.handle());
    let shutdown_result = app_runtime.shutdown();

    presentation_result?;
    shutdown_result
}
