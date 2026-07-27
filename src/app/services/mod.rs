//! Application services — pure orchestration above the kernel, below the GUI.
//!
//! Handlers in `app::gui::application::update` should prefer these services for
//! domain decisions so UI message routing stays thin. Services must not depend
//! on the GUI shell, view publishers, or presentation frameworks.

pub mod host_stream;
pub mod session_catalog;
