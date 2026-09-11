//! 性能内核测试：建树、单条目脏检测、扫描端到端。
//!
//! 对拍基准：与官方 gitstatusd 在相同 fixture 上的候选集合一致
//! （tests/compat.rs 的差分测试将来覆盖；这里先锚定算法语义）。

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

/// 构造 index 条目：path 加 NUL，stat 字段取自真实文件。
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
    // SAFETY: 路径 NUL 结尾，st 合法缓冲。
    let r = unsafe { libc::lstat(bytes.as_ptr().cast(), &mut st) };
    assert_eq!(r, 0, "lstat {path:?}");
    st
}

fn open_root(path: &Path) -> RawFd {
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(0);
    // SAFETY: 路径 NUL 结尾。
    let fd = unsafe { libc::open(bytes.as_ptr().cast(), libc::O_RDONLY | libc::O_DIRECTORY) };
    assert!(fd >= 0);
    fd
}

#[test]
fn from_entries_builds_tree() {
    // 用假 stat（建树不读 stat 字段）
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

/// fixture：文件写 1 字节，index 记录 fsize=0。
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

/// ZERO_NSEC 特例：index 记录 nsec==0 时不比 nsec（对齐 GITSTATUS_ZERO_NSEC）。
#[test]
fn is_modified_zero_nsec_ignores_nsec_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("f");
    File::create(&f).unwrap();
    let st = lstat(&f);
    let mut e = entry("f", &st);
    e.mtime_nsec = 0;
    assert!(!is_modified(&e, &st, &caps()));
    // 秒不同仍脏
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

/// mode 规范化：仅可执行位参与比较（磁盘 0644、index 记录 0755 → 脏）。
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

/// 扫描端到端：a 修改、b 删除、c 干净、d untracked → 候选 {a,b,d}。
#[test]
fn scan_detects_modified_deleted_untracked() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // a：磁盘存在且写 1 字节，但 index 记录 fsize=0（视为修改）
    std::fs::write(root.join("a"), b"x").unwrap();
    let st_a = lstat(&root.join("a"));
    let mut e_a = entry("a", &st_a);
    e_a.fsize = 0;
    // b：磁盘不存在（删除）——用 a 的 stat 伪造（比较只发生在磁盘存在时）
    let mut e_b = entry("b", &st_a);
    e_b.path = b"b\0".to_vec();
    File::create(root.join("c")).unwrap();
    let st_c = lstat(&root.join("c"));
    let e_c = entry("c", &st_c);
    // d：untracked（无 index 条目）
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

/// 探针：真实文件系统上应通过（ext4/btrfs/tmpfs 都满足 mtime 行为）。
/// 后台线程 sleep 1s，测试等待结论。
#[test]
fn untracked_cache_probe_passes_on_local_fs() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = p11k_gitstatus::untracked_cache::UntrackedCache::start_probe(tmp.path());
    // 结论未出时乐观 true
    assert!(cache.enabled());
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < 5 && cache.enabled() {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(cache.enabled());
}

/// 多目录 + 多分片回归：600 文件分 10 个目录，1 线程触发 16 片，
/// 片边界会切断目录树——每个目录的脏文件都必须被检出。
/// （回归背景：分片内起点目录的父链断裂导致整片漏扫。）
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
                // 最后一个目录里的修改（保证落在靠后的分片）
                e.fsize = 0;
            }
            entries.push(e);
        }
    }
    // untracked 文件放在中间目录
    std::fs::write(root.join("dir05/u"), b"x").unwrap();

    // git index 保证字节序（"file10" < "file2"），测试数据需同样排序
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let mut index = Index::from_entries(entries);
    index.init_splits(1); // 16 片
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

/// gitlink（submodule, mode 160000）在磁盘上是目录：stat 字段/mode 与 gitlink
/// 条目不可比。目录仍在 → 不算 dirty（对齐原版 GIT_SUBMODULE_IGNORE_DIRTY）。
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
