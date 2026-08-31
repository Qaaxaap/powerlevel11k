//! git index 解析与脏候选扫描（性能核心，对齐 `index.cc`）。
//!
//! # 总体策略（请照搬算法，不必照搬代码）
//!
//! 1. **建树**：把 index 条目按路径建为目录树 [`IndexDir`]，每个目录
//!    持有其下条目的扁平列表。
//! 2. **分片**：按条目权重把顶层切片切成 `16 * num_threads` 片，
//!    线程池并行执行 [`get_dirty_candidates`]。
//! 3. **脏检测不调用 git**：对每个条目用 `openat/fstatat` 取磁盘 stat，
//!    与 index 里记录的 stat 比较 **ino / fsize / mtime(ns) / mode**
//!    （[`is_modified`]）。不匹配 = 候选脏路径。
//! 4. **提前终止**：只需要回答"是否存在脏文件/达到计数上限"，
//!    一命中上限就停，不做精确 diff。
//! 5. **精确确认**：候选路径在 repo 层再跑 `git_diff_index_to_workdir`
//!    确认（对候选子集做，不是全量）。
//! 6. **分配优化**：目录栈复用 + Arena 分配，避免逐级 path lookup 与
//!    堆碎片（这是原版快于 libgit2 的关键之一）。
//!
//! # index 格式
//!
//! v2/v3/v4 均有 12 字节头：`DIRC` + version(u32) + entry_count(u32)。
//! v4 的路径用变长整数编码 + 与前一条目的公共前缀压缩，解析时必须
//! 还原完整路径。p11k 先支持 v2（最常见）；v3/v4 解析失败时应报错
//! 并回退为"无法读 index"（对齐原版行为，不要 panic）。

/// index 中一个条目。stat 字段只存脏检测需要的四项。
pub struct IndexEntry {
    /// 完整路径（相对仓库根，字节串；git 路径无编码约定）。
    pub path: Vec<u8>,
    /// index 记录的 inode。
    pub ino: u64,
    /// index 记录的文件大小。
    pub fsize: u64,
    /// index 记录的 mtime：秒。
    pub mtime_sec: i64,
    /// index 记录的 mtime：纳秒部分。
    pub mtime_nsec: i64,
    /// index 记录的 mode。
    pub mode: u32,
    // TODO(实现者)：其余字段（oid、flags、extended flags、stage）按需补充；
    // 精确 diff 阶段需要 oid 与 stage。
}

/// index 的目录树形态：每个目录节点持有一份条目切片。
pub struct IndexDir {
    /// 目录名（根目录为空）。
    pub name: Vec<u8>,
    /// 直接位于本目录下的文件条目。
    pub files: Vec<IndexEntry>,
    /// 子目录。
    pub subdirs: Vec<IndexDir>,
}

/// 解析 `.git/index` 为目录树。
///
/// 实现要点：
/// - 校验 12 字节头与版本号；不支持 v4 压缩路径编码时返回 Err
///   （调用方按"index 不可读"降级处理）。
/// - 条目按路径排序是 v2+ 的保证，可用来二分；v4 解码后同样有序。
/// - 符号链接条目（mode 120000）走 `lstat` 而非 `fstatat`。
pub fn parse_index(data: &[u8]) -> Result<IndexDir, String> {
    let _ = data;
    todo!("实现：头校验 → 按版本解码条目 → 按 '/' 建树")
}

/// 单条目脏检测：比较 index 记录的 stat 与磁盘 stat。
///
/// 返回 true = **可能**已修改（候选，需精确 diff 确认）。
///
/// 实现要点：
/// - 磁盘 stat 经 `fstatat(dir_fd, basename, AT_SYMLINK_NOFOLLOW)` 获取，
///   dir_fd 是条目父目录的已打开 fd（openat 链），避免逐级路径查找。
/// - 比较顺序按成本排序：fsize → mtime(sec+nsec) → ino → mode。
///   任何一项不匹配即返回 true（原版 IsModified 语义）。
/// - index 未记录 stat（无 assume-valid 且 stat 全零）时按"已修改"处理。
pub fn is_modified(entry: &IndexEntry, dir_fd: i32) -> bool {
    let _ = (entry, dir_fd);
    todo!("实现：fstatat 取 stat，逐项比较 fsize/mtime/ino/mode")
}

/// 并行扫描，返回脏候选路径列表；达到上限即提前返回。
///
/// 实现要点：
/// - 分片：`16 * num_threads` 片，线程池（原版为手工线程池；p11k 可用
///   std::thread::scope + 每片一个任务）。
/// - 每片内复用目录栈：进入子目录不重新分配路径缓冲。
/// - 结果按需累计；一旦 unstaged/untracked 命中上限，其余片也应尽快
///   停止（原子标志 + 周期性检查）。
pub fn get_dirty_candidates(
    dir: &IndexDir,
    num_threads: usize,
    limits: &crate::scan::ScanLimits,
) -> Vec<Vec<u8>> {
    let _ = (dir, num_threads, limits);
    todo!("实现：切 16*num_threads 片，并行扫描，提前终止")
}
