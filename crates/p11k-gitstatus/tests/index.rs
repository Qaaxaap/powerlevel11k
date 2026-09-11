//! Performance-core tests: tree building, single-entry dirty detection, scan
//! end to end.
//!
//! Reference baseline: the candidate set matches official gitstatusd on the
//! same fixture
//! (differential tests in tests/compat.rs will cover it later; here we anchor
//! the algorithm semantics first).

use p11k_gitstatus::index::{Index, IndexEntry, RepoCaps, is_modified};
use p11k_gitstatus::scan::ScanOpts;
use std::fs::File;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

fn caps() -> RepoCaps {
    RepoCaps {
        trust_filemode: true,
        has_symlinks: true,
        case_sensitive: true,
    }
}

fn opts() -> ScanOpts {
    ScanOpts {
        include_untracked: true,
        untracked_cache_enabled: false,
    }
}

/// Build an index entry: path gets a NUL, stat fields come from a real file.
fn entry(path: &str, st: &libc::stat) -> IndexEntry {
    let mut p = path.as_bytes().to_vec();
    p.push(0);
    IndexEntry {
        path: p,
        ino: st.st_ino as u32,
        fsize: st.st_size as u32,
        mtime_sec: st.st_mtime as i32,
        mtime_nsec: st.st_mtime_nsec as u32,
        mode: st.st_mode,
        stage: 0,
        flags_extended: 0,
        assume_valid: false,
    }
}

fn lstat(path: &Path) -> libc::stat {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: the path is NUL-terminated, st is a valid buffer.
    let r = unsafe { libc::lstat(bytes.as_ptr().cast(), &mut st) };
    assert_eq!(r, 0, "lstat {path:?}");
    st
}

fn open_root(path: &Path) -> RawFd {
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: the path is NUL-terminated.
    let fd = unsafe { libc::open(bytes.as_ptr().cast(), libc::O_RDONLY | libc::O_DIRECTORY) };
    assert!(fd >= 0);
    fd
}

#[test]
fn from_entries_builds_tree() {
    // Use a fake stat (tree building does not read stat fields)
    let fake = |path: &str| {
        let mut p = path.as_bytes().to_vec();
        p.push(0);
        IndexEntry {
            path: p,
            ino: 0,
            fsize: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            mode: 0o100644,
            stage: 0,
            flags_extended: 0,
            assume_valid: false,
        }
    };
    let entries = vec![fake("a"), fake("dir/b"), fake("dir/sub/c")];
    let index = Index::from_entries(entries);
    assert_eq!(index.dirs.len(), 3);
    assert_eq!(&index.dirs[0].path[..index.dirs[0].path.len() - 1], b"");
    assert_eq!(&index.dirs[1].path[..index.dirs[1].path.len() - 1], b"dir/");
    assert_eq!(
        &index.dirs[2].path[..index.dirs[2].path.len() - 1],
        b"dir/sub/"
    );
    assert_eq!(index.dirs[0].files, vec![0]);
    assert_eq!(index.dirs[1].files, vec![1]);
    assert_eq!(index.dirs[2].files, vec![2]);
    assert_eq!(index.dirs[0].subdirs, vec![b"dir".to_vec()]);
    assert_eq!(index.dirs[1].subdirs, vec![b"sub".to_vec()]);
    assert_eq!(index.dirs[0].depth, 0);
    assert_eq!(index.dirs[1].depth, 1);
    assert_eq!(index.dirs[2].depth, 2);
}

#[test]
fn is_modified_clean_is_false() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let e = entry("f", &st);
    assert!(!is_modified(&e, &st, &caps()));
}

/// fixture: the file holds 1 byte, the index records fsize=0.
#[test]
fn is_modified_detects_size_change() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    std::fs::write(&f, b"x").unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.fsize = 0;
    assert!(is_modified(&e, &st, &caps()));
}

#[test]
fn is_modified_detects_ino_change() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.ino += 1;
    assert!(is_modified(&e, &st, &caps()));
}

/// ZERO_NSEC special case: an index nsec of 0 skips nsec comparison (mirrors GITSTATUS_ZERO_NSEC).
#[test]
fn is_modified_zero_nsec_ignores_nsec_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.mtime_nsec = 0;
    assert!(!is_modified(&e, &st, &caps()));
    // A different second is still dirty
    let mut e2 = entry("f", &st);
    e2.mtime_nsec = 0;
    e2.mtime_sec += 1;
    assert!(is_modified(&e2, &st, &caps()));
}

#[test]
fn is_modified_conflict_stage_is_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.stage = 1;
    assert!(is_modified(&e, &st, &caps()));
}

/// mode normalization: only the executable bit takes part in the comparison (disk 0644, index 0755 → dirty).
#[test]
fn is_modified_exec_bit_only_differs() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.mode = 0o100755;
    assert!(is_modified(&e, &st, &caps()));
    let e2 = entry("f", &st);
    assert!(!is_modified(&e2, &st, &caps()));
}

/// Scan end to end: a modified, b deleted, c clean, d untracked → candidates {a,b,d}.
#[test]
fn scan_detects_modified_deleted_untracked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // a: exists on disk with 1 byte written, but the index records fsize=0 (counts as modified)
    std::fs::write(root.join("a"), b"x").unwrap();
    let st_a = lstat(&root.join("a"));
    let mut e_a = entry("a", &st_a);
    e_a.fsize = 0;
    // b: missing on disk (deleted) — reuse a's stat (comparison only happens when the file is on disk)
    let mut e_b = entry("b", &st_a);
    e_b.path = b"b\0".to_vec();
    File::create(root.join("c")).unwrap();
    let st_c = lstat(&root.join("c"));
    let e_c = entry("c", &st_c);
    // d: untracked (no index entry)
    File::create(root.join("d")).unwrap();

    let entries = vec![e_a, e_b, e_c];
    let mut index = Index::from_entries(entries);
    index.init_splits(1);
    let root_fd = open_root(root);
    let mut out = index.get_dirty_candidates(root_fd, &caps(), &opts());
    out.sort();
    assert_eq!(out, vec![b"a".to_vec(), b"b".to_vec(), b"d".to_vec()]);
}

#[test]
fn splits_are_monotonic() {
    let fake = |i: usize| {
        let path = format!("d{i}/f");
        let mut p = path.into_bytes();
        p.push(0);
        IndexEntry {
            path: p,
            ino: 0,
            fsize: 0,
            mtime_sec: 0,
            mtime_nsec: 0,
            mode: 0o100644,
            stage: 0,
            flags_extended: 0,
            assume_valid: false,
        }
    };
    let entries: Vec<IndexEntry> = (0..2000).map(fake).collect();
    let mut index = Index::from_entries(entries);
    index.init_splits(4);
    assert_eq!(index.splits.first(), Some(&0));
    assert_eq!(index.splits.last(), Some(&index.dirs.len()));
    for pair in index.splits.windows(2) {
        assert!(pair[0] < pair[1]);
    }
}

/// Probe: should pass on a real filesystem (ext4/btrfs/tmpfs all satisfy the mtime behavior).
/// The background thread sleeps 1s, the test waits for the verdict.
#[test]
fn untracked_cache_probe_passes_on_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = p11k_gitstatus::untracked_cache::UntrackedCache::start_probe(tmp.path());
    // Optimistically true until the verdict arrives
    assert!(cache.enabled());
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 5 && cache.enabled() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(cache.enabled());
}

/// Multi-dir + multi-shard regression: 600 files across 10 dirs, 1 thread triggers 16 shards,
/// shard boundaries cut through the directory tree — every dir's dirty files must still be found.
/// (Regression background: a broken parent chain at a shard's first dir made the whole shard miss.)
#[test]
fn scan_multi_dir_shards_find_all_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut entries = Vec::new();
    for d in 0..10 {
        let dir = root.join(format!("dir{d:02}"));
        std::fs::create_dir(&dir).unwrap();
        for i in 0..60 {
            let name = format!("file{i}");
            let f = dir.join(&name);
            std::fs::write(&f, b"clean").unwrap();
            let st = lstat(&f);
            let mut e = entry(&format!("dir{d:02}/{name}"), &st);
            if d == 9 && i == 0 {
                // Modification in the last directory (to land in a later shard)
                e.fsize = 0;
            }
            entries.push(e);
        }
    }
    // untracked file in a middle directory
    std::fs::write(root.join("dir05/u"), b"x").unwrap();

    // git index guarantees byte order ("file10" < "file2"), so the test data must be sorted the same way
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut index = Index::from_entries(entries);
    index.init_splits(1); // 16 shards
    assert!(
        index.splits.len() > 2,
        "expected multiple shards: {:?}",
        index.splits
    );
    let root_fd = open_root(root);
    let mut out = index.get_dirty_candidates(root_fd, &caps(), &opts());
    out.sort();
    assert_eq!(out, vec![b"dir05/u".to_vec(), b"dir09/file0".to_vec()]);
}

/// A gitlink (submodule, mode 160000) is a directory on disk: its stat fields/mode are not comparable
/// with the gitlink entry. A directory still present → not dirty (mirrors the original GIT_SUBMODULE_IGNORE_DIRTY).
#[test]
fn gitlink_dir_stat_is_not_modified() {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    st.st_mode = libc::S_IFDIR | 0o755;
    st.st_size = 64;
    st.st_mtime = 12345;
    st.st_mtime_nsec = 0;
    st.st_ino = 999;
    let mut e = entry("custom/plugins/theme", &st);
    e.mode = 0o160000; // gitlink
    e.fsize = 0;
    e.mtime_sec = 0;
    e.mtime_nsec = 0;
    assert!(
        !is_modified(&e, &st, &caps()),
        "a gitlink dir still present must not be dirty (otherwise every submodule in a clean repo reports unstaged)"
    );
}
