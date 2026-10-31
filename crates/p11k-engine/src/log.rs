//! Engine diagnostics log.
//!
//! The log lives under the engine state directory, not `/tmp`: the bug report template asks
//! for it, and it should survive long enough for a user to attach it.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use crate::paths;

/// Rotate the log once it grows past this size. One generation is kept.
const MAX_LOG_BYTES: u64 = 1 << 20;

/// Severity of a log line. A line is written when its level is at or below the configured one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Off = 0,
    Error = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    pub fn parse(name: &str) -> Option<Level> {
        match name {
            "off" => Some(Level::Off),
            "error" => Some(Level::Error),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

pub fn path() -> PathBuf {
    paths::state_home().join("p11k-engine.log")
}

/// Rotate an oversized log. Call once at startup, before the first write.
pub fn init() {
    let path = path();
    let Ok(meta) = fs::metadata(&path) else {
        return;
    };
    if meta.len() > MAX_LOG_BYTES {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
}

/// Write one line when its level passes the current threshold.
pub fn log(level: Level, msg: &str) {
    if (level as u8) > LEVEL.load(Ordering::Relaxed) {
        return;
    }
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{msg}");
    }
}

/// Low-frequency engine events: startup, reload, recoverable errors.
pub fn info(msg: &str) {
    log(Level::Info, msg);
}

/// Per-prompt detail: geometry, redraws, resize nudges.
pub fn debug(msg: &str) {
    log(Level::Debug, msg);
}

#[cfg(test)]
mod tests {
    use super::Level;

    #[test]
    fn level_parses_the_accepted_names() {
        assert_eq!(Level::parse("off"), Some(Level::Off));
        assert_eq!(Level::parse("error"), Some(Level::Error));
        assert_eq!(Level::parse("info"), Some(Level::Info));
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
    }

    #[test]
    fn level_rejects_unknown_names() {
        assert_eq!(Level::parse("verbose"), None);
        assert_eq!(Level::parse(""), None);
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Off < Level::Error);
        assert!(Level::Error < Level::Info);
        assert!(Level::Info < Level::Debug);
    }
}
