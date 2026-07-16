mod screen_capture;
mod screen_layout;

pub(crate) use screen_capture::{
    KmsScreenCaptureManagerState, create_screen_capture_manager_state,
};
pub(crate) use screen_layout::{
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_screen_layout_manager_state,
};
