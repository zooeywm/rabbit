use crate::kernel::{geometry::PixelSize, screen_manager::ScreenId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenStreamRequestId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetScreenStreams {
    pub request_id: ScreenStreamRequestId,
    pub desired_streams: Vec<ScreenStreamRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenStreamRequest {
    pub screen_id: ScreenId,
    pub remote_display: RemoteDisplayMode,
    pub frame_size: PixelSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteDisplayMode {
    Preserve,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenStreamsConfigured {
    pub request_id: ScreenStreamRequestId,
    pub outcomes: Vec<ScreenResolutionOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenResolutionOutcome {
    pub screen_id: ScreenId,
    pub status: ScreenResolutionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenResolutionStatus {
    Configured(ResolutionResult),
    Failed {
        requested: PixelSize,
        actual: Option<PixelSize>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResolutionResult {
    Exact {
        applied: PixelSize,
    },
    Fallback {
        requested: PixelSize,
        applied: PixelSize,
    },
    Preserved {
        requested: PixelSize,
        actual: PixelSize,
    },
}
