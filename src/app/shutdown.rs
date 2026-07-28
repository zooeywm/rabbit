//! Process-wide graceful shutdown request.
//!
//! First SIGINT/SIGTERM marks shutdown requested; a second signal force-exits
//! so a hung finalize cannot trap the process. Shells (GUI / headless / record)
//! subscribe and run their own cleanup.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Install OS signal handlers once. Idempotent and safe from any thread.
pub fn install() {
    if !INSTALLED.swap(true, Ordering::SeqCst) {
        super::platform::install_shutdown_handlers();
    }
}

/// Mark shutdown requested (e.g. Enter to stop recording, or UI close path).
pub fn request() {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Whether a graceful shutdown has been requested.
pub fn is_requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// Block until [`request`] or a stop signal. Installs handlers if needed.
pub fn wait() {
    install();
    while !is_requested() {
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Subscribe to the next shutdown request. The receiver is `Send` so it can be
/// awaited on the app runtime after a background thread observes the signal.
///
/// Only one notification is delivered (bounded channel capacity 1).
pub fn subscribe() -> flume::Receiver<()> {
    install();
    let (tx, rx) = flume::bounded(1);
    std::thread::Builder::new()
        .name("rabbit-shutdown".into())
        .spawn(move || {
            wait();
            let _ = tx.send(());
        })
        .expect("Failed to spawn rabbit-shutdown watcher");
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_observable() {
        // Do not install real signals in unit tests; only exercise the flag API.
        REQUESTED.store(false, Ordering::SeqCst);
        assert!(!is_requested());
        request();
        assert!(is_requested());
        // Leave requested true so concurrent tests that wait would see it;
        // process is short-lived.
    }
}
