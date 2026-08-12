use crate::{
    app::container::{
        packetization::PacketizerContainer, root::outbound_port::MetricsRecorder,
        screen_capture::ScreenCaptureContainer, stream_pipeline::StreamPipelineContainer,
    },
    domain::stream::models::vo::FrameId,
    infrastructure::common::OpenTelemetryMetricsRecorderImpl,
};

impl<State> MetricsRecorder for ScreenCaptureContainer<State> {
    fn register_metrics_target(&self) {
        MetricsRecorder::register_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn unregister_metrics_target(&self) {
        MetricsRecorder::unregister_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn record_captured_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_captured_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_converted_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_converted_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_encoded_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_encoded_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_packetized_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_packetized_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }
}

impl<CvtSt, EcdSt> MetricsRecorder for StreamPipelineContainer<CvtSt, EcdSt> {
    fn register_metrics_target(&self) {
        MetricsRecorder::register_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn unregister_metrics_target(&self) {
        MetricsRecorder::unregister_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn record_captured_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_captured_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_converted_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_converted_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_encoded_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_encoded_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_packetized_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_packetized_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }
}

impl<State> MetricsRecorder for PacketizerContainer<State> {
    fn register_metrics_target(&self) {
        MetricsRecorder::register_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn unregister_metrics_target(&self) {
        MetricsRecorder::unregister_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn record_captured_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_captured_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_converted_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_converted_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_encoded_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_encoded_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }

    fn record_packetized_frame(&self, frame_id: FrameId, duration: std::time::Duration) {
        MetricsRecorder::record_packetized_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            frame_id,
            duration,
        );
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "fake")] {
        #[path = "platform/fake.rs"]
        mod selected_platform;
    } else if #[cfg(target_os = "linux")] {
        #[path = "platform/linux.rs"]
        mod selected_platform;
    } else {
        #[path = "platform/unsupported.rs"]
        mod selected_platform;
    }
}

pub(super) fn compose_app() -> impl FnOnce() -> eros::Result<(
    selected_platform::PlatformApp,
    crate::infrastructure::common::MetricsRuntime,
)> + Send
+ 'static {
    let platform_app_constructor = selected_platform::compose_app();

    move || {
        let app = platform_app_constructor()?;
        let metrics_runtime = crate::infrastructure::common::init_metrics();

        Ok((app, metrics_runtime))
    }
}
