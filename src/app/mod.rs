mod config;
pub(crate) mod container;
mod logging;
mod runtime;

use config::Config;
use container::{
    root::{
        AppContainer, TransporterStateFor,
        outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec, MetricsRecorder, TransporterConstructor,
            TransporterConstructorStateSpec, TransporterMetricsRecorder,
        },
    },
    stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
    transporter::{TransporterContainer, outbound_port::Transporter},
};
use directories::ProjectDirs;
use eros::Context;
use runtime::AppRuntime;

use container::root::{CapturedFrameFor, EncodedBufferFor, EncoderInputFor, StreamPipelineFor};

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
    TransporterContainer<TransporterStateFor<TprCstSt>>: Transporter<EncodedBuffer = EncodedBufferFor<CvtMgrSt, EcdMgrSt>>
        + TransporterMetricsRecorder,
    AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt, TprCstSt>: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>
        + TransporterConstructor<State = TprCstSt>,
    StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
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
