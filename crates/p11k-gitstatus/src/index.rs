//! git index 解析与脏候选扫描（性能核心，对齐 index.cc）。
//!
//! # 总体策略（照搬算法，不照搬代码）
//!
//! 1. **建树**：index 条目（已按路径排序）经栈算法建成目录树 [`IndexDir`]
//!    （对齐 InitDirs 的 CommonDir 公共前缀法）。
//! 2. **分片**：`16 * num_threads` 片、最小片权重 512，按目录权重切分
//!    （对齐 InitSplits）；每片由 [`Index::get_dirty_candidates`] 交给一个
//!    作用域线程并行扫描，分片保证目录不重叠（每片独立 `&mut [IndexDir]`）。
//! 3. **脏检测不调用 git**：`fstatat(dir_fd, basename, AT_SYMLINK_NOFOLLOW)`
//!    取磁盘 stat 与 index 记录比较（[`is_modified`]）。
//! 4. **候选收集**：候选 = modified / deleted / new（untracked）/ unreadable，
//!    全量收集、排序、去重后交给 repo 层做精确 diff（原版同样先收集再
//!    git_diff_index_to_workdir）。
//!
//! # 路径内嵌 NUL
//!
//! [`IndexEntry::path`] 与 [`IndexDir::path`] 的最后一个字节是 NUL（可见
//! 长度 = len - 1）：`fstatat`/`openat` 直传指针，零分配。这是原版 Arena
//! 布局（d_type 存 `[-1]`、NUL 结尾）的 Rust 等价。

use crate::scan::{self, ScanOpts};
use std::os::fd::RawFd;

/// 仓库能力位（对齐 index.cc RepoCaps）。
/// precompose_unicode（macOS HFS+ 归一化）TODO(macOS)，第一阶段不实现。
#[derive(Clone, Copy)]
pub struct RepoCaps {
    /// core.filemode。
    pub trust_filemode: bool,
    /// core.symlinks。
    pub has_symlinks: bool,
    /// 大小写敏感（!core.ignorecase）。
    pub case_sensitive: bool,
}

/// index 条目（从 git2 IndexEntry 拷贝）。
pub struct IndexEntry {
    /// 完整相对路径，含尾部 NUL（见模块文档）。
    pub path: Vec<u8>,
    pub ino: u32,
    pub fsize: u32,
    pub mtime_sec: i32,
    pub mtime_nsec: u32,
    pub mode: u32,
    /// GIT_INDEX_ENTRY_STAGE（非 0 = 冲突条目，恒为候选）。
    pub stage: u16,
    /// GIT_INDEX_ENTRY_EXTENDED 位：SKIP_WORKTREE / INTENT_TO_ADD。
    pub flags_extended: u16,
    /// GIT_INDEX_ENTRY_VALID（assume-unchanged）。
    pub assume_valid: bool,
}

/// index 目录树节点。`dirs` 为前序（`dirs[0]` 是根）。
pub struct IndexDir {
    /// 完整相对路径，以 '/' 结尾（根为 ""），含尾部 NUL。
    pub path: Vec<u8>,
    /// 目录名（不含父路径、不含 '/'）。
    pub basename: Vec<u8>,
    /// 深度（根为 0）。
    pub depth: usize,
    /// 本目录下条目在 [`Index::entries`] 中的下标。
    pub files: Vec<usize>,
    /// 子目录在 [`Index::dirs`] 中的下标。
    pub subdirs: Vec<usize>,
    /// untracked cache：上次 readdir 时的目录 mtime。
    pub st: Option<(i64, i64)>,
    /// untracked cache：上次 readdir 发现的 untracked 名。
    pub unmatched: Vec<Vec<u8>>,
}

pub struct Index {
    pub entries: Vec<IndexEntry>,
    pub dirs: Vec<IndexDir>,
    /// 分片边界：相邻两个下标成一片（对齐 index.cc splits_）。
    pub splits: Vec<usize>,
}

impl Index {
    /// 建树。entries 必须按路径排序（git index 保证）。
    /// 照搬 InitDirs 的栈算法：与栈顶求公共目录前缀 → 弹多余层 →
    /// 为剩余路径逐层建子目录 → 条目落入栈顶 files。
    pub fn from_entries(entries: Vec<IndexEntry>) -> Index {
        let mut dirs = vec![IndexDir {
            path: vec![0], // 根："" + NUL
            basename: Vec::new(),
            depth: 0,
            files: Vec::new(),
            subdirs: Vec::new(),
            st: None,
            unmatched: Vec::new(),
        }];
        let mut stack: Vec<usize> = vec![0];
        for (i, entry) in entries.iter().enumerate() {
            let path = &entry.path[..entry.path.len() - 1]; // 去 NUL
            // 与栈顶求公共目录前缀（对齐 CommonDir：含末尾 '/' 的长度 + 深度）
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
            // 弹掉公共深度之下的层
            while stack.len() > common_depth + 1 {
                stack.pop();
            }
            // 路径剩余部分逐层建子目录
            let mut p = common_len;
            while let Some(rel) = path[p..].iter().position(|&b| b == b'/') {
                let slash = p + rel;
                let parent = stack[stack.len() - 1];
                let parent_len = dirs[parent].path.len() - 1;
                let basename = path[parent_len..slash].to_vec();
                let mut dir_path = path[..=slash].to_vec(); // 含 '/'
                dir_path.push(0); // NUL
                let idx = dirs.len();
                dirs.push(IndexDir {
                    path: dir_path,
                    basename,
                    depth: stack.len(),
                    files: Vec::new(),
                    subdirs: Vec::new(),
                    st: None,
                    unmatched: Vec::new(),
                });
                dirs[parent].subdirs.push(idx);
                stack.push(idx);
                p = slash + 1;
            }
            // 条目落入栈顶目录
            let dir_idx = stack[stack.len() - 1];
            dirs[dir_idx].files.push(i);
        }
        Index {
            entries,
            dirs,
            splits: Vec::new(),
        }
    }

    /// 按 `16 * num_threads` 分片（对齐 InitSplits）：最小片权重 512，
    /// 按目录权重累计切分；splits 升序、无重复、首 0 尾 len。
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

    /// 并行收集脏候选（对齐 GetDirtyCandidates）：每片一个作用域线程，
    /// 结果合并、排序（大小写敏感按字节序，不敏感按 ASCII fold）、
    /// 按字节相等去重。root_fd 是仓库根目录 fd。
    pub fn get_dirty_candidates(
        &mut self,
        root_fd: RawFd,
        caps: &RepoCaps,
        opts: &ScanOpts,
    ) -> Vec<Vec<u8>> {
        let entries = &self.entries;
        let mut results: Vec<Vec<Vec<u8>>> = Vec::new();
        // 分片不重叠：从 dirs 头部逐片切 &mut 切片，交给各线程。
        std::thread::scope(|scope| {
            let mut rest: &mut [IndexDir] = &mut self.dirs;
            for pair in self.splits.windows(2) {
                let (from, to) = (pair[0], pair[1]);
                let (shard, tail) = std::mem::take(&mut rest).split_at_mut(to - from);
                results.push(
                    scope
                        .spawn(move || scan::scan_dirs(shard, entries, root_fd, caps, opts))
                        .join()
                        .expect("scan thread panicked"),
                );
                rest = tail;
            }
        });
        let mut out: Vec<Vec<u8>> = results.into_iter().flatten().collect();
        // 排序：大小写不敏感时按 ASCII fold 排（对齐 StrSort 的 C locale 语义）
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

    /// 目录权重（对齐 index.cc Weight：1 + subdirs + files）。
    pub fn weight(dir: &IndexDir) -> usize {
        1 + dir.subdirs.len() + dir.files.len()
    }
}

/// 单条目脏检测（对齐 index.cc IsModified）。
///
/// mode 先规范化：常规文件仅保留可执行位（`0755/0644`）；trust_filemode
/// 或 symlinks 能力缺失时跳过 mode 比较；非常规文件只比文件类型。
/// 比较序 ino → stage → fsize → mtime → mode，任一不等即候选。
/// mtime 的 nsec 特例：index 记录 nsec 为 0 时不比较 nsec（对齐
/// GITSTATUS_ZERO_NSEC：git 在 racy 检测后会把 nsec 归零）。
pub fn is_modified(entry: &IndexEntry, st: &libc::stat, caps: &RepoCaps) -> bool {
    let mut mode = st.st_mode;
    if mode & libc::S_IFMT == libc::S_IFREG {
        // symlinks 能力缺失且条目是符号链接，或 filemode 不可信 → 跳过 mode 比较
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
