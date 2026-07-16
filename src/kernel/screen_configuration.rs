use crate::kernel::screen_manager::ScreenId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PixelSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenConfigurationRequestId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetScreenResolutions {
    pub request_id: ScreenConfigurationRequestId,
    pub changes: Vec<ScreenResolutionRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScreenResolutionRequest {
    pub screen_id: ScreenId,
    pub mode: RemoteDisplayMode,
    pub requested: PixelSize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteDisplayMode {
    Preserve,
    MatchRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScreenResolutionCompleted {
    pub request_id: ScreenConfigurationRequestId,
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
