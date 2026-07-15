mod niri;

pub(crate) use niri::{NiriScreenLayoutManager, NiriScreenLayoutManagerState};

/// Creates the screen-layout manager state selected for Linux.
pub(crate) fn create_screen_layout_manager_state() -> eros::Result<NiriScreenLayoutManagerState> {
    NiriScreenLayoutManagerState::new()
}
