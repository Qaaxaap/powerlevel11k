//! Untracked-cache probe (mirrors `check_dir_mtime.cc` CheckDirMtime).
//!
//! git's untracked cache relies on a filesystem behavior: changes inside a
//! subdirectory update the parent directory's mtime. Not all filesystems
//! guarantee this (some network/overlay filesystems don't), and enabling
//! the cache wrongly causes untracked files to be missed — correctness
//! wins over performance.
//!
//! The probe (CheckDirMtime): mkdtemp under gitdir, create `a` and `b`
//! subdirs and record their mtimes, sleep 1s (mtime resolution), then mkdir
//! inside `a` and touch a file inside `b`; both parent mtimes changing means
//! the filesystem is supported.
//!
//! The probe runs on a background thread so the first request is not
//! blocked; until it finishes the cache is treated as supported (git's
//! optimistic default). Leftover probe dirs (`.gitstatus.` prefix) are
//! removed so a crashed previous run cannot interfere.

use crate::sys;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct UntrackedCache {
    /// Probe result; true until the probe finishes (git's optimistic default).
    supported: Arc<AtomicBool>,
}

impl UntrackedCache {
    /// Start the background probe and return immediately. gitdir = the
    /// repo's .git directory.
    pub fn start_probe(gitdir: &Path) -> UntrackedCache {
        let supported = Arc::new(AtomicBool::new(true));
        let flag = supported.clone();
        let gitdir = gitdir.to_path_buf();
        std::thread::spawn(move || {
            let ok = probe_support(&gitdir);
            flag.store(ok, Ordering::Relaxed);
        });
        UntrackedCache { supported }
    }

    /// Current probe result (true while the probe is unfinished).
    pub fn enabled(&self) -> bool {
        self.supported.load(Ordering::Relaxed)
    }
}

/// Probe body (mirrors CheckDirMtime). Any failure → false (safe degrade).
fn probe_support(gitdir: &Path) -> bool {
    // Remove probe dirs older than 10s (mirrors RemoveStaleDirs).
    remove_stale_dirs(gitdir);

    // mkdtemp: gitdir/.gitstatus.XXXXXX
    let Ok(tmp) = sys::mkdtemp(&gitdir.join(".gitstatus.XXXXXX")) else {
        return false;
    };
    // RAII cleanup: remove the probe dir on every exit path.
    let _cleanup = Cleanup(tmp.clone());

    let a_dir = tmp.join("a");
    let b_dir = tmp.join("b");
    if sys::mkdir(&a_dir, 0o755).is_err() {
        return false;
    }
    let Some(a_st) = lstat(&a_dir) else {
        return false;
    };
    if sys::mkdir(&b_dir, 0o755).is_err() {
        return false;
    }
    let Some(b_st) = lstat(&b_dir) else {
        return false;
    };

    // Ensure mtime resolution (matches the original `while (sleep(1))`).
    std::thread::sleep(std::time::Duration::from_secs(1));

    // a/1 subdir: does creating a subdir update the parent's mtime?
    let a1 = a_dir.join("1");
    if sys::mkdir(&a1, 0o755).is_err() {
        return false;
    }
    if !stat_changed(&a_dir, &a_st) {
        return false;
    }

    // b/1 file: does creating a file update the parent's mtime?
    // (matches Touch = creat 0444)
    let b1 = b_dir.join("1");
    let Ok(fd) = sys::create_file(&b1, 0o444) else {
        return false;
    };
    if let Err(e) = sys::close(fd) {
        log::debug!("close of the probe file failed: {e}");
    }
    stat_changed(&b_dir, &b_st)
}

/// RAII cleanup: recursively remove the probe dir on drop
/// (rmdir chain: a/1, a, b/1, b, root).
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let root = &self.0;
        let a1 = root.join("a/1");
        let a = root.join("a");
        let b1 = root.join("b/1");
        let b = root.join("b");
        for p in [&b1, &b, &a1, &a, root] {
            let is_dir = p != &b1;
            let r = if is_dir {
                sys::rmdir(p)
            } else {
                sys::unlink(p)
            };
            if let Err(e) = r {
                log::debug!("probe cleanup of {} failed: {e}", p.display());
            }
        }
    }
}

/// lstat → (mtime_sec, mtime_nsec).
fn lstat(path: &Path) -> Option<(i64, i64)> {
    sys::lstat(path)
        .ok()
        .map(|st| (st.st_mtime, st.st_mtime_nsec))
}

/// Whether the mtime changed (StatChanged: negation of StatEq).
fn stat_changed(path: &Path, prev: &(i64, i64)) -> bool {
    lstat(path).map(|cur| &cur != prev).unwrap_or(false)
}

/// Remove probe dirs older than 10s (mirrors RemoveStaleDirs): readdir
/// gitdir, find `.gitstatus.`-prefixed dirs with mtime older than now-10s,
/// and delete them in reverse order (a/1, a, b/1, b, root).
fn remove_stale_dirs(gitdir: &Path) {
    let Ok(dir_fd) = sys::open_dir(gitdir) else {
        return;
    };
    let now = sys::time_seconds();
    let entries = match sys::read_dir(dir_fd) {
        Ok(entries) => entries,
        Err(e) => {
            log::debug!("cannot list the gitdir for stale probes: {e}");
            let _ = sys::close(dir_fd);
            return;
        }
    };
    for entry in entries {
        if !entry.name.starts_with(b".gitstatus.") {
            continue;
        }
        // `read_dir` keeps the NUL for the scanner; a path does not take it.
        let name = &entry.name[..entry.name.len() - 1];
        let Ok(st) = sys::fstatat(dir_fd, &entry.name) else {
            continue;
        };
        if st.st_mtime + 10 > now {
            continue;
        }
        let mut path = gitdir.to_path_buf();
        path.push(OsStr::from_bytes(name));
        // Reverse-order delete: a/1, a, b/1, b, root. A path may already be gone, which is not
        // an error here.
        ignore_missing(sys::rmdir(&path.join("a/1")));
        ignore_missing(sys::rmdir(&path.join("a")));
        ignore_missing(sys::unlink(&path.join("b/1")));
        ignore_missing(sys::rmdir(&path.join("b")));
        ignore_missing(sys::rmdir(&path));
    }
    if let Err(e) = sys::close(dir_fd) {
        log::debug!("close of the gitdir failed: {e}");
    }
}

/// Cleanup of a stale probe path: absent is the normal case after a crash, other errors go to
/// the debug log.
fn ignore_missing(result: std::io::Result<()>) {
    if let Err(e) = result
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::debug!("stale probe cleanup failed: {e}");
    }
}
