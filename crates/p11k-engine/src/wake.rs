//! The wake pipe: one byte makes the main poll return.
//!
//! Signal handlers and the git worker use it. The write end is an atomic because a signal
//! handler cannot take a lock; `wake` therefore only performs an atomic load and one write,
//! both async-signal-safe.

use std::sync::atomic::{AtomicI32, Ordering};

/// Write end of the pipe. `-1` until the engine creates the pipe; `wake` then does nothing.
static WAKE_FD: AtomicI32 = AtomicI32::new(-1);

/// Publish the write end. Called once, before the handlers are installed and the worker starts.
pub fn install(fd: i32) {
    WAKE_FD.store(fd, Ordering::Relaxed);
}

/// Wake the main poll with one byte. Safe to call from a signal handler and from a worker thread.
pub fn wake() {
    let fd = WAKE_FD.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: writes one byte to a pipe the engine created; the fd is valid for the process
        // lifetime and the return value is intentionally ignored.
        unsafe {
            libc::write(fd, (&byte as *const u8).cast(), 1);
        }
    }
}
