//! Worktree traversal (mirrors `index.cc` ScanDirs and `dir.cc` ListDir).
//!
//! - Walk from a parent directory fd: `openat(fd, name)` + `fstatat(fd,
//!   name, AT_SYMLINK_NOFOLLOW)`, avoiding absolute-path lookups.
//! - Directory stack: `fds[d-1]` is the fd of depth d; a pre-order visit
//!   keeps the parent fd on top, so `truncate(depth)` + push needs the same
//!   number of openat calls as the original's rotate.
//! - StatFiles: index files are fstatat'ed + is_modified one by one
//!   (always done, regardless of the untracked cache).
//! - Merge join of sorted readdir entries against index files and
//!   subdirectories yields modified / deleted / new (untracked) candidates.
//! - Untracked-cache pruning: when a dir's mtime is unchanged, skip readdir
//!   and reuse the stored unmatched entries.
//! - d_type is trusted as-is (DT_DIR); DT_UNKNOWN is treated as a regular
//!   file (no fallback stat), like the original.

use crate::index::{IndexDir, IndexEntry, RepoCaps, is_modified};
use std::os::fd::RawFd;

/// RAII directory-fd stack: fds opened during a scan are closed on
/// truncate, clear, and drop. Without this, a nixpkgs-scale scan leaks
/// thousands of directory fds per pass; once the process fd limit is hit,
/// openat fails and untracked scanning degrades to 0.
struct DirStack {
    fds: Vec<RawFd>,
}

impl DirStack {
    fn new() -> Self {
        Self { fds: Vec::new() }
    }
    fn push(&mut self, fd: RawFd) {
        self.fds.push(fd);
    }
    fn get(&self, i: usize) -> Option<&RawFd> {
        self.fds.get(i)
    }
    fn last(&self) -> Option<&RawFd> {
        self.fds.last()
    }
    fn truncate(&mut self, n: usize) {
        for fd in self.fds.drain(n..) {
            // SAFETY: fd was pushed here, from openat/dup.
            unsafe { libc::close(fd) };
        }
    }
    fn close_all(&mut self) {
        for fd in self.fds.drain(..) {
            // SAFETY: as above.
            unsafe { libc::close(fd) };
        }
    }
}

impl Drop for DirStack {
    fn drop(&mut self) {
        self.close_all();
    }
}

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
pub fn scan_dirs(
    dirs: &mut [IndexDir],
    entries: &[IndexEntry],
    root_fd: RawFd,
    caps: &RepoCaps,
    opts: &ScanOpts,
) -> Vec<Vec<u8>> {
    let mut candidates: Vec<Vec<u8>> = Vec::new();
    // fds[d-1] = fd of depth d.
    let mut fds = DirStack::new();

    for idx in 0..dirs.len() {
        // Open the current dir (parent fd from the stack, truncated then push).
        let fd = match open_dir(&mut fds, root_fd, dirs, idx) {
            Some(fd) => fd,
            None => {
                // Unopenable dir: clear untracked cache, no candidates.
                dirs[idx].st = None;
                dirs[idx].unmatched.clear();
                continue;
            }
        };
        let dir_path_len = dirs[idx].path.len() - 1;

        // StatFiles: compare every index file (regardless of the cache).
        let file_idxs = dirs[idx].files.clone();
        for &ei in &file_idxs {
            let entry = &entries[ei];
            let basename = &entry.path[dir_path_len..entry.path.len() - 1];
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: basename is derived from an entry path and NUL-terminated.
            let r = unsafe {
                libc::fstatat(
                    fd,
                    basename.as_ptr().cast(),
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
        if opts.untracked_cache_enabled {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            // SAFETY: fd is an open directory.
            if unsafe { libc::fstat(fd, &mut st) } == 0 {
                let cur = (st.st_mtime, st.st_mtime_nsec);
                if dirs[idx].st == Some(cur) {
                    for p in dirs[idx].unmatched.clone() {
                        candidates.push(p);
                    }
                    continue;
                }
                dirs[idx].st = Some(cur);
            } else {
                dirs[idx].st = None;
                dirs[idx].unmatched.clear();
                continue;
            }
        }

        // readdir + sort; on failure clear cache, no candidates.
        let Some(dirents) = list_dir(fd, caps.case_sensitive) else {
            dirs[idx].st = None;
            dirs[idx].unmatched.clear();
            continue;
        };
        dirs[idx].unmatched.clear();

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
                            fd,
                            de.name.as_ptr().cast(),
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

/// Open dirs[idx]'s directory fd, maintaining the fds stack
/// (fds[d-1] = fd of depth d).
///
/// Two paths:
/// - Parent fd already on the stack (pre-order accumulation) → truncate the
///   stack to depth, openat, push.
/// - Parent fd not on the stack (shard start: ancestors fell in a
///   neighbouring shard) → clear the stack, dup the root, and rebuild the
///   full ancestor chain along `dirs[idx].path` (like the original's
///   OpenTail at each shard start).
fn open_dir(fds: &mut DirStack, root_fd: RawFd, dirs: &[IndexDir], idx: usize) -> Option<RawFd> {
    let depth = dirs[idx].depth;
    if depth != 0 && fds.get(depth - 1).is_none() {
        // Shard start: rebuild the ancestor chain.
        fds.close_all();
        // SAFETY: standard dup semantics.
        let mut fd = unsafe { libc::dup(root_fd) };
        if fd < 0 {
            return None;
        }
        fds.push(fd);
        let path = &dirs[idx].path[..dirs[idx].path.len() - 1]; // e.g. "a/b/"
        for seg in path.split(|&b| b == b'/') {
            if seg.is_empty() {
                continue;
            }
            let mut name = seg.to_vec();
            name.push(0); // NUL
            let parent = *fds.last().expect("chain non-empty");
            // SAFETY: name is NUL-terminated.
            fd = unsafe {
                libc::openat(
                    parent,
                    name.as_ptr().cast(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return None;
            }
            fds.push(fd);
        }
        return fds.last().copied();
    }
    let parent_fd = if depth == 0 {
        root_fd
    } else {
        *fds.get(depth - 1)?
    };
    fds.truncate(depth);
    let fd = if depth == 0 {
        // SAFETY: standard dup semantics.
        unsafe { libc::dup(root_fd) }
    } else {
        // SAFETY: basename is NUL-terminated (see module docs).
        unsafe {
            libc::openat(
                parent_fd,
                dirs[idx].basename.as_ptr().cast(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        }
    };
    if fd < 0 {
        return None;
    }
    fds.push(fd);
    Some(fd)
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
