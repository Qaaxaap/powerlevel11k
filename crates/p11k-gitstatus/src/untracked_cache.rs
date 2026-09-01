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

    /// Dir mtime unchanged (the cache is fresh) when it equals the record.
    pub fn is_fresh(&self, cached: Option<(i64, i64)>, cur: (i64, i64)) -> bool {
        self.enabled() && cached == Some(cur)
    }
}

/// Probe body (mirrors CheckDirMtime). Any failure → false (safe degrade).
fn probe_support(gitdir: &Path) -> bool {
    // Remove probe dirs older than 10s (mirrors RemoveStaleDirs).
    remove_stale_dirs(gitdir);

    // mkdtemp: gitdir/.gitstatus.XXXXXX
    let Some(tmp) = mkdtemp(gitdir.join(".gitstatus.XXXXXX")) else {
        return false;
    };
    // RAII cleanup: remove the probe dir on every exit path.
    let _cleanup = Cleanup(tmp.clone());

    let a_dir = tmp.join("a");
    let b_dir = tmp.join("b");
    // SAFETY: path is NUL-terminated via nul(); standard mkdir semantics.
    if unsafe { libc::mkdir(nul(&a_dir).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    let Some(a_st) = lstat(&a_dir) else {
        return false;
    };
    // SAFETY: as above.
    if unsafe { libc::mkdir(nul(&b_dir).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    let Some(b_st) = lstat(&b_dir) else {
        return false;
    };

    // Ensure mtime resolution (matches the original `while (sleep(1))`).
    std::thread::sleep(std::time::Duration::from_secs(1));

    // a/1 subdir: does creating a subdir update the parent's mtime?
    let a1 = a_dir.join("1");
    // SAFETY: as above.
    if unsafe { libc::mkdir(nul(&a1).as_ptr().cast(), 0o755) } != 0 {
        return false;
    }
    if !stat_changed(&a_dir, &a_st) {
        return false;
    }

    // b/1 file: does creating a file update the parent's mtime?
    // (matches Touch = creat 0444)
    let b1 = b_dir.join("1");
    // SAFETY: as above.
    let fd = unsafe {
        libc::open(
            nul(&b1).as_ptr().cast(),
            libc::O_CREAT | libc::O_WRONLY | libc::O_CLOEXEC,
            0o444,
        )
    };
    if fd < 0 {
        return false;
    }
    // SAFETY: standard close semantics.
    unsafe { libc::close(fd) };
    stat_changed(&b_dir, &b_st)
}

/// Path bytes + trailing NUL (a helper so every lstat/mkdir call is
/// NUL-terminated and cannot over-read).
fn nul(path: &Path) -> Vec<u8> {
    let mut b = path.as_os_str().as_bytes().to_vec();
    b.push(0);
    b
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
            // SAFETY: path is NUL-terminated via nul().
            let r = if is_dir {
                unsafe { libc::rmdir(nul(p).as_ptr().cast()) }
            } else {
                unsafe { libc::unlink(nul(p).as_ptr().cast()) }
            };
            let _ = r; // cleanup failure is not fatal
        }
    }
}

/// mkdtemp wrapper; libc fills the trailing XXXXXX.
fn mkdtemp(template: PathBuf) -> Option<PathBuf> {
    let mut bytes = template.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: bytes is a NUL-terminated mutable buffer; last 6 bytes are X.
    let p = unsafe { libc::mkdtemp(bytes.as_mut_ptr().cast()) };
    if p.is_null() {
        return None;
    }
    // SAFETY: on success p points to the filled NUL-terminated path.
    let cstr = unsafe { std::ffi::CStr::from_ptr(p) };
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes())))
}

/// lstat → (mtime_sec, mtime_nsec).
fn lstat(path: &Path) -> Option<(i64, i64)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: path is NUL-terminated via nul(); st is a valid buffer.
    if unsafe { libc::lstat(nul(path).as_ptr().cast(), &mut st) } != 0 {
        return None;
    }
    Some((st.st_mtime, st.st_mtime_nsec))
}

/// Whether the mtime changed (StatChanged: negation of StatEq).
fn stat_changed(path: &Path, prev: &(i64, i64)) -> bool {
    lstat(path).map(|cur| &cur != prev).unwrap_or(false)
}

/// Remove probe dirs older than 10s (mirrors RemoveStaleDirs): readdir
/// gitdir, find `.gitstatus.`-prefixed dirs with mtime older than now-10s,
/// and delete them in reverse order (a/1, a, b/1, b, root).
fn remove_stale_dirs(gitdir: &Path) {
    // SAFETY: path is NUL-terminated via nul().
    let dir_fd = unsafe {
        libc::open(
            nul(gitdir).as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if dir_fd < 0 {
        return;
    }
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut names: Vec<Vec<u8>> = Vec::new();
    let dup_fd = unsafe { libc::dup(dir_fd) };
    if dup_fd >= 0 {
        let dirp = unsafe { libc::fdopendir(dup_fd) };
        if !dirp.is_null() {
            loop {
                let ent = unsafe { libc::readdir(dirp) };
                if ent.is_null() {
                    break;
                }
                let name = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
                if name.starts_with(b".gitstatus.") {
                    names.push(name.to_vec());
                }
            }
            unsafe { libc::closedir(dirp) };
        } else {
            unsafe { libc::close(dup_fd) };
        }
    }
    for name in names {
        let mut path = gitdir.to_path_buf();
        path.push(std::ffi::OsStr::from_bytes(&name));
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: standard fstatat semantics.
        let ok = unsafe {
            libc::fstatat(
                dir_fd,
                std::ffi::CString::new(name.clone())
                    .unwrap()
                    .as_bytes_with_nul()
                    .as_ptr()
                    .cast(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if !ok || st.st_mtime + 10 > now {
            continue;
        }
        // Reverse-order delete: a/1, a, b/1, b, root (NUL via nul()).
        let mut sub = path.clone();
        sub.push("a/1");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("a");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("b/1");
        unsafe { libc::unlink(nul(&sub).as_ptr().cast()) };
        let mut sub = path.clone();
        sub.push("b");
        unsafe { libc::rmdir(nul(&sub).as_ptr().cast()) };
        unsafe { libc::rmdir(nul(&path).as_ptr().cast()) };
    }
    // SAFETY: standard close semantics.
    unsafe { libc::close(dir_fd) };
}
