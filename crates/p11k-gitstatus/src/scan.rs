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
use crate::sys::{self, DirEntry as Dirent};
use std::os::fd::RawFd;

pub struct ScanOpts {
    /// Collect untracked candidates (`-d` cap > 0).
    pub include_untracked: bool,
    /// Whether the untracked cache probe succeeded.
    pub untracked_cache_enabled: bool,
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

    for d in dirs.iter_mut() {
        // StatFiles: compare every index file (regardless of the cache).
        // Entry paths are root-relative and NUL-terminated: one fstatat
        // from root_fd, no directory open needed.
        let file_idxs = d.files.clone();
        for &ei in &file_idxs {
            let entry = &entries[ei];
            let rel = &entry.path[..entry.path.len() - 1];
            match sys::fstatat(root_fd, &entry.path) {
                // Deleted (ENOENT) or unreadable → candidate.
                Err(_) => candidates.push(rel.to_vec()),
                Ok(st) if is_modified(entry, &st, caps) => candidates.push(rel.to_vec()),
                Ok(_) => {}
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
            let cur = stat_dir(root_fd, d);
            match cur {
                Some(cur) => {
                    if d.st == Some(cur) {
                        for p in d.unmatched.clone() {
                            candidates.push(p);
                        }
                        continue;
                    }
                    d.st = Some(cur);
                }
                None => {
                    // Unstatable dir: clear untracked cache, no candidates.
                    d.st = None;
                    d.unmatched.clear();
                    continue;
                }
            }
        }

        // readdir + sort; on failure clear cache, no candidates.
        let dir_fd = open_dir(root_fd, d);
        let Some(dir_fd) = dir_fd else {
            d.st = None;
            d.unmatched.clear();
            continue;
        };
        let dirents = list_dir(dir_fd, caps.case_sensitive);
        // The descriptor is closed on every path; a close failure leaves nothing to retry, and
        // `dir_fd` is not used afterwards.
        if let Err(e) = sys::close(dir_fd) {
            log::debug!("close of a listed directory failed: {e}");
        }
        let Some(dirents) = dirents else {
            d.st = None;
            d.unmatched.clear();
            continue;
        };
        d.unmatched.clear();
        let dir_path_len = d.path.len() - 1;

        // Merge join: dirents (sorted) vs files (entries sorted) vs
        // subdirs (tree order).
        let files = d.files.clone();
        let subdirs = d.subdirs.clone();
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
                    let rel = &entry.path[..entry.path.len() - 1];
                    let modified = match sys::fstatat(root_fd, &entry.path) {
                        Err(_) => true,
                        Ok(st) => is_modified(entry, &st, caps),
                    };
                    if modified {
                        candidates.push(rel.to_vec());
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
                let mut p = d.path[..d.path.len() - 1].to_vec();
                p.extend_from_slice(name);
                if de.is_dir {
                    p.push(b'/');
                }
                d.unmatched.push(p.clone());
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
    let st = if dir.depth == 0 {
        sys::fstat(root_fd)
    } else {
        sys::fstatat(root_fd, &dir.path)
    };
    st.ok().map(|st| (st.st_mtime, st.st_mtime_nsec))
}

/// Open one directory for listing (untracked-cache miss): the root dir via
/// dup(root_fd), any other dir via openat(root_fd, path, O_DIRECTORY).
fn open_dir(root_fd: RawFd, dir: &IndexDir) -> Option<RawFd> {
    if dir.depth == 0 {
        return sys::dup(root_fd).ok();
    }
    sys::open_dir_at(root_fd, &dir.path).ok()
}

/// readdir + sort (mirrors dir.cc ListDir): dup(fd) + fdopendir + readdir
/// loop, skipping "." and "..", names kept NUL-terminated. Sorting is
/// byte-wise when case-sensitive, ASCII-folded otherwise (C-locale
/// strcasecmp semantics; independent of the process locale).
fn list_dir(fd: RawFd, case_sensitive: bool) -> Option<Vec<Dirent>> {
    let mut entries = sys::read_dir(fd).ok()?;
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
