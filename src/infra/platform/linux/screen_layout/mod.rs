mod gnome;
mod niri;

pub(crate) use gnome::{GnomeScreenLayoutManager, GnomeScreenLayoutManagerState};
pub(crate) use niri::{NiriScreenLayoutManager, NiriScreenLayoutManagerState};

pub(crate) fn create_niri_screen_layout_manager_state() -> eros::Result<NiriScreenLayoutManagerState>
{
    NiriScreenLayoutManagerState::new()
}

pub(crate) fn create_gnome_screen_layout_manager_state()
-> eros::Result<GnomeScreenLayoutManagerState> {
    GnomeScreenLayoutManagerState::new()
}
