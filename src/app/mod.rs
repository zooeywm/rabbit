mod config;
pub(crate) mod container;
mod logging;
mod runtime;

use config::Config;
use container::{
    root::{
        AppContainer,
        outbound_port::{
            CapturerManager, CapturerManagerStateSpec, ConverterManager, ConverterManagerStateSpec,
            EncoderManager, EncoderManagerStateSpec,
        },
    },
    stream_pipeline::outbound_port::{EncoderFrameConverter, VideoEncoder},
};
use directories::ProjectDirs;
use eros::Context;
use runtime::AppRuntime;

use crate::app::container::root::{CapturedFrameFor, EncoderInputFor, StreamPipelineFor};

pub(crate) fn run<CapMgrSt, CvtMgrSt, EcdMgrSt>(
    app_constructor: impl FnOnce() -> eros::Result<AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>>
    + Send
    + 'static,
) -> eros::Result<()>
where
    CapMgrSt: CapturerManagerStateSpec,
    CvtMgrSt: ConverterManagerStateSpec,
    EcdMgrSt: EncoderManagerStateSpec,
    AppContainer<CapMgrSt, CvtMgrSt, EcdMgrSt>: CapturerManager<State = CapMgrSt>
        + ConverterManager<State = CvtMgrSt>
        + EncoderManager<State = EcdMgrSt>,
    StreamPipelineFor<CvtMgrSt, EcdMgrSt>: EncoderFrameConverter<CapturedFrame = CapturedFrameFor<CapMgrSt>>
        + VideoEncoder<EncoderInput = EncoderInputFor<CvtMgrSt, EcdMgrSt>>,
{
    let project_dirs = ProjectDirs::from("", "", "rabbit")
        .with_context(|| "Failed looking for app project dir")?;

    let config = Config::load(&project_dirs)?;
    let _logging_guard = logging::init(&project_dirs, &config.logging)?;

    let app_handle = AppRuntime::start(app_constructor)?;

    tracing::trace!("rabbit started");
    tracing::debug!("rabbit started");
    tracing::info!("rabbit started");
    tracing::warn!("rabbit started");
    tracing::error!("rabbit started");

    app_handle.shutdown()
}
