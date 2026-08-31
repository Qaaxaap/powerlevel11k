//! 仓库句柄与缓存（对齐 `repo_cache.cc:86-149`）。
//!
//! # 常驻 vs 现算
//!
//! **常驻**（以 gitdir 为 key 的 map + LRU，TTL 到期才释放）：
//! - git 对象模型句柄：HEAD、分支、远端、tag 数据库、stash 列表、
//!   commit message/summary
//! - 已解析的 index 目录树（见 [`crate::index`]）
//! - staged / conflicted 差分结果（**按 head_oid 缓存**：HEAD 未变则复用，
//!   变了才用 `git_diff_tree_to_index` 重算；skip-worktree /
//!   assume-unchanged 计数同批缓存）
//!
//! **现算**（每次请求重新计算，靠缓存剪枝提速）：
//! - unstaged / untracked 的工作区遍历（见 [`crate::scan`]）
//!
//! # LRU 语义
//!
//! 主循环每次迭代调用 [`RepoCache::evict_expired`]；TTL（`-r`，默认
//! 3600 秒）从**最后一次访问**起算。原版实现为 map + 时间戳扫描，
//! p11k 可用等价的 LRU 结构，但淘汰条件必须一致：只按时间，不按容量。

use std::path::Path;

/// 单个仓库的常驻状态。字段在 M2 引入 git2 后补齐。
pub struct Repo {
    /// 仓库工作目录（请求中的 dir；GIT_DIR 请求时是 .git 的父目录）。
    pub workdir: String,
    // TODO(实现者)：补充——
    // - git 句柄（git2::Repository，M2）
    // - HEAD oid 缓存 + staged/conflicted 差分结果缓存
    // - 已解析的 index 树（crate::index::IndexDir）
    // - untracked cache（crate::untracked_cache::UntrackedCache）
    // - 最近访问时间戳（TTL 依据）
}

/// gitdir → [`Repo`] 的 LRU 缓存。
pub struct RepoCache {
    /// 闲置关闭秒数（`-r`；负值=永不过期）。
    pub ttl_seconds: i64,
    // TODO(实现者)：LRU 结构（如 HashMap<String, Repo> + 访问序队列）
}

impl RepoCache {
    /// 取出或打开一个仓库；打不开（不是 git 仓库等）返回 None，
    /// 调用方按"非仓库"响应处理。
    ///
    /// 实现要点：
    /// - `dir_is_gitdir = false` 时从 `dir` 向上逐级搜索 `.git`
    ///   （对齐 git 的 repository discovery：裸仓库、`.git` 文件指向
    ///   GIT_DIR 的 linked worktree 都要考虑）。
    /// - `dir_is_gitdir = true` 时直接把 `dir` 当 GIT_DIR 打开。
    /// - 命中缓存时刷新访问时间（TTL 依据）。
    pub fn get_or_open(&mut self, dir: &Path, dir_is_gitdir: bool) -> Option<&mut Repo> {
        let _ = (dir, dir_is_gitdir);
        todo!("实现：打开/搜索仓库，更新 LRU 访问序；失败返回 None")
    }

    /// 主循环每次迭代调用：关闭 TTL 到期的闲置仓库。
    pub fn evict_expired(&mut self) {
        todo!("实现：按最近访问时间与 ttl_seconds 淘汰并释放句柄")
    }
}
