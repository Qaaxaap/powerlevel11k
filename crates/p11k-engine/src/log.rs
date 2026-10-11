//! Engine diagnostics log.

use std::fs;
use std::io::Write;

/// Engine diagnostics log, written to a fixed file to avoid polluting the pass-through
/// stream.
pub fn log(msg: &str) {
    if let Ok(mut f) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/p11k-engine.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}
