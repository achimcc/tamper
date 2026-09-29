//! An interrupted run cleans up after itself.
//!
//! Up to 0.3.1 a SIGINT killed tamper where it stood: the throwaway tree
//! stayed in `$XDG_RUNTIME_DIR/tamper/…/run-<pid>/` and in `git worktree
//! list`, and the next run did not clear it, because the directory still
//! existed (audit 3, CD-8). Now SIGINT, SIGTERM and SIGHUP are caught, the
//! run's own trees are removed, and the process exits with 128 + signal.
//! A run killed with SIGKILL cannot do that — `tree::sweep_dead_runs` at the
//! next start is the second half.
//!
//! No crate for three signals: the handler only stores the number (the one
//! thing a signal handler may safely do), and a watcher thread does the
//! work. A second signal while the cleanup runs ends the process at once.

use std::sync::atomic::{AtomicI32, Ordering};
use std::time::Duration;

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGTERM: i32 = 15;

static RECEIVED: AtomicI32 = AtomicI32::new(0);

unsafe extern "C" {
    fn signal(signum: i32, handler: extern "C" fn(i32)) -> usize;
    fn _exit(status: i32) -> !;
}

extern "C" fn on_signal(sig: i32) {
    if RECEIVED.swap(sig, Ordering::SeqCst) != 0 {
        // SAFETY: _exit is async-signal-safe; this is the second signal.
        unsafe { _exit(128 + sig) }
    }
}

/// Has a stop been asked for? Workers check this before they create the
/// next tree, so the cleanup is not raced by a fresh one.
pub fn requested() -> bool {
    RECEIVED.load(Ordering::SeqCst) != 0
}

/// Catch SIGINT, SIGTERM and SIGHUP; on the first one run `cleanup` and
/// exit with 128 + signal.
pub fn install(cleanup: impl FnOnce() + Send + 'static) {
    for sig in [SIGHUP, SIGINT, SIGTERM] {
        // SAFETY: the handler only touches an atomic (or calls _exit).
        unsafe {
            signal(sig, on_signal);
        }
    }
    std::thread::spawn(move || {
        loop {
            let sig = RECEIVED.load(Ordering::SeqCst);
            if sig != 0 {
                cleanup();
                std::process::exit(128 + sig);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
}
