//! Shared application runtime policies — GUI and headless adapters only.
//!
//! Nothing in this module depends on the presentation shell or view publishers.
//! Entry points call these policies, then perform I/O.

pub mod controller_policy;
pub mod host_control;
pub mod host_policy;
pub mod host_stream_launch;
pub mod host_stream_lifecycle;
pub mod session_lifecycle;
