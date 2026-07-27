//! Process-wide graceful shutdown request.
//!
//! First SIGINT/SIGTERM marks shutdown requested; a second signal force-exits
//! so a hung finalize cannot trap the process. Shells (GUI / headless / record)
//! subscribe and run their own cleanup.

use std::{
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::Duration,
};

static INSTALLED: AtomicBool = AtomicBool::new(false);
static REQUESTED: AtomicBool = AtomicBool::new(false);
static SIGNAL_COUNT: AtomicU32 = AtomicU32::new(0);

/// Install OS signal handlers once. Idempotent and safe from any thread.
pub fn install() {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }

    #[cfg(unix)]
    install_unix_handlers();
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

#[cfg(unix)]
fn install_unix_handlers() {
    // SAFETY: handler only touches atomics and may call `_exit` (async-signal-safe).
    unsafe extern "C" fn on_stop_signal(_: libc::c_int) {
        let n = SIGNAL_COUNT.fetch_add(1, Ordering::SeqCst);
        if n >= 1 {
            unsafe { libc::_exit(130) };
        }
        REQUESTED.store(true, Ordering::SeqCst);
    }

    unsafe {
        // On Linux glibc, `sighandler_t` is a raw address-sized integer.
        let handler = on_stop_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
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
