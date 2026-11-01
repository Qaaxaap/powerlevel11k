//! Engine diagnostics log.
//!
//! The log lives under the engine state directory, not `/tmp`: the bug report template asks
//! for it, and it should survive long enough for a user to attach it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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

/// Rotate an oversized log. Call at startup before the first write; [`append`] also rotates
/// while the engine runs.
pub fn init() {
    rotate_if_oversized(&path());
}

/// Move the log to its one kept generation.
fn rotate(path: &Path) {
    let _ = fs::rename(path, path.with_extension("log.1"));
}

fn rotate_if_oversized(path: &Path) {
    if fs::metadata(path)
        .map(|meta| meta.len() > MAX_LOG_BYTES)
        .unwrap_or(false)
    {
        rotate(path);
    }
}

/// Append one line, rotating first when the file is already past the limit.
///
/// The check runs on every write, not only at startup: a long session with the debug level on
/// would otherwise grow the file without bound.
fn append(path: &Path, msg: &str) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    if file.metadata()?.len() > MAX_LOG_BYTES {
        drop(file);
        rotate(path);
        file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
    }
    writeln!(file, "{msg}")
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
    let _ = append(&path, msg);
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
    use super::*;

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

    /// A long session with the debug level on must not grow the log without bound: the write
    /// path rotates once the file passes the limit.
    #[test]
    fn a_write_rotates_an_oversized_log() {
        let dir = std::env::temp_dir().join(format!("p11k-log-rotate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p11k-engine.log");
        append(&path, &"x".repeat(MAX_LOG_BYTES as usize + 1)).unwrap();
        append(&path, "second line").unwrap();
        let rotated = dir.join("p11k-engine.log.1");
        assert!(rotated.exists(), "the oversized log must be rotated");
        assert_eq!(fs::read_to_string(&path).unwrap(), "second line\n");
        assert!(fs::read_to_string(&rotated).unwrap().starts_with('x'));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_small_log_is_not_rotated() {
        let dir = std::env::temp_dir().join(format!("p11k-log-small-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p11k-engine.log");
        append(&path, "one").unwrap();
        append(&path, "two").unwrap();
        assert!(!dir.join("p11k-engine.log.1").exists());
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        let _ = fs::remove_dir_all(&dir);
    }
}
