#[cfg_attr(target_os = "linux", path = "platform/linux/mod.rs")]
#[cfg_attr(target_os = "windows", path = "platform/windows/mod.rs")]
mod platform;
mod rayon_thread_pool;
mod transport;
pub(crate) mod unsync_queue;

pub(crate) use platform::{
    NiriScreenLayoutManager, NiriScreenLayoutManagerState, create_screen_layout_manager_state,
};
pub(crate) use rayon_thread_pool::{RayonThreadPool, RayonThreadPoolState};
