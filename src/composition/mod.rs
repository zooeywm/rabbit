use crate::{
    app::container::{
        ResourceUsage,
        host::outbound_port::MetricsRecorder,
        host_stream_pipeline::HostStreamPipelineContainer,
        network::{NetworkContainer, NetworkMetricsHandle, outbound_port::NetworkMetricsRecorder},
        screen_capture::ScreenCaptureContainer,
    },
    domain::stream::models::vo::{CaptureSourceId, FrameId, StreamId},
    infrastructure::common::OpenTelemetryMetricsRecorderImpl,
};

impl<State> MetricsRecorder for ScreenCaptureContainer<State> {
    fn register_metrics_target(&self) {
        MetricsRecorder::register_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn unregister_metrics_target(&self) {
        MetricsRecorder::unregister_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn register_capture_pool_usage(&self, usage: ResourceUsage) {
        MetricsRecorder::register_capture_pool_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            usage,
        );
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
}

impl<CvtSt, EcdSt> MetricsRecorder for HostStreamPipelineContainer<CvtSt, EcdSt> {
    fn register_metrics_target(&self) {
        MetricsRecorder::register_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn unregister_metrics_target(&self) {
        MetricsRecorder::unregister_metrics_target(OpenTelemetryMetricsRecorderImpl::inj_ref(self));
    }

    fn register_capture_pool_usage(&self, usage: ResourceUsage) {
        MetricsRecorder::register_capture_pool_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            usage,
        );
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
}

impl<State> NetworkMetricsRecorder for NetworkContainer<State> {
    fn register_network_queue_usage(&self, usage: ResourceUsage) {
        NetworkMetricsRecorder::register_network_queue_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            usage,
        );
    }

    fn unregister_network_queue_usage(&self) {
        NetworkMetricsRecorder::unregister_network_queue_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
        );
    }

    fn record_packetized_frame(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        frame_id: FrameId,
        duration: std::time::Duration,
    ) {
        NetworkMetricsRecorder::record_packetized_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            capture_source_id,
            stream_id,
            frame_id,
            duration,
        );
    }

    fn record_sent_bytes(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    ) {
        NetworkMetricsRecorder::record_sent_bytes(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            capture_source_id,
            stream_id,
            bytes,
        );
    }
}

impl NetworkMetricsRecorder for NetworkMetricsHandle {
    fn register_network_queue_usage(&self, usage: ResourceUsage) {
        NetworkMetricsRecorder::register_network_queue_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            usage,
        );
    }

    fn unregister_network_queue_usage(&self) {
        NetworkMetricsRecorder::unregister_network_queue_usage(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
        );
    }

    fn record_packetized_frame(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        frame_id: FrameId,
        duration: std::time::Duration,
    ) {
        NetworkMetricsRecorder::record_packetized_frame(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            capture_source_id,
            stream_id,
            frame_id,
            duration,
        );
    }

    fn record_sent_bytes(
        &self,
        capture_source_id: CaptureSourceId,
        stream_id: StreamId,
        bytes: usize,
    ) {
        NetworkMetricsRecorder::record_sent_bytes(
            OpenTelemetryMetricsRecorderImpl::inj_ref(self),
            capture_source_id,
            stream_id,
            bytes,
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
