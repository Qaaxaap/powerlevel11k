//! Worktree traversal (mirrors `index.cc` ScanDirs and `dir.cc` ListDir).
//!
//! - All stat calls go through a single root directory fd:
//!   `fstatat(root_fd, rel_path, AT_SYMLINK_NOFOLLOW)` with the full
//!   repository-relative path (index paths are NUL-terminated, so no
//!   allocation). No per-directory open is needed on the hot path — one
//!   syscall per file or dir stat, like the original's path-based stat.
//! - StatFiles: index files are fstatat'ed + is_modified one by one
//!   (always done, regardless of the untracked cache).
//! - Untracked-cache pruning: when a dir's mtime is unchanged (checked with
//!   one fstatat on the dir), skip readdir and reuse the stored unmatched
//!   entries.
//! - readdir happens only on an untracked-cache miss: a single openat from
//!   root_fd, fdopendir, readdir, close.
//! - Merge join of sorted readdir entries against index files and
//!   subdirectories yields modified / deleted / new (untracked) candidates.
//! - d_type is trusted as-is (DT_DIR); DT_UNKNOWN is treated as a regular
//!   file (no fallback stat), like the original.

use crate::index::{IndexDir, IndexEntry, RepoCaps, is_modified};
use std::os::fd::RawFd;

pub struct ScanOpts {
    /// Collect untracked candidates (`-d` cap > 0).
    pub include_untracked: bool,
    /// Whether the untracked cache probe succeeded.
    pub untracked_cache_enabled: bool,
}

/// One readdir entry. Name carries a trailing NUL (passed straight to
/// fstatat, zero allocation).
struct Dirent {
    name: Vec<u8>,
    is_dir: bool,
}

/// Scan dirs[from..to] and return candidate paths (relative to the repo
/// root, no NUL).
///
/// Hot path (dirs unchanged since the last scan): one fstatat per tracked
/// file (StatFiles) plus one fstatat per directory (untracked-cache mtime
/// check), no directory opens, no readdir. Directory fds are opened only
/// when the untracked cache misses and the dir must be listed.
pub fn scan_dirs(
    dirs: &mut [IndexDir],
    entries: &[IndexEntry],
    root_fd: RawFd,
    caps: &RepoCaps,
    opts: &ScanOpts,
) -> Vec<Vec<u8>> {
    let mut candidates: Vec<Vec<u8>> = Vec::new();

    for idx in 0..dirs.len() {
        // StatFiles: compare every index file (regardless of the cache).
        // Entry paths are root-relative and NUL-terminated: one fstatat
        // from root_fd, no directory open needed.
        let file_idxs = dirs[idx].files.clone();
        for &ei in &file_idxs {
            let entry = &entries[ei];
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: entry.path is NUL-terminated.
            let r = unsafe {
                libc::fstatat(
                    root_fd,
                    entry.path.as_ptr().cast(),
                    &mut st,
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if r != 0 {
                // Deleted (ENOENT) or unreadable → candidate.
                candidates.push(entry.path[..entry.path.len() - 1].to_vec());
            } else if is_modified(entry, &st, caps) {
                candidates.push(entry.path[..entry.path.len() - 1].to_vec());
            }
        }

        if !opts.include_untracked {
            continue;
        }

        // Untracked cache: unchanged mtime → reuse unmatched, skip readdir.
        // Dir mtime via one fstatat from root_fd (the root dir itself via
        // fstat on root_fd). The trailing '/' on dir paths is fine for
        // fstatat.
        if opts.untracked_cache_enabled {
            let cur = stat_dir(root_fd, &dirs[idx]);
            match cur {
                Some(cur) => {
                    if dirs[idx].st == Some(cur) {
                        for p in dirs[idx].unmatched.clone() {
                            candidates.push(p);
                        }
                        continue;
                    }
                    dirs[idx].st = Some(cur);
                }
                None => {
                    // Unstatable dir: clear untracked cache, no candidates.
                    dirs[idx].st = None;
                    dirs[idx].unmatched.clear();
                    continue;
                }
            }
        }

        // readdir + sort; on failure clear cache, no candidates.
        let dir_fd = open_dir(root_fd, &dirs[idx]);
        let Some(dir_fd) = dir_fd else {
            dirs[idx].st = None;
            dirs[idx].unmatched.clear();
            continue;
        };
        let dirents = list_dir(dir_fd, caps.case_sensitive);
        // SAFETY: dir_fd was opened above; close on every path.
        unsafe { libc::close(dir_fd) };
        let Some(dirents) = dirents else {
            dirs[idx].st = None;
            dirs[idx].unmatched.clear();
            continue;
        };
        dirs[idx].unmatched.clear();
        let dir_path_len = dirs[idx].path.len() - 1;

        // Merge join: dirents (sorted) vs files (entries sorted) vs
        // subdirs (tree order).
        let files = dirs[idx].files.clone();
        let subdirs = dirs[idx].subdirs.clone();
        let mut fi = 0usize;
        let mut si = 0usize;
        for de in &dirents {
            let name = &de.name[..de.name.len() - 1]; // strip NUL
            // Merge against index files.
            let mut matched = false;
            while fi < files.len() {
                let entry = &entries[files[fi]];
                let base = &entry.path[dir_path_len..entry.path.len() - 1];
                let cmp = cmp_name(base, name, caps.case_sensitive);
                if cmp == std::cmp::Ordering::Less {
                    // In index, missing on disk → deleted.
                    candidates.push(entry.path[..entry.path.len() - 1].to_vec());
                    fi += 1;
                } else if cmp == std::cmp::Ordering::Equal {
                    let mut st: libc::stat = unsafe { std::mem::zeroed() };
                    // SAFETY: name comes from readdir and is NUL-terminated.
                    let r = unsafe {
                        libc::fstatat(
                            root_fd,
                            entry.path.as_ptr().cast(),
                            &mut st,
                            libc::AT_SYMLINK_NOFOLLOW,
                        )
                    };
                    if r != 0 || is_modified(entry, &st, caps) {
                        candidates.push(entry.path[..entry.path.len() - 1].to_vec());
                    }
                    matched = true;
                    fi += 1;
                    break;
                } else {
                    break;
                }
            }
            if matched {
                continue;
            }
            // Merge against subdirectories.
            while si < subdirs.len() {
                let cmp = cmp_name(&subdirs[si], name, caps.case_sensitive);
                if cmp == std::cmp::Ordering::Greater {
                    break;
                }
                if cmp == std::cmp::Ordering::Equal {
                    matched = true;
                    si += 1;
                    break;
                }
                si += 1;
            }
            if !matched {
                // Untracked: dir prefix + name; dirs get a trailing '/'.
                let mut p = dirs[idx].path[..dirs[idx].path.len() - 1].to_vec();
                p.extend_from_slice(name);
                if de.is_dir {
                    p.push(b'/');
                }
                dirs[idx].unmatched.push(p.clone());
                candidates.push(p);
            }
        }
        // Remaining index files → deleted.
        while fi < files.len() {
            let entry = &entries[files[fi]];
            candidates.push(entry.path[..entry.path.len() - 1].to_vec());
            fi += 1;
        }
    }
    candidates
}

/// Stat a directory (for the untracked-cache mtime check): the root dir via
/// fstat on root_fd, any other dir via fstatat(root_fd, path) — dir paths
/// carry a trailing '/' and NUL, both accepted by fstatat. Returns
/// (mtime_sec, mtime_nsec); None when the dir cannot be stat'ed.
fn stat_dir(root_fd: RawFd, dir: &IndexDir) -> Option<(i64, i64)> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let ok = if dir.depth == 0 {
        // SAFETY: root_fd is an open directory.
        unsafe { libc::fstat(root_fd, &mut st) }
    } else {
        // SAFETY: dir.path is NUL-terminated.
        unsafe {
            libc::fstatat(
                root_fd,
                dir.path.as_ptr().cast(),
                &mut st,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        }
    };
    (ok == 0).then_some((st.st_mtime, st.st_mtime_nsec))
}

/// Open one directory for listing (untracked-cache miss): the root dir via
/// dup(root_fd), any other dir via openat(root_fd, path, O_DIRECTORY).
fn open_dir(root_fd: RawFd, dir: &IndexDir) -> Option<RawFd> {
    if dir.depth == 0 {
        // SAFETY: standard dup semantics.
        let fd = unsafe { libc::dup(root_fd) };
        return (fd >= 0).then_some(fd);
    }
    // SAFETY: dir.path is NUL-terminated.
    let fd = unsafe {
        libc::openat(
            root_fd,
            dir.path.as_ptr().cast(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    (fd >= 0).then_some(fd)
}

/// readdir + sort (mirrors dir.cc ListDir): dup(fd) + fdopendir + readdir
/// loop, skipping "." and "..", names kept NUL-terminated. Sorting is
/// byte-wise when case-sensitive, ASCII-folded otherwise (C-locale
/// strcasecmp semantics; independent of the process locale).
fn list_dir(fd: RawFd, case_sensitive: bool) -> Option<Vec<Dirent>> {
    // SAFETY: the dup'd fd is taken over by fdopendir and released on
    // closedir.
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return None;
    }
    // SAFETY: fdopendir takes ownership of dup_fd.
    let dirp = unsafe { libc::fdopendir(dup_fd) };
    if dirp.is_null() {
        // SAFETY: on fdopendir failure the fd was not taken over; close it.
        unsafe { libc::close(dup_fd) };
        return None;
    }
    let mut entries: Vec<Dirent> = Vec::with_capacity(128);
    loop {
        // SAFETY: readdir returns a pointer into dirp's internal buffer.
        let ent = unsafe { libc::readdir(dirp) };
        if ent.is_null() {
            break;
        }
        // SAFETY: ent is valid (readdir returned non-null).
        let name_bytes = unsafe { std::ffi::CStr::from_ptr((*ent).d_name.as_ptr()) }.to_bytes();
        if name_bytes == b"." || name_bytes == b".." {
            continue;
        }
        let mut name = name_bytes.to_vec();
        name.push(0); // NUL (passed to fstatat as-is)
        // SAFETY: ent is valid.
        let is_dir = unsafe { (*ent).d_type == libc::DT_DIR };
        entries.push(Dirent { name, is_dir });
    }
    // SAFETY: closedir releases the fd taken by fdopendir.
    unsafe { libc::closedir(dirp) };
    if case_sensitive {
        entries.sort_by(|a, b| a.name[..a.name.len() - 1].cmp(&b.name[..b.name.len() - 1]));
    } else {
        entries.sort_by(|a, b| {
            let ka = a.name[..a.name.len() - 1]
                .iter()
                .map(|&c| c.to_ascii_lowercase());
            let kb = b.name[..b.name.len() - 1]
                .iter()
                .map(|&c| c.to_ascii_lowercase());
            ka.cmp(kb)
        });
    }
    Some(entries)
}

/// Name comparison (StrCmp semantics: byte-wise, length decides the tie).
fn cmp_name(a: &[u8], b: &[u8], case_sensitive: bool) -> std::cmp::Ordering {
    let n = a.len().min(b.len());
    for i in 0..n {
        let (x, y) = if case_sensitive {
            (a[i], b[i])
        } else {
            (a[i].to_ascii_lowercase(), b[i].to_ascii_lowercase())
        };
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    a.len().cmp(&b.len())
}
