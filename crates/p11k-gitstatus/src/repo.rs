//! 仓库句柄与缓存（对齐 `repo_cache.cc` 与 `gitstatus.cc` 的 ProcessRequest）。
//!
//! # 常驻 vs 现算
//!
//! **常驻**（以 gitdir 为 key 的 map + TTL，见 [`RepoCache::evict_expired`]）：
//! - git 对象模型句柄：HEAD、分支、远端、tag 数据库、stash 列表、commit message
//! - HEAD oid 缓存（staged 差分与 tag 查询的依据，对齐原版 head_target）
//!
//! **现算**（每次请求重新计算）：
//! - 本模块的字段组装（分支/远端/action/ahead-behind 等，libgit2 自身有缓存）
//! - TODO(index)：unstaged/untracked 的工作区遍历（下轮接入）
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

use crate::options::Options;
use crate::protocol::field;
use git2::{Oid, Repository as GitRepository, RepositoryState};
use std::collections::HashMap;
use std::ffi::OsStr;
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

/// 单个仓库的常驻状态。
pub struct Repo {
    /// 工作目录绝对路径（无尾部 /；字节忠实）。
    pub workdir: Vec<u8>,
    git: GitRepository,
    /// HEAD 指向的 oid；空仓库为 None（对齐原版 head_target）。
    head_oid: Option<Oid>,
    /// 计数上限与开关（原版 Repo 构造时保存 Limits，此处保存整个 Options）。
    limits: Options,
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
            GitRepository::open(dir).ok()?
        } else {
            // 对齐原版：向上搜索 .git（含 linked worktree 的 .git 文件）
            GitRepository::discover(dir).ok()?
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
        // 空仓库：find_reference("HEAD") 成功（symbolic），resolve() 失败 → None
        let head_oid = git
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.resolve().ok())
            .and_then(|r| r.target());
        Repo {
            workdir: workdir.as_os_str().as_bytes().to_vec(),
            git,
            head_oid,
            limits,
            last_used: Instant::now(),
        }
    }

    /// 组装 27 个数据字段（对齐 gitstatus.cc ProcessRequest 的 Print 顺序）。
    ///
    /// 8 个 dirty 统计字段 TODO(index)：index 扫描接入前 todo!() panic，
    /// 被 daemon 层 catch_unwind 捕获（该请求暂无响应）。
    /// allow(unreachable_code)：TODO(index) 接入后删除。
    #[allow(unreachable_code)]
    pub fn build_fields(&mut self) -> [Vec<u8>; field::COUNT] {
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

        // TODO(index)：index 扫描接入后填充 index_size 与 9 个 dirty 计数。
        todo!(
            "index 扫描接入后填充 index_size/num_staged/num_unstaged/num_conflicted/num_untracked/num_unstaged_deleted/num_staged_new/num_staged_deleted/num_skip_worktree/num_assume_unchanged"
        );

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

    /// 本地分支名；detached 或空仓库为空串（对齐 git.cc LocalBranchName）。
    fn local_branch(&self) -> String {
        self.git
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.resolve().ok())
            .map(|r| {
                if r.is_branch() {
                    r.shorthand().unwrap_or("").to_string()
                } else {
                    String::new()
                }
            })
            .unwrap_or_default()
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
}
