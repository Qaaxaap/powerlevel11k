//! 工作区遍历（对齐 `index.cc` 中 GetDirtyCandidates 的遍历部分）。
//!
//! # 遍历要求
//!
//! - **从父目录 fd 出发**：`openat(fd, name)` + `fstatat(fd, name)`，
//!   避免逐级绝对路径查找（每次 open 都是一次全路径 walk）。
//! - **目录栈复用**：进入/退出子目录时复用路径与 fd 缓冲，
//!   不重新分配（Arena 思路）。
//! - **untracked cache 剪枝**：目录 mtime 未变且缓存命中时跳过
//!   readdir（见 [`crate::untracked_cache`]）。
//! - **提前终止**：unstaged/untracked 命中上限后立即返回，
//!   上层并行扫描据此取消其他分片。
//!
//! # untracked 判定
//!
//! 工作区文件不在 index 中即 untracked；`.gitignore` 规则由
//! git 对象模型层提供（M2，git2 的 status 选项），本层只负责
//! 枚举文件与 stat。注意：git 语义里 `.git` 目录自身必须跳过。

/// 计数上限（对齐 `-s/-u/-c/-d/-e` 参数与 repo 层组合后的语义）。
#[derive(Debug, Clone, Copy)]
pub struct ScanLimits {
    /// unstaged（含 unstaged_deleted）计数上限，负值=无限。
    pub max_unstaged: i64,
    /// untracked 计数上限，负值=无限。
    pub max_untracked: i64,
    /// 是否递归展开 untracked 目录（`-e`）。
    pub recurse_untracked_dirs: bool,
    /// index 条目总数超过该值时整段跳过（`-m`，在 repo 层生效，
    /// 这里保留供上层判断）。负值=不启用。
    pub dirty_max_index_size: i64,
}

/// 遍历一个目录，输出脏候选路径（相对仓库根的字节串）。
/// 可提前返回；调用方在 [`crate::index::get_dirty_candidates`] 中
/// 负责分片与并行。
///
/// 实现要点：
/// - `fd` 为已打开的目录 fd；内部用 `openat` 打开子目录递归。
/// - 每轮 readdir 前检查上层传入的取消标志（原子 bool 或引用）。
/// - 返回的路径是**目录相对**路径，由调用方拼接目录前缀。
pub fn scan_dir(fd: i32, limits: &ScanLimits) -> Vec<Vec<u8>> {
    let _ = (fd, limits);
    todo!("实现：readdir + openat/fstatat + 目录栈复用 + untracked 判定 + 提前终止")
}
