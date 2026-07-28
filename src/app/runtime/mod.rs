//! Shared application runtime orchestration for GUI and headless shells.
//!
//! Nothing in this module depends on presentation components or shell message types.
//! Shells only adapt runtime effects to their own queues and visible state.

pub mod controller_policy;
pub mod host_control;
pub mod host_policy;
pub mod host_stream_launch;
pub mod host_stream_lifecycle;
pub mod session_lifecycle;
