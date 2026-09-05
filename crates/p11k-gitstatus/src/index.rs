//! git index parsing and dirty-candidate scan (performance core, mirrors
//! index.cc).
//!
//! Strategy (same algorithm, not same code):
//!
//! 1. **Build the tree**: index entries (path-sorted by git) become a
//!    directory tree of [`IndexDir`] via a stack algorithm (InitDirs'
//!    CommonDir common-prefix method).
//! 2. **Shard**: `16 * num_threads` shards with a minimum shard weight of
//!    512, split by directory weight (InitSplits). Each shard is scanned by
//!    one scoped thread via [`Index::get_dirty_candidates`]; shards never
//!    overlap directories (each gets its own `&mut [IndexDir]`).
//! 3. **Dirty detection calls no git**: `fstatat(root_fd, rel_path,
//!    AT_SYMLINK_NOFOLLOW)` compares the on-disk stat against the index
//!    record ([`is_modified`]); no directory opens on the hot path (see
//!    scan.rs).
//! 4. **Candidates**: modified / deleted / new (untracked) / unreadable.
//!    Collected fully, sorted, deduped, then handed to the repo layer for
//!    the precise diff (the original also collects first, then runs
//!    git_diff_index_to_workdir).
//!
//! # Embedded NUL in paths
//!
//! [`IndexEntry::path`] and [`IndexDir::path`] keep a trailing NUL byte
//! (visible length = len - 1): `fstatat`/`openat` take the pointer directly,
//! zero allocation. This is the Rust equivalent of the original's Arena
//! layout (d_type at `[-1]`, NUL-terminated).

use crate::scan::{self, ScanOpts};
use std::os::fd::RawFd;

/// Repo capability bits (index.cc RepoCaps). precompose_unicode (macOS HFS+
/// normalization) is TODO(macOS), not implemented in phase 1.
#[derive(Clone, Copy)]
pub struct RepoCaps {
    /// core.filemode.
    pub trust_filemode: bool,
    /// core.symlinks.
    pub has_symlinks: bool,
    /// Case sensitivity (!core.ignorecase).
    pub case_sensitive: bool,
}

/// Index entry (copied from a git2 IndexEntry).
pub struct IndexEntry {
    /// Full relative path, NUL-terminated (see module docs).
    pub path: Vec<u8>,
    pub ino: u32,
    pub fsize: u32,
    pub mtime_sec: i32,
    pub mtime_nsec: u32,
    pub mode: u32,
    /// GIT_INDEX_ENTRY_STAGE (non-zero = conflict entry, always a candidate).
    pub stage: u16,
    /// GIT_INDEX_ENTRY_EXTENDED bits: SKIP_WORKTREE / INTENT_TO_ADD.
    pub flags_extended: u16,
    /// GIT_INDEX_ENTRY_VALID (assume-unchanged).
    pub assume_valid: bool,
}

/// Index directory-tree node. `dirs` is in pre-order (`dirs[0]` is the root).
pub struct IndexDir {
    /// Full relative path ending in '/', NUL-terminated (root is "").
    pub path: Vec<u8>,
    /// Directory name (no parent path, no '/'), NUL-terminated (passed to
    /// openat; empty Vec for the root).
    pub basename: Vec<u8>,
    /// Depth (root = 0).
    pub depth: usize,
    /// Indices into [`Index::entries`] for files in this dir.
    pub files: Vec<usize>,
    /// Subdirectory basenames (copies, not indices): during sharded scans a
    /// subdir may fall in a neighbouring shard, and the merge join only
    /// needs name comparison, so copies keep each shard slice self-contained.
    pub subdirs: Vec<Vec<u8>>,
    /// Untracked cache: the dir mtime from the last readdir.
    pub st: Option<(i64, i64)>,
    /// Untracked cache: untracked names found by the last readdir.
    pub unmatched: Vec<Vec<u8>>,
}

pub struct Index {
    pub entries: Vec<IndexEntry>,
    pub dirs: Vec<IndexDir>,
    /// Shard boundaries: each pair of adjacent indices is one shard
    /// (index.cc splits_).
    pub splits: Vec<usize>,
}

impl Index {
    /// Build the tree. Entries must be path-sorted (git guarantees this).
    /// Mirrors InitDirs' stack algorithm: find the common dir prefix with
    /// the stack top, pop the excess levels, create subdirs for the
    /// remaining path, and file the entry into the top dir.
    pub fn from_entries(entries: Vec<IndexEntry>) -> Index {
        let mut dirs = vec![IndexDir {
            path: vec![0], // root: "" + NUL
            basename: Vec::new(),
            depth: 0,
            files: Vec::new(),
            subdirs: Vec::new(),
            st: None,
            unmatched: Vec::new(),
        }];
        let mut stack: Vec<usize> = vec![0];
        for (i, entry) in entries.iter().enumerate() {
            let path = &entry.path[..entry.path.len() - 1]; // strip NUL
            // Common dir prefix with the stack top (CommonDir: length incl.
            // trailing '/', plus depth).
            let mut common_len = 0usize;
            let mut common_depth = 0usize;
            {
                let top = &dirs[stack[stack.len() - 1]];
                let top_path = &top.path[..top.path.len() - 1];
                let n = top_path.len().min(path.len());
                let mut j = 0;
                while j < n && top_path[j] == path[j] {
                    j += 1;
                    if path[j - 1] == b'/' {
                        common_len = j;
                        common_depth += 1;
                    }
                }
            }
            // Pop levels below the common depth.
            while stack.len() > common_depth + 1 {
                stack.pop();
            }
            // Create subdirs for the remaining path segments.
            let mut p = common_len;
            while let Some(rel) = path[p..].iter().position(|&b| b == b'/') {
                let slash = p + rel;
                let parent = stack[stack.len() - 1];
                let parent_len = dirs[parent].path.len() - 1;
                // basename NUL-terminated (zero-alloc openat); subdirs keep
                // NUL-free copies.
                let mut basename = path[parent_len..slash].to_vec();
                basename.push(0);
                let mut dir_path = path[..=slash].to_vec(); // incl. '/'
                dir_path.push(0); // NUL
                let idx = dirs.len();
                // Record the subdir name on the parent (copy, no NUL), then
                // push the new dir (basename moved in).
                dirs[parent]
                    .subdirs
                    .push(basename[..basename.len() - 1].to_vec());
                dirs.push(IndexDir {
                    path: dir_path,
                    basename,
                    depth: stack.len(),
                    files: Vec::new(),
                    subdirs: Vec::new(),
                    st: None,
                    unmatched: Vec::new(),
                });
                stack.push(idx);
                p = slash + 1;
            }
            // File the entry into the top dir.
            let dir_idx = stack[stack.len() - 1];
            dirs[dir_idx].files.push(i);
        }
        Index {
            entries,
            dirs,
            splits: Vec::new(),
        }
    }

    /// Split into `16 * num_threads` shards (InitSplits): minimum shard
    /// weight 512, accumulated by dir weight; splits ascending, no
    /// duplicates, first 0 last len.
    pub fn init_splits(&mut self, num_threads: usize) {
        const MIN_SHARD_WEIGHT: usize = 512;
        let num_shards = 16 * num_threads.max(1);
        let total: usize = self.dirs.iter().map(Index::weight).sum();
        let shard_weight = MIN_SHARD_WEIGHT.max(total / num_shards);
        self.splits.clear();
        self.splits.push(0);
        let mut w = 0usize;
        for (i, dir) in self.dirs.iter().enumerate() {
            w += Index::weight(dir);
            if w >= shard_weight {
                w = 0;
                self.splits.push(i + 1);
            }
        }
        if self.splits.last() != Some(&self.dirs.len()) {
            self.splits.push(self.dirs.len());
        }
    }

    /// Collect dirty candidates in parallel (GetDirtyCandidates): one scoped
    /// thread per shard, results merged, sorted (byte-wise when
    /// case-sensitive, ASCII-folded otherwise), deduped by byte equality.
    /// root_fd is the repo root directory fd.
    pub fn get_dirty_candidates(
        &mut self,
        root_fd: RawFd,
        caps: &RepoCaps,
        opts: &ScanOpts,
    ) -> Vec<Vec<u8>> {
        let entries = &self.entries;
        let mut results: Vec<Vec<Vec<u8>>> = Vec::new();
        // Shards don't overlap: carve &mut slices from the front of dirs.
        // All shards are spawned first, then joined (spawn-then-join in the
        // same loop would serialize the scan).
        std::thread::scope(|scope| {
            let mut rest: &mut [IndexDir] = &mut self.dirs;
            let mut handles = Vec::with_capacity(self.splits.len() - 1);
            for pair in self.splits.windows(2) {
                let (from, to) = (pair[0], pair[1]);
                let (shard, tail) = std::mem::take(&mut rest).split_at_mut(to - from);
                handles.push(
                    scope.spawn(move || scan::scan_dirs(shard, entries, root_fd, caps, opts)),
                );
                rest = tail;
            }
            for h in handles {
                results.push(h.join().expect("scan thread panicked"));
            }
        });
        let mut out: Vec<Vec<u8>> = results.into_iter().flatten().collect();
        // Sort: ASCII-fold when case-insensitive (StrSort's C-locale semantics).
        if caps.case_sensitive {
            out.sort();
        } else {
            out.sort_by(|a, b| {
                let ka = a.iter().map(|&c| c.to_ascii_lowercase());
                let kb = b.iter().map(|&c| c.to_ascii_lowercase());
                ka.cmp(kb)
            });
        }
        out.dedup();
        out
    }

    /// Refresh only the stat fields from new entries (the caller guarantees
    /// the path set is unchanged), reusing the tree structure and untracked
    /// state. Used when libgit2's racy write-back only mutates stat fields,
    /// avoiding a full rebuild of a large index.
    pub fn update_stats(&mut self, new_entries: &[IndexEntry]) {
        for (old, new) in self.entries.iter_mut().zip(new_entries) {
            old.ino = new.ino;
            old.fsize = new.fsize;
            old.mtime_sec = new.mtime_sec;
            old.mtime_nsec = new.mtime_nsec;
            old.mode = new.mode;
            old.stage = new.stage;
            old.flags_extended = new.flags_extended;
            old.assume_valid = new.assume_valid;
        }
    }

    /// Directory weight (index.cc Weight: 1 + subdirs + files).
    pub fn weight(dir: &IndexDir) -> usize {
        1 + dir.subdirs.len() + dir.files.len()
    }
}

/// Per-entry dirty check (index.cc IsModified).
///
/// mode is normalized first: regular files keep only the executable bit
/// (`0755/0644`); when trust_filemode or symlinks capability is missing,
/// mode comparison is skipped; non-regular files compare only the file
/// type. Comparison order: ino → stage → fsize → mtime → mode; any
/// difference makes it a candidate. mtime nsec special case: an index
/// nsec of 0 skips nsec comparison (GITSTATUS_ZERO_NSEC — git zeroes nsec
/// after racy detection).
pub fn is_modified(entry: &IndexEntry, st: &libc::stat, caps: &RepoCaps) -> bool {
    let mut mode = st.st_mode;
    if mode & libc::S_IFMT == libc::S_IFREG {
        // Missing symlinks capability with a symlink entry, or untrusted
        // filemode → skip mode comparison.
        if (!caps.has_symlinks && entry.mode & libc::S_IFMT == libc::S_IFLNK)
            || !caps.trust_filemode
        {
            mode = entry.mode;
        } else {
            mode = libc::S_IFREG | if mode & 0o100 != 0 { 0o755 } else { 0o644 };
        }
    } else {
        mode &= libc::S_IFMT;
    }
    let ino_ok = entry.ino == 0 || u64::from(entry.ino) == st.st_ino;
    let stage_ok = entry.stage == 0;
    let fsize_ok = i64::from(entry.fsize) == st.st_size;
    let mtime_ok = i64::from(entry.mtime_sec) == st.st_mtime
        && (i64::from(entry.mtime_nsec) == st.st_mtime_nsec || entry.mtime_nsec == 0);
    let mode_ok = entry.mode == mode;
    !(ino_ok && stage_ok && fsize_ok && mtime_ok && mode_ok)
}
