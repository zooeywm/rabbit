//! Application services — pure orchestration above the kernel, below the GUI.
//!
//! Handlers in `app::gui::application::update` should prefer these services for
//! domain decisions so UI message routing stays thin. Services must not depend
//! on Slint, `RootMessage`, or view publishers.

pub mod host_stream;
pub mod session_catalog;
