mod config;
pub(crate) mod container;
mod logging;
mod runtime;

use config::Config;
use container::{
    host::{
        CapturedFrameFor, EncodedBufferFor, EncoderInputFor, HostContainer, HostStreamPipelineFor,
        outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec,
        },
    },
    host_stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
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

pub(crate) fn run<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt, AppRuntimeGuard>(
    app_constructor: impl FnOnce() -> eros::Result<(
        AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>,
        AppRuntimeGuard,
    )> + Send
    + 'static,
    run_presentation: impl FnOnce(AppHandle) -> eros::Result<()>,
) -> eros::Result<()>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    TprCstSt: TransporterConstructorStateSpec,
    NetworkContainer<TransporterStateFor<TprCstSt>>:
        Transporter<EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>> + NetworkMetricsRecorder,
    HostContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>: TransporterConstructor<State = TprCstSt>,
    HostStreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>,
    EncodedBufferFor<CvtMgrSt, EcdMgrSt>: Send + 'static,
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
