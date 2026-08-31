//! 仓库句柄与缓存（对齐 `repo_cache.cc` 与 `gitstatus.cc` 的 ProcessRequest）。
//!
//! # 常驻 vs 现算
//!
//! **常驻**（以 gitdir 为 key 的 map + TTL，见 [`RepoCache::evict_expired`]）：
//! - git 对象模型句柄：HEAD、分支、远端、tag 数据库、stash 列表、commit message
//! - HEAD oid 缓存（staged 差分与 tag 查询的依据，对齐原版 head_target）
//! - staged/conflicted 差分结果（**按 head_oid 缓存**：HEAD 未变则复用，
//!   变了才用 `Diff::tree_to_index` 重算；skip-worktree/assume-unchanged
//!   计数同批缓存）
//!
//! **现算**（每次请求重新计算）：
//! - 本模块的字段组装（分支/远端/action/ahead-behind 等，libgit2 自身有缓存）
//! - unstaged/untracked 的工作区遍历（index 树每次重建，候选经
//!   `Diff::index_to_workdir` 精确确认；TODO(perf)：index 未变时复用树）
//!
//! # 与原版的已知简化
//!
//! - 原版 staged 扫描与 dirty 扫描、tag 查询并行（RunAsync + Wait）；
//!   p11k 当前顺序执行。TODO(perf)：benchmark 后再引入并行。
//! - 原版按路径区间分片跑 `git_diff_tree_to_index`；p11k 单次全量 diff。
//!   TODO(perf)：同上。
//!
//! # TTL 语义
//!
//! 主循环每次迭代调用 [`RepoCache::evict_expired`]；TTL（`-r`，默认
//! 3600 秒）从**最后一次访问**起算。对齐原版：只按时间淘汰，不按容量。
//!
//! # 路径字节
//!
//! 仓库路径用 `Vec<u8>` 承载：Linux 路径无编码约定，原版 std::string 同样
//! 字节忠实。本 crate 仅支持 unix 系（对齐 p10k 的支持矩阵：Linux/macOS/WSL）。

use crate::index::{Index, IndexEntry, RepoCaps};
use crate::options::Options;
use crate::protocol::field;
use crate::untracked_cache::UntrackedCache;
use git2::{DiffOptions, Index as GitIndex, Oid, Repository as GitRepository, RepositoryState};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

/// 远端信息（tracking 或 push 共用）。
struct RemoteInfo {
    /// remote 名（如 "origin"）。
    name: String,
    /// 剥掉 `<remote>/` 前缀的分支名（如 "master"）。
    branch: String,
    /// remote URL。
    url: String,
    /// 远端分支的完整 ref 名（如 "refs/remotes/origin/master"），revwalk 范围用。
    ref_name: String,
}

/// staged 差分缓存（按 head_oid 复用，对齐原版 staged_/conflicted_ 等原子量）。
#[derive(Default, Clone, Copy)]
struct StagedStats {
    staged: usize,
    conflicted: usize,
    staged_new: usize,
    staged_deleted: usize,
    skip_worktree: usize,
    assume_unchanged: usize,
}

/// index 与 dirty 统计（对齐原版 IndexStats）。
#[derive(Default)]
struct IndexStats {
    index_size: usize,
    num_staged: usize,
    num_unstaged: usize,
    num_conflicted: usize,
    num_untracked: usize,
    num_unstaged_deleted: usize,
    num_staged_new: usize,
    num_staged_deleted: usize,
    num_skip_worktree: usize,
    num_assume_unchanged: usize,
}

/// 单个仓库的常驻状态。
pub struct Repo {
    /// 工作目录绝对路径（无尾部 /；字节忠实）。
    pub workdir: Vec<u8>,
    git: GitRepository,
    /// HEAD 指向的 oid；空仓库为 None（对齐原版 head_target）。
    head_oid: Option<Oid>,
    /// 计数上限与开关（原版 Repo 构造时保存 Limits，此处保存整个 Options）。
    limits: Options,
    /// staged 缓存对应的 HEAD（对齐原版 head_：变了才重算）。
    staged_head: Option<Oid>,
    /// staged 差分缓存。
    staged_stats: StagedStats,
    /// untracked cache 探针（后台线程）。
    untracked: UntrackedCache,
    /// p11k index 树缓存：dirs 的 untracked 状态（st/unmatched）随树持久化，
    /// index 未变时复用，readdir 靠 mtime 剪枝（对齐原版常驻 Index）。
    index_tree: Option<Index>,
    /// 上次建树时 `.git/index` 的 (mtime_sec, mtime_nsec, size)；变了重建。
    index_stat: Option<(i64, i64, i64)>,
    /// TTL 依据：最后一次访问时刻。
    last_used: Instant,
}

/// gitdir → [`Repo`] 的缓存。
pub struct RepoCache {
    /// 闲置关闭秒数（`-r`；负值=永不过期）。
    pub ttl_seconds: i64,
    /// 传给每个 Repo 的 limits 副本。
    limits: Options,
    /// key = gitdir 路径（`repo.path()`，形如 "/path/.git/"）。
    repos: HashMap<Vec<u8>, Repo>,
}

impl RepoCache {
    pub fn new(options: &Options) -> RepoCache {
        RepoCache {
            ttl_seconds: options.repo_ttl_seconds,
            limits: options.clone(),
            repos: HashMap::new(),
        }
    }

    /// 取出或打开一个仓库；打不开（非 git 仓库 / bare 仓库）返回 None，
    /// 调用方按"非仓库"响应处理（对齐原版 ProcessRequest 的 `if (!repo) return`）。
    pub fn get_or_open(&mut self, dir: &[u8], dir_is_gitdir: bool) -> Option<&mut Repo> {
        let dir = Path::new(OsStr::from_bytes(dir));
        let git = if dir_is_gitdir {
            // 对齐原版 from_dotgit：直接把 dir 当 GIT_DIR 打开，不向上搜索
            GitRepository::open(dir)
        } else {
            // 对齐原版：向上搜索 .git（含 linked worktree 的 .git 文件）
            GitRepository::discover(dir)
        };
        let git = match git {
            Ok(g) => g,
            Err(_) => return None,
        };
        // bare 仓库无 workdir → 非仓库（对齐原版 workdir.len == 0 → return）。
        // workdir 转 owned 以解除对 git 的借用，便于 git 随后 move 进 Repo。
        let workdir = git.workdir()?.to_path_buf();
        let key = git.path().as_os_str().as_bytes().to_vec();
        let limits = self.limits.clone();
        let now = Instant::now();
        // entry API：命中刷新访问时间；未命中时 git/workdir move 进新建 Repo
        let repo = self
            .repos
            .entry(key)
            .or_insert_with(|| Repo::open(git, &workdir, limits));
        repo.last_used = now;
        Some(repo)
    }

    /// 主循环每次迭代调用：关闭 TTL 到期的闲置仓库（对齐 repo_cache.cc Free）。
    pub fn evict_expired(&mut self) {
        if self.ttl_seconds < 0 {
            return; // 负值 = 永不过期
        }
        let cutoff = std::time::Duration::from_secs(self.ttl_seconds as u64);
        let now = Instant::now();
        self.repos
            .retain(|_, repo| now.duration_since(repo.last_used) < cutoff);
    }
}

impl Repo {
    fn open(git: GitRepository, workdir: &Path, limits: Options) -> Repo {
        // 对齐原版 main 的 libgit2 opts：关闭严格 hash 校验（git2 唯一有绑定的
        // 一项；其余为 romkatv fork 专有 opts，官方 libgit2 无对应）
        git2::opts::strict_hash_verification(false);
        // 空仓库：find_reference("HEAD") 成功（symbolic），resolve() 失败 → None
        let head_oid = git
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.resolve().ok())
            .and_then(|r| r.target());
        // untracked cache 探针在 gitdir 上跑（对齐原版 Index 构造时的 CheckDirMtime）
        let untracked = UntrackedCache::start_probe(git.path());
        // 对齐原版：workdir 去尾部 '/'（git2/libgit2 的 workdir 带尾斜杠，
        // 原版 gitstatus.cc 显式 --workdir.len）
        let mut workdir_bytes = workdir.as_os_str().as_bytes().to_vec();
        if workdir_bytes.len() > 1 && workdir_bytes.last() == Some(&b'/') {
            workdir_bytes.pop();
        }
        Repo {
            workdir: workdir_bytes,
            git,
            head_oid,
            limits,
            staged_head: None,
            staged_stats: StagedStats::default(),
            untracked,
            index_tree: None,
            index_stat: None,
            last_used: Instant::now(),
        }
    }

    /// 组装 27 个数据字段（对齐 gitstatus.cc ProcessRequest 的 Print 顺序）。
    ///
    /// skip_index（线上 diff='1'）时跳过 index 统计：对齐原版
    /// `if (req.diff) stats = repo->GetIndexStats(...)`——跳过后
    /// stats 为默认全 0。
    pub fn build_fields(&mut self, skip_index: bool) -> [Vec<u8>; field::COUNT] {
        let mut f: [Vec<u8>; field::COUNT] = std::array::from_fn(|_| Vec::new());
        f[field::WORKDIR] = self.workdir.clone();
        f[field::COMMIT] = self
            .head_oid
            .map(|o| o.to_string().into_bytes())
            .unwrap_or_default();
        f[field::LOCAL_BRANCH] = self.local_branch().into_bytes();
        let remote = self.upstream_remote();
        if let Some(r) = &remote {
            f[field::REMOTE_BRANCH] = r.branch.clone().into_bytes();
            f[field::REMOTE_NAME] = r.name.clone().into_bytes();
            f[field::REMOTE_URL] = r.url.clone().into_bytes();
        }
        f[field::ACTION] = self.repo_state().into_bytes();

        let stats = if skip_index {
            IndexStats::default()
        } else {
            self.get_index_stats()
        };
        f[field::INDEX_SIZE] = stats.index_size.to_string().into_bytes();
        f[field::NUM_STAGED] = stats.num_staged.to_string().into_bytes();
        f[field::NUM_UNSTAGED] = stats.num_unstaged.to_string().into_bytes();
        f[field::NUM_CONFLICTED] = stats.num_conflicted.to_string().into_bytes();
        f[field::NUM_UNTRACKED] = stats.num_untracked.to_string().into_bytes();
        f[field::NUM_UNSTAGED_DELETED] = stats.num_unstaged_deleted.to_string().into_bytes();
        f[field::NUM_STAGED_NEW] = stats.num_staged_new.to_string().into_bytes();
        f[field::NUM_STAGED_DELETED] = stats.num_staged_deleted.to_string().into_bytes();
        f[field::NUM_SKIP_WORKTREE] = stats.num_skip_worktree.to_string().into_bytes();
        f[field::NUM_ASSUME_UNCHANGED] = stats.num_assume_unchanged.to_string().into_bytes();

        if let Some(r) = &remote {
            f[field::COMMITS_AHEAD] = self.count_range(&format!("{}..HEAD", r.ref_name));
            f[field::COMMITS_BEHIND] = self.count_range(&format!("HEAD..{}", r.ref_name));
        } else {
            f[field::COMMITS_AHEAD] = b"0".to_vec();
            f[field::COMMITS_BEHIND] = b"0".to_vec();
        }
        f[field::STASHES] = self.num_stashes().to_string().into_bytes();
        f[field::TAG] = self.tag_name();
        let push = self.push_remote();
        if let Some(p) = &push {
            f[field::PUSH_REMOTE_NAME] = p.name.clone().into_bytes();
            f[field::PUSH_REMOTE_URL] = p.url.clone().into_bytes();
            f[field::PUSH_COMMITS_AHEAD] = self.count_range(&format!("{}..HEAD", p.ref_name));
            f[field::PUSH_COMMITS_BEHIND] = self.count_range(&format!("HEAD..{}", p.ref_name));
        } else {
            f[field::PUSH_COMMITS_AHEAD] = b"0".to_vec();
            f[field::PUSH_COMMITS_BEHIND] = b"0".to_vec();
        }
        if let Some(oid) = self.head_oid {
            if let Ok(commit) = self.git.find_commit(oid) {
                f[field::COMMIT_ENCODING] =
                    commit.message_encoding().unwrap_or("").as_bytes().to_vec();
                let mut summary = commit.summary().unwrap_or("").as_bytes().to_vec();
                // 对齐原版 Truncate：字节级 resize（gitstatus.cc:44-46）
                summary.truncate(self.limits.max_commit_summary_length);
                f[field::COMMIT_SUMMARY] = summary;
            }
        }
        f
    }

    /// 本地分支名（对齐 git.cc LocalBranchName）：
    /// - HEAD resolve 成功（direct）→ 是分支则 shorthand，否则（detached）空串
    /// - resolve 失败（空仓库，symbolic unborn HEAD）→ target 以
    ///   `refs/heads/` 开头则返回其后缀，否则空串
    fn local_branch(&self) -> String {
        let Some(head) = self.git.find_reference("HEAD").ok() else {
            return String::new();
        };
        match head.resolve() {
            Ok(direct) => {
                if direct.is_branch() {
                    direct.shorthand().unwrap_or("").to_string()
                } else {
                    String::new()
                }
            }
            Err(_) => match head.symbolic_target() {
                Some(t) if t.starts_with("refs/heads/") => t["refs/heads/".len()..].to_string(),
                _ => String::new(),
            },
        }
    }

    /// tracking remote（对齐 git.cc GetRemote）：
    /// 读 config `branch.<name>.remote` 与 `branch.<name>.merge`；
    /// 未配置或无本地分支 → None（三字段全空）。
    fn upstream_remote(&self) -> Option<RemoteInfo> {
        let branch = self.local_branch();
        if branch.is_empty() {
            return None;
        }
        let cfg = self.git.config().ok()?;
        let remote_name = cfg.get_string(&format!("branch.{branch}.remote")).ok()?;
        if remote_name == "." {
            return None;
        }
        let url = self
            .git
            .find_remote(&remote_name)
            .ok()
            .map(|r| r.url().unwrap_or("").to_string())
            .unwrap_or_default();
        let merge = cfg.get_string(&format!("branch.{branch}.merge")).ok()?;
        let short = merge.strip_prefix("refs/heads/")?;
        let ref_name = format!("refs/remotes/{remote_name}/{short}");
        Some(RemoteInfo {
            name: remote_name,
            branch: short.to_string(),
            url,
            ref_name,
        })
    }

    /// push remote（对齐 git.cc GetPushRemote）：
    /// `branch.<name>.pushRemote` → `remote.pushDefault` → None。
    /// 主流路径（pushRemote/pushDefault 配置）push ref 取 tracking ref。
    fn push_remote(&self) -> Option<RemoteInfo> {
        let branch = self.local_branch();
        if branch.is_empty() {
            return None;
        }
        let cfg = self.git.config().ok()?;
        let remote_name = cfg
            .get_string(&format!("branch.{branch}.pushremote"))
            .or_else(|_| cfg.get_string("remote.pushdefault"))
            .ok()?;
        if remote_name == "." {
            return None;
        }
        let url = self
            .git
            .find_remote(&remote_name)
            .ok()
            .map(|r| r.url().unwrap_or("").to_string())
            .unwrap_or_default();
        let merge = cfg.get_string(&format!("branch.{branch}.merge")).ok()?;
        let short = merge.strip_prefix("refs/heads/")?;
        let ref_name = format!("refs/remotes/{remote_name}/{short}");
        Some(RemoteInfo {
            name: remote_name,
            branch: short.to_string(),
            url,
            ref_name,
        })
    }

    /// 仓库状态（对齐 git.cc RepoState）：libgit2 的 repository state + rebase 进度后缀。
    fn repo_state(&self) -> String {
        let state = match self.git.state() {
            RepositoryState::Clean => "",
            RepositoryState::Merge => "merge",
            RepositoryState::Revert => "revert",
            RepositoryState::RevertSequence => "revert-seq",
            RepositoryState::CherryPick => "cherry",
            RepositoryState::CherryPickSequence => "cherry-seq",
            RepositoryState::Bisect => "bisect",
            RepositoryState::Rebase => "rebase",
            RepositoryState::RebaseInteractive => "rebase-i",
            RepositoryState::RebaseMerge => "rebase-m",
            RepositoryState::ApplyMailbox => "am",
            RepositoryState::ApplyMailboxOrRebase => "am/rebase",
        };
        let gitdir = self.git.path();
        // 对齐原版：rebase-merge/{msgnum,end} 或 rebase-apply/{next,last}
        let (next_file, last_file) = if gitdir.join("rebase-merge").is_dir() {
            ("rebase-merge/msgnum", "rebase-merge/end")
        } else if gitdir.join("rebase-apply").is_dir() {
            ("rebase-apply/next", "rebase-apply/last")
        } else {
            ("", "")
        };
        if next_file.is_empty() {
            return state.to_string();
        }
        let read = |f: &str| {
            std::fs::read_to_string(gitdir.join(f))
                .ok()
                .map(|s| s.trim().to_string())
                .unwrap_or_default()
        };
        let next = read(next_file);
        let last = read(last_file);
        if next.is_empty() || last.is_empty() {
            state.to_string()
        } else {
            format!("{state} {next}/{last}")
        }
    }

    /// 指向 HEAD 的 tag 名（对齐 `git describe --tags --exact-match` + tag_db）：
    /// 遍历 refs/tags/*（含 packed-refs），resolve 后 target 等于 HEAD 的
    /// 候选里选**字典序最大**者；无则空串。
    fn tag_name(&self) -> Vec<u8> {
        let Some(oid) = self.head_oid else {
            return Vec::new();
        };
        let mut best: Option<String> = None;
        if let Ok(tags) = self.git.references_glob("refs/tags/*") {
            for t in tags.flatten() {
                // resolve：annotated tag 剥到 commit 再比对
                let matches = t
                    .resolve()
                    .ok()
                    .and_then(|r| r.target())
                    .map(|o| o == oid)
                    .unwrap_or(false);
                if matches {
                    if let Some(name) = t.shorthand() {
                        if best.as_deref().map(|b| name > b).unwrap_or(true) {
                            best = Some(name.to_string());
                        }
                    }
                }
            }
        }
        best.map(String::into_bytes).unwrap_or_default()
    }

    /// stash 数（对齐 git.cc NumStashes）：git_stash_foreach 计数；
    /// 任一步失败回 0（对齐原版 WARN 后 return 0）。
    /// 需要 `&mut self`：git2 的 stash_foreach 借用可变。
    fn num_stashes(&mut self) -> usize {
        let mut n = 0usize;
        match self.git.stash_foreach(|_, _, _| {
            n += 1;
            true
        }) {
            Ok(()) => n,
            Err(_) => 0,
        }
    }

    /// revwalk 范围计数（对齐 git.cc CountRange），失败回 0。
    fn count_range(&self, range: &str) -> Vec<u8> {
        let count = self
            .git
            .revwalk()
            .ok()
            .and_then(|mut w| w.push_range(range).ok().map(|_| w))
            .map(|w| w.count())
            .unwrap_or(0);
        count.to_string().into_bytes()
    }

    // ────────────────────────── index 与 dirty 统计 ──────────────────────────

    /// index 与 dirty 统计（对齐 repo.cc GetIndexStats）。
    ///
    /// 流程：config 开关（showUntrackedFiles / showDirtyState）→ staged
    /// 差分（head_oid 缓存，变了才 `Diff::tree_to_index`）→ dirty 候选
    /// （`Index::get_dirty_candidates`）→ `Diff::index_to_workdir` 精确计数
    /// （notify 回调 + 上限截断）→ min(cap) 汇总。
    fn get_index_stats(&mut self) -> IndexStats {
        // config 开关（对齐 Off lambda：config 显式 false 时清零对应计数；
        // -U/-W/-D 参数可覆盖忽略）
        if let Ok(cfg) = self.git.config() {
            let off = |name: &str| cfg.get_bool(name).map(|v| !v).unwrap_or(false);
            if !self.limits.ignore_status_show_untracked_files && off("status.showUntrackedFiles") {
                self.limits.max_num_untracked = 0;
            }
            if !self.limits.ignore_bash_show_untracked_files && off("bash.showUntrackedFiles") {
                self.limits.max_num_untracked = 0;
            }
            if !self.limits.ignore_bash_show_dirty_state && off("bash.showDirtyState") {
                self.limits.max_num_staged = 0;
                self.limits.max_num_unstaged = 0;
                self.limits.max_num_conflicted = 0;
            }
        }

        // git2 Index：每次新对象（缓存对象会导致 libgit2 复用 read 路径的
        // racy 写回改写 .git/index mtime，进而使 index 树缓存失效——
        // 实测比不缓存慢一倍，故不缓存）
        let mut git_index = match self.git.index() {
            Ok(i) => i,
            Err(_) => return IndexStats::default(),
        };
        // 对齐原版 git_index_read_ex 的增量刷新
        let _ = git_index.read(false);
        let index_size = git_index.len();

        // index 树缓存：.git/index 未变则复用（dirs 的 untracked 状态持久化，
        // readdir 靠 mtime 剪枝；对齐原版常驻 Index 对象）
        let mut index = self.index_tree_or_rebuild(&git_index);

        // caps（对齐 RepoCaps；config 缺失按默认 true）
        let caps = self.repo_caps(&git_index);

        let mut stats = IndexStats {
            index_size,
            ..Default::default()
        };

        // ── staged 路径（对齐 GetIndexStats 的 staged 分支）──
        // 上限非零即启用（对齐原版 size_t 语义：-1 = SIZE_MAX = 无限）
        let want_staged = self.limits.max_num_staged != 0 || self.limits.max_num_conflicted != 0;
        if !want_staged {
            self.staged_head = None;
            self.staged_stats = StagedStats::default();
        } else if let Some(head) = self.head_oid {
            if self.staged_head != Some(head) {
                self.staged_head = Some(head);
                self.staged_stats = self.compute_staged(&git_index, head);
            }
        } else {
            // 空仓库/初始提交：无 HEAD 树，staged = 全部非 intent-to-add 条目
            self.staged_head = None;
            let mut staged = 0usize;
            let mut skip = 0usize;
            let mut assume = 0usize;
            for e in git_index.iter() {
                if e.flags_extended & git2::IndexEntryExtendedFlag::INTENT_TO_ADD.bits() == 0 {
                    staged += 1;
                }
                if e.flags_extended & git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits() != 0 {
                    skip += 1;
                }
                if e.flags & git2::IndexEntryFlag::VALID.bits() != 0 {
                    assume += 1;
                }
            }
            self.staged_stats = StagedStats {
                staged,
                conflicted: 0,
                staged_new: staged,
                staged_deleted: 0,
                skip_worktree: skip,
                assume_unchanged: assume,
            };
        }
        stats.num_staged = cap(self.staged_stats.staged, self.limits.max_num_staged);
        stats.num_conflicted = cap(self.staged_stats.conflicted, self.limits.max_num_conflicted);
        stats.num_staged_new = cap(self.staged_stats.staged_new, stats.num_staged as i64);
        stats.num_staged_deleted = cap(self.staged_stats.staged_deleted, stats.num_staged as i64);
        stats.num_skip_worktree = self.staged_stats.skip_worktree;
        stats.num_assume_unchanged = self.staged_stats.assume_unchanged;

        // ── dirty 路径（对齐 GetIndexStats 的候选 + StartDirtyScan）──
        let dirty_allowed = self.limits.dirty_max_index_size < 0
            || index_size <= self.limits.dirty_max_index_size as usize;
        if dirty_allowed
            && (self.limits.max_num_unstaged != 0 || self.limits.max_num_untracked != 0)
        {
            let root_fd = self.open_workdir_fd();
            if let Some(root_fd) = root_fd {
                let opts = crate::scan::ScanOpts {
                    include_untracked: self.limits.max_num_untracked != 0,
                    untracked_cache_enabled: self.untracked.enabled(),
                };
                let candidates = index.get_dirty_candidates(root_fd, &caps, &opts);
                unsafe { libc::close(root_fd) };
                self.compute_dirty(&git_index, &candidates, &mut stats);
            }
        }

        // 扫描完成后把树放回缓存（untracked 状态随树持久化）
        self.index_tree = Some(index);
        stats
    }

    /// 取 index 树：`.git/index` 的 (mtime, size) 未变则复用缓存树
    /// （保留 untracked cache 状态），否则从 git2 条目重建。
    fn index_tree_or_rebuild(&mut self, git_index: &GitIndex) -> Index {
        let mut index_path = self.git.path().to_path_buf();
        index_path.push("index");
        let cur_stat = {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let mut bytes = index_path.as_os_str().as_bytes().to_vec();
            bytes.push(0);
            // SAFETY: 路径 NUL 结尾，st 合法缓冲。
            let ok = unsafe { libc::stat(bytes.as_ptr().cast(), &mut st) } == 0;
            ok.then_some((st.st_mtime, st.st_mtime_nsec, st.st_size))
        };
        if let Some(tree) = self.index_tree.take() {
            if self.index_stat == cur_stat {
                eprintln!("DBG 树复用");
                return tree; // 复用
            }
            eprintln!("DBG 树重建: {:?} vs {:?}", self.index_stat, cur_stat);
        }
        // 重建：条目拷贝 + 建树 + 分片
        let entries: Vec<IndexEntry> = git_index
            .iter()
            .map(|e| {
                let mut p = e.path.clone();
                p.push(0);
                IndexEntry {
                    path: p,
                    ino: e.ino,
                    fsize: e.file_size,
                    mtime_sec: e.mtime.seconds(),
                    mtime_nsec: e.mtime.nanoseconds(),
                    mode: e.mode,
                    // GIT_INDEX_ENTRY_STAGE_SHIFT = 12（git2 无 stage 访问器）
                    stage: (e.flags >> 12) & 0x3,
                    flags_extended: e.flags_extended,
                    assume_valid: e.flags & git2::IndexEntryFlag::VALID.bits() != 0,
                }
            })
            .collect();
        let mut index = Index::from_entries(entries);
        index.init_splits(self.limits.num_threads);
        self.index_stat = cur_stat;
        index
    }

    /// staged 差分（对齐 StartStagedScan）：HEAD tree vs index，
    /// foreach 计数 staged/conflicted/staged_new/staged_deleted，
    /// 达到上限后提前停止遍历（对齐 OnDelta 的 GIT_EUSER 语义）。
    ///
    /// 与原版差异：原版靠 notify_cb 在 diff 构造中提前终止；git2 0.20
    /// 未暴露 notify 回调（diff.rs 中 TODO），改为构造后 foreach 提前停止。
    /// TODO(perf)：benchmark 后再评估。
    fn compute_staged(&self, git_index: &GitIndex, head: Oid) -> StagedStats {
        let mut stats = StagedStats::default();
        // skip-worktree/assume-unchanged 遍历 index 统计（对齐原版分片内统计）
        for e in git_index.iter() {
            if e.flags_extended & git2::IndexEntryExtendedFlag::SKIP_WORKTREE.bits() != 0 {
                stats.skip_worktree += 1;
            }
            if e.flags & git2::IndexEntryFlag::VALID.bits() != 0 {
                stats.assume_unchanged += 1;
            }
        }
        let Ok(commit) = self.git.find_commit(head) else {
            return stats;
        };
        let Ok(tree) = commit.tree() else {
            return stats;
        };
        let mut opts = DiffOptions::new();
        opts.include_typechange_trees(true);
        let Ok(diff) = self
            .git
            .diff_tree_to_index(Some(&tree), Some(git_index), Some(&mut opts))
        else {
            return stats;
        };
        let m_staged = self.limits.max_num_staged;
        let m_conflicted = self.limits.max_num_conflicted;
        let s = &mut stats;
        let _ = diff.foreach(
            &mut |delta, _| {
                if delta.status() == git2::Delta::Conflicted {
                    s.conflicted += 1;
                    should_continue(s.conflicted, m_conflicted, s.staged, m_staged)
                } else {
                    if delta.status() == git2::Delta::Added {
                        s.staged_new += 1;
                    }
                    if delta.status() == git2::Delta::Deleted {
                        s.staged_deleted += 1;
                    }
                    s.staged += 1;
                    should_continue(s.staged, m_staged, s.conflicted, m_conflicted)
                }
            },
            None,
            None,
            None,
        );
        stats
    }

    /// dirty 精确计数（对齐 StartDirtyScan）：候选 pathspec 的
    /// index_to_workdir diff，foreach 计数 untracked/unstaged/unstaged_deleted。
    ///
    /// 与原版差异：① 无 notify_cb（git2 0.20 未暴露），foreach 后置计数；
    /// ② 无 ignore_submodules（同上）——脏 submodule 计数可能偏多，
    /// TODO(submodule)；③ 无 GIT_DIFF_EXEMPLARS（builder 无对应开关）。
    fn compute_dirty(&self, git_index: &GitIndex, candidates: &[Vec<u8>], stats: &mut IndexStats) {
        if candidates.is_empty() {
            return;
        }
        let m_unstaged = self.limits.max_num_unstaged;
        let m_untracked = self.limits.max_num_untracked;
        let mut opts = DiffOptions::new();
        opts.include_typechange_trees(true)
            .skip_binary_check(true)
            .disable_pathspec_match(true);
        if m_untracked != 0 {
            opts.include_untracked(true);
            if self.limits.recurse_untracked_dirs {
                opts.recurse_untracked_dirs(true);
            }
        } else {
            opts.enable_fast_untracked_dirs(true);
        }
        // pathspec：候选路径（累积式，对齐原版 pathspec 数组）
        for c in candidates {
            if let Ok(cs) = std::ffi::CString::new(c.clone()) {
                opts.pathspec(cs);
            }
        }
        let Ok(diff) = self
            .git
            .diff_index_to_workdir(Some(git_index), Some(&mut opts))
        else {
            return;
        };
        let s = &mut *stats;
        let _ = diff.foreach(
            &mut |delta, _| {
                if delta.status() == git2::Delta::Conflicted {
                    true // 冲突在 workdir diff 中不计数（对齐原版 DO_NOT_INSERT）
                } else if delta.status() == git2::Delta::Untracked {
                    s.num_untracked += 1;
                    should_continue(s.num_untracked, m_untracked, s.num_unstaged, m_unstaged)
                } else {
                    if delta.status() == git2::Delta::Deleted {
                        s.num_unstaged_deleted += 1;
                    }
                    s.num_unstaged += 1;
                    should_continue(s.num_unstaged, m_unstaged, s.num_untracked, m_untracked)
                }
            },
            None,
            None,
            None,
        );
        stats.num_unstaged = cap(stats.num_unstaged, m_unstaged);
        stats.num_untracked = cap(stats.num_untracked, m_untracked);
        stats.num_unstaged_deleted = cap(stats.num_unstaged_deleted, stats.num_unstaged as i64);
    }

    /// 仓库能力位（对齐 RepoCaps；config 缺失按默认值）。
    fn repo_caps(&self, _index: &GitIndex) -> RepoCaps {
        let get_bool = |name: &str, default: bool| {
            self.git
                .config()
                .ok()
                .and_then(|c| c.get_bool(name).ok())
                .unwrap_or(default)
        };
        RepoCaps {
            trust_filemode: get_bool("core.filemode", true),
            has_symlinks: get_bool("core.symlinks", true),
            case_sensitive: !get_bool("core.ignorecase", false),
        }
    }

    /// 打开工作目录 fd（dirty 扫描的 root_fd）；失败 None（跳过扫描）。
    fn open_workdir_fd(&self) -> Option<RawFd> {
        let mut p = self.workdir.clone();
        p.push(0);
        // SAFETY: workdir 已 NUL 结尾。
        let fd = unsafe { libc::open(p.as_ptr().cast(), libc::O_RDONLY | libc::O_DIRECTORY) };
        (fd >= 0).then_some(fd)
    }
}

/// 计数上限截断（对齐原版 size_t 上限语义：负值 = 无限）。
fn cap(count: usize, max: i64) -> usize {
    if max < 0 {
        count
    } else {
        count.min(max as usize)
    }
}

/// 对齐 OnDelta 的停止条件：c1 达到 m1 且 c2 达到 m2 → 停止 diff（返回 false）。
/// 负值上限 = 无限（永不到达）。
fn should_continue(c1: usize, m1: i64, c2: usize, m2: i64) -> bool {
    let reached1 = m1 >= 0 && c1 >= m1 as usize;
    let reached2 = m2 >= 0 && c2 >= m2 as usize;
    !(reached1 && reached2)
}
