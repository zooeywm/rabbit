//! Architecture invariant tests (compile-time discipline as runtime checks).
//!
//! These tests lock the textbook layering so regressions fail in CI rather than
//! review folklore.

#![cfg(test)]

use std::{
    fs,
    path::{Path, PathBuf},
};

fn crate_src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(entries) = fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files
}

fn source_imports(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn kernel_does_not_depend_on_app_or_infra() {
    let kernel = crate_src().join("kernel");
    for file in rust_sources_under(&kernel) {
        let text = source_imports(&file);
        assert!(
            !text.contains("crate::app::") && !text.contains("crate::infra::"),
            "{} must not import app or infra",
            file.display()
        );
    }
}

#[test]
fn services_and_runtime_do_not_depend_on_slint_or_gui_messages() {
    for rel in ["app/services", "app/runtime"] {
        let dir = crate_src().join(rel);
        for file in rust_sources_under(&dir) {
            let text = source_imports(&file);
            for forbidden in ["slint::", "RootMessage", "GuiIntent", "crate::app::gui::"] {
                assert!(
                    !text.contains(forbidden),
                    "{} must not reference {forbidden}",
                    file.display()
                );
            }
        }
    }
}

#[test]
fn protocol_constants_are_single_sourced_in_transport_and_screen_id() {
    // Behavioral lock; complements kernel::protocol unit tests.
    use crate::kernel::{
        protocol::{CONTROL_CHANNEL_ID, MAX_VIDEO_SCREEN_ID},
        screen_manager::ScreenId,
        transport::TransportChannel,
    };

    assert_eq!(u8::from(TransportChannel::Control), CONTROL_CHANNEL_ID);
    assert_eq!(ScreenId::MAX, MAX_VIDEO_SCREEN_ID);
}
