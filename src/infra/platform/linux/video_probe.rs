use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct HostVideoFrameProbe {
    pub(crate) frame_id: u64,
    pub(crate) pts_ns: u64,
    pub(crate) vblank_wait_started: Instant,
    pub(crate) capture_started: Instant,
    pub(crate) capture_completed: Instant,
    pub(crate) gpu_received: Option<Instant>,
    pub(crate) gpu_submitted: Option<Instant>,
    pub(crate) pipeline_ready: Option<Instant>,
    pub(crate) encoder_submitted: Option<Instant>,
    pub(crate) vpp_entered: Option<Instant>,
    pub(crate) vpp_completed: Option<Instant>,
}

impl HostVideoFrameProbe {
    pub(crate) fn new(
        frame_id: u64,
        epoch: Instant,
        vblank_wait_started: Instant,
        capture_started: Instant,
        capture_completed: Instant,
    ) -> Self {
        let pts_ns = duration_ns(capture_started.duration_since(epoch));

        Self {
            frame_id,
            pts_ns,
            vblank_wait_started,
            capture_started,
            capture_completed,
            gpu_received: None,
            gpu_submitted: None,
            pipeline_ready: None,
            encoder_submitted: None,
            vpp_entered: None,
            vpp_completed: None,
        }
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}
