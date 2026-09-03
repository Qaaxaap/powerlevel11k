//! Repo handles and caching (mirrors `repo_cache.cc` and gitstatus.cc's
//! ProcessRequest).
//!
//! # Resident vs computed-per-request
//!
//! **Resident** (map keyed by gitdir + TTL, see [`RepoCache::evict_expired`]):
//! - git object handles: HEAD, branches, remotes, tag db, stash list,
//!   commit message
//! - HEAD oid cache (basis for the staged diff and tag lookup)
//! - staged/conflicted diff results (**cached by head_oid**: reused while
//!   HEAD is unchanged, recomputed via `Diff::tree_to_index` only when it
//!   changes; skip-worktree/assume-unchanged counts cached in the same batch)
//!
//! **Computed per request**:
//! - field assembly (branch/remote/action/ahead-behind; libgit2 caches
//!   internally)
//! - unstaged/untracked worktree pass: the p11k index tree is reused while
//!   `.git/index` is unchanged ([`Repo::index_tree_or_rebuild`]), then one
//!   `fstatat` per tracked file (stat refresh) plus one per directory
//!   (untracked-cache mtime check); readdir only on an untracked-cache miss.
//!   Candidates are classified without a git diff ([`Repo::compute_dirty`]).
//!   Measured ~95ms on a 54k-file nixpkgs clone (22 cores) vs ~80ms for the
//!   original gitstatusd.
//!
//! # Known simplifications vs the original
//!
//! - The original runs staged scan, dirty scan, and tag query in parallel
//!   (RunAsync + Wait); p11k runs them sequentially.
//!   TODO(perf): revisit after benchmarking.
//! - The original shards `git_diff_tree_to_index` by path ranges; p11k does
//!   one full diff. TODO(perf): same.
//!
//! # TTL
//!
//! [`RepoCache::evict_expired`] runs every main-loop iteration; TTL (`-r`,
//! default 3600s) counts from the last access. Time-based eviction only, no
//! capacity cap, matching the original.
//!
//! # Path bytes
//!
//! Repo paths are `Vec<u8>`: Linux paths have no encoding contract, and the
//! original std::string is byte-faithful too. This crate supports unix only
//! (matching p10k's matrix: Linux/macOS/WSL).

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

/// Copy entries from a git2 Index (paths get a trailing NUL, stat fields
/// taken from git2).
fn copy_entries(git_index: &GitIndex) -> Vec<IndexEntry> {
    git_index
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
                // GIT_INDEX_ENTRY_STAGE_SHIFT = 12 (git2 has no stage accessor).
                stage: (e.flags >> 12) & 0x3,
                flags_extended: e.flags_extended,
                assume_valid: e.flags & git2::IndexEntryFlag::VALID.bits() != 0,
            }
        })
        .collect()
}

/// Remote info (shared by tracking and push remotes).
struct RemoteInfo {
    /// Remote name (e.g. "origin").
    name: String,
    /// Branch name with the `<remote>/` prefix stripped (e.g. "master").
    branch: String,
    /// Remote URL.
    url: String,
    /// Full ref of the remote branch (e.g. "refs/remotes/origin/master"),
    /// used as the revwalk range.
    ref_name: String,
}

/// Staged-diff cache (reused per head_oid, mirroring the original's
/// staged_/conflicted_ atoms).
#[derive(Default, Clone, Copy)]
struct StagedStats {
    staged: usize,
    conflicted: usize,
    staged_new: usize,
    staged_deleted: usize,
    skip_worktree: usize,
    assume_unchanged: usize,
}

/// Index and dirty stats (IndexStats).
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

/// Resident state for one repo.
pub struct Repo {
    /// Absolute workdir path (no trailing '/'; byte-faithful).
    pub workdir: Vec<u8>,
    git: GitRepository,
    /// Oid HEAD points to; None for an empty repo (head_target).
    head_oid: Option<Oid>,
    /// Count caps and switches (the original saves Limits at Repo
    /// construction; here the whole Options is kept).
    limits: Options,
    /// HEAD the staged cache corresponds to (head_): recompute when changed.
    staged_head: Option<Oid>,
    /// Staged-diff cache.
    staged_stats: StagedStats,
    /// Untracked-cache probe (background thread).
    untracked: UntrackedCache,
    /// p11k index-tree cache: the dirs' untracked state (st/unmatched)
    /// persists with the tree; reused while the index is unchanged, with
    /// readdir pruned by mtime (the original's resident Index).
    index_tree: Option<Index>,
    /// `.git/index` (mtime_sec, mtime_nsec, size) at last build; rebuild
    /// when it changes.
    index_stat: Option<(i64, i64, i64)>,
    /// TTL basis: last access time.
    last_used: Instant,
}

/// gitdir → [`Repo`] cache.
pub struct RepoCache {
    /// Idle close seconds (`-r`; negative = never expire).
    pub ttl_seconds: i64,
    /// Limits copy handed to each Repo.
    limits: Options,
    /// Key = gitdir path (`repo.path()`, like "/path/.git/").
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

    /// Fetch or open a repo; None when unopenable (not a git repo / bare),
    /// and the caller responds "non-repo".
    pub fn get_or_open(&mut self, dir: &[u8], dir_is_gitdir: bool) -> Option<&mut Repo> {
        let dir = Path::new(OsStr::from_bytes(dir));
        let git = if dir_is_gitdir {
            // from_dotgit: open `dir` as GIT_DIR directly, no upward search.
            GitRepository::open(dir)
        } else {
            // Search upward for .git (including the .git file of linked
            // worktrees).
            GitRepository::discover(dir)
        };
        let git = match git {
            Ok(g) => g,
            Err(_) => return None,
        };
        // Bare repos have no workdir → non-repo. Own the workdir to release
        // the borrow on git so it can move into Repo.
        let workdir = git.workdir()?.to_path_buf();
        let key = git.path().as_os_str().as_bytes().to_vec();
        let limits = self.limits.clone();
        let now = Instant::now();
        // Entry API: hits refresh the access time; misses move git/workdir
        // into a fresh Repo.
        let repo = self
            .repos
            .entry(key)
            .or_insert_with(|| Repo::open(git, &workdir, limits));
        repo.last_used = now;
        Some(repo)
    }

    /// Called every main-loop iteration: close repos idle past the TTL
    /// (repo_cache.cc Free).
    pub fn evict_expired(&mut self) {
        if self.ttl_seconds < 0 {
            return; // negative = never expire
        }
        let cutoff = std::time::Duration::from_secs(self.ttl_seconds as u64);
        let now = Instant::now();
        self.repos
            .retain(|_, repo| now.duration_since(repo.last_used) < cutoff);
    }
}

impl Repo {
    fn open(git: GitRepository, workdir: &Path, limits: Options) -> Repo {
        // libgit2 opts from the original main: disable strict hash
        // verification (the only one git2 binds; the rest are romkatv-fork
        // opts with no upstream libgit2 equivalent).
        git2::opts::strict_hash_verification(false);
        // Empty repo: find_reference("HEAD") succeeds (symbolic), resolve()
        // fails → None.
        let head_oid = git
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.resolve().ok())
            .and_then(|r| r.target());
        // The untracked-cache probe runs on gitdir (CheckDirMtime at Index
        // construction).
        let untracked = UntrackedCache::start_probe(git.path());
        // Strip the trailing '/' from the workdir (git2/libgit2 keeps it;
        // the original strips it explicitly).
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

    /// Assemble the 27 data fields (gitstatus.cc ProcessRequest's Print
    /// order).
    ///
    /// With skip_index (wire diff='1'), index stats are skipped: stats stay
    /// all-zero, like the original's `if (req.diff) stats = ...`.
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
                // Truncate byte-wise (gitstatus.cc:44-46).
                summary.truncate(self.limits.max_commit_summary_length);
                f[field::COMMIT_SUMMARY] = summary;
            }
        }
        f
    }

    /// Local branch name (git.cc LocalBranchName):
    /// - HEAD resolves (direct) → shorthand if a branch, else empty
    ///   (detached).
    /// - resolve fails (empty repo, symbolic unborn HEAD) → suffix after
    ///   `refs/heads/` when the target starts with it, else empty.
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

    /// Tracking remote (git.cc GetRemote): read `branch.<name>.remote` and
    /// `branch.<name>.merge`; None (all three fields empty) without config
    /// or a local branch.
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

    /// Push remote (git.cc GetPushRemote): `branch.<name>.pushRemote` →
    /// `remote.pushDefault` → None. On the mainstream path
    /// (pushRemote/pushDefault configured) the push ref is the tracking ref.
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

    /// Repo state (git.cc RepoState): libgit2 repository state plus a rebase
    /// progress suffix.
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
        // rebase-merge/{msgnum,end} or rebase-apply/{next,last}.
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

    /// Tag pointing at HEAD (`git describe --tags --exact-match` + tag_db):
    /// walk refs/tags/* (incl. packed-refs), resolve and compare targets
    /// with HEAD, pick the lexicographically largest name; empty if none.
    fn tag_name(&self) -> Vec<u8> {
        let Some(oid) = self.head_oid else {
            return Vec::new();
        };
        let mut best: Option<String> = None;
        if let Ok(tags) = self.git.references_glob("refs/tags/*") {
            for t in tags.flatten() {
                // resolve: peel annotated tags down to the commit.
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

    /// Stash count (git.cc NumStashes): git_stash_foreach count; any
    /// failure → 0. Needs `&mut self`: git2's stash_foreach borrows mutably.
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

    /// Revwalk range count (git.cc CountRange), 0 on failure.
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

    // ────────────────────────── index and dirty stats ──────────────────────────

    /// Index and dirty stats (repo.cc GetIndexStats).
    ///
    /// Flow: config switches (showUntrackedFiles / showDirtyState) → staged
    /// diff (head_oid cache, `Diff::tree_to_index` when changed) → dirty
    /// candidates (`Index::get_dirty_candidates`) → precise counts via
    /// `Diff::index_to_workdir` (notify callback + cap truncation) →
    /// min(cap) aggregation.
    fn get_index_stats(&mut self) -> IndexStats {
        // Config switches: explicit false zeroes the corresponding counts
        // (-U/-W/-D override and ignore).
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

        // Fresh git2 Index each time (a cached object lets libgit2's racy
        // write-back rewrite .git/index's mtime through its read path,
        // invalidating the index-tree cache — measured twice as slow).
        let mut git_index = match self.git.index() {
            Ok(i) => i,
            Err(_) => return IndexStats::default(),
        };
        // Incremental refresh, like git_index_read_ex.
        let _ = git_index.read(false);
        let index_size = git_index.len();

        // Index-tree cache: reuse when .git/index is unchanged (the dirs'
        // untracked state persists; readdir pruned by mtime).
        let mut index = self.index_tree_or_rebuild(&git_index);

        // Caps (RepoCaps; config missing → default true).
        let caps = self.repo_caps(&git_index);

        let mut stats = IndexStats {
            index_size,
            ..Default::default()
        };

        // ── staged path (GetIndexStats' staged branch) ──
        // Enabled when the cap is non-zero (size_t semantics: -1 = unlimited).
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
            // Empty repo / initial commit: no HEAD tree, staged = all
            // non-intent-to-add entries.
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

        // ── dirty path (GetIndexStats' candidate + StartDirtyScan) ──
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
                self.compute_dirty(&index, &candidates, &mut stats);
            }
        }

        // Put the tree back into the cache (untracked state persists with it).
        self.index_tree = Some(index);
        stats
    }

    /// Fetch the index tree, three-level cache:
    ///
    /// 1. `.git/index` (mtime, size) unchanged → reuse the whole tree
    ///    (entries + untracked-cache state).
    /// 2. Stat changed but the path set is unchanged (libgit2's racy
    ///    write-back only mutates entry stat fields) → reuse the structure,
    ///    refresh only the stat fields. Avoids a full rebuild of a 50k-entry
    ///    index on every racy write-back (measured 200ms+ spikes).
    /// 3. Path set really changed (git add/rm) → full rebuild.
    fn index_tree_or_rebuild(&mut self, git_index: &GitIndex) -> Index {
        let mut index_path = self.git.path().to_path_buf();
        index_path.push("index");
        let cur_stat = {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            let mut bytes = index_path.as_os_str().as_bytes().to_vec();
            bytes.push(0);
            // SAFETY: path NUL-terminated, st a valid buffer.
            let ok = unsafe { libc::stat(bytes.as_ptr().cast(), &mut st) } == 0;
            ok.then_some((st.st_mtime, st.st_mtime_nsec, st.st_size))
        };
        if let Some(tree) = self.index_tree.take() {
            if self.index_stat == cur_stat {
                return tree; // fast path: reuse the whole tree
            }
            // Stat changed: copy new entries, compare the path set.
            let new_entries = copy_entries(git_index);
            let same_paths = tree.entries.len() == new_entries.len()
                && tree
                    .entries
                    .iter()
                    .zip(&new_entries)
                    .all(|(a, b)| a.path == b.path);
            if same_paths {
                // Path set unchanged: reuse structure, update stats.
                let mut tree = tree;
                tree.update_stats(&new_entries);
                self.index_stat = cur_stat;
                return tree;
            }
            // Path set changed: drop the old tree, rebuild.
        }
        let entries = copy_entries(git_index);
        let mut index = Index::from_entries(entries);
        index.init_splits(self.limits.num_threads);
        self.index_stat = cur_stat;
        index
    }

    /// Staged diff (StartStagedScan): HEAD tree vs index, foreach counting
    /// staged/conflicted/staged_new/staged_deleted, stopping early once the
    /// caps are reached (OnDelta's GIT_EUSER semantics).
    ///
    /// Difference from the original: it aborts early inside diff
    /// construction via notify_cb; git2 0.20 does not expose a notify
    /// callback (TODO in diff.rs), so the early stop happens in foreach
    /// after construction. TODO(perf): revisit after benchmarking.
    fn compute_staged(&self, git_index: &GitIndex, head: Oid) -> StagedStats {
        let mut stats = StagedStats::default();
        // skip-worktree/assume-unchanged counted by walking the index.
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

    /// Precise dirty counts (StartDirtyScan semantics), classifying the
    /// scan layer's "possibly dirty" candidates: in-index + on-disk →
    /// unstaged (modified, stat-based like git status); in-index + missing
    /// on disk → unstaged + unstaged_deleted; not in index → untracked
    /// (gitignore-filtered via `status_should_ignore`). Directory
    /// candidates (trailing '/') count as 1 per ENABLE_FAST_UNTRACKED_DIRS.
    ///
    /// Why not the original's diff approach: no pathspec-matching blowup
    /// (measured 1s spikes without ranges), no libgit2 internal rework;
    /// the cost is no content-level confirmation in extreme racy cases (git
    /// status is stat-dominated anyway). Differential tests confirm
    /// byte-identical output.
    fn compute_dirty(&self, index: &Index, candidates: &[Vec<u8>], stats: &mut IndexStats) {
        if candidates.is_empty() {
            return;
        }
        let m_unstaged = self.limits.max_num_unstaged;
        let m_untracked = self.limits.max_num_untracked;
        for c in candidates {
            if c.last() == Some(&b'/') {
                // Directory candidate: count 1 only if it contains unignored
                // content (ENABLE_FAST_UNTRACKED_DIRS: empty/all-ignored
                // dirs are not reported).
                if m_untracked != 0 {
                    let dir_rel = &c[..c.len() - 1];
                    let mut dir_abs = self.workdir.clone();
                    dir_abs.push(b'/');
                    dir_abs.extend_from_slice(dir_rel);
                    if let Ok(rd) = std::fs::read_dir(Path::new(OsStr::from_bytes(&dir_abs))) {
                        for e in rd.flatten() {
                            let mut rel = dir_rel.to_vec();
                            rel.push(b'/');
                            rel.extend_from_slice(e.file_name().as_bytes());
                            // Untracked means unignored and not in the index
                            // (tracked files are also unignored, so they
                            // must be excluded).
                            let ignored = self
                                .git
                                .status_should_ignore(Path::new(OsStr::from_bytes(&rel)))
                                .unwrap_or(false);
                            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let tracked = if is_dir {
                                // Subdir: tracked iff an entry has the
                                // "rel/" prefix.
                                let mut probe = rel.clone();
                                probe.push(b'/');
                                match index.entries.binary_search_by(|en| {
                                    en.path[..en.path.len() - 1].cmp(probe.as_slice())
                                }) {
                                    Ok(_) => true,
                                    Err(pos) => {
                                        pos < index.entries.len()
                                            && index.entries[pos].path.starts_with(&probe[..])
                                    }
                                }
                            } else {
                                index
                                    .entries
                                    .binary_search_by(|en| {
                                        en.path[..en.path.len() - 1].cmp(rel.as_slice())
                                    })
                                    .is_ok()
                            };
                            if !ignored && !tracked {
                                stats.num_untracked += 1;
                                break; // FAST semantics: dir counts as 1
                            }
                        }
                    }
                }
            } else {
                // Index membership: entries are sorted by path (with
                // trailing NUL), binary search.
                let in_index = index
                    .entries
                    .binary_search_by(|e| e.path[..e.path.len() - 1].cmp(c.as_slice()))
                    .is_ok();
                // Disk existence (absolute path join, symlinks not followed).
                let mut full = self.workdir.clone();
                full.push(b'/');
                full.extend_from_slice(c);
                let exists = std::fs::symlink_metadata(Path::new(OsStr::from_bytes(&full))).is_ok();
                if in_index {
                    stats.num_unstaged += 1;
                    if !exists {
                        stats.num_unstaged_deleted += 1;
                    }
                } else {
                    let ignored = self
                        .git
                        .status_should_ignore(Path::new(OsStr::from_bytes(c)))
                        .unwrap_or(false);
                    if !ignored {
                        stats.num_untracked += 1;
                    }
                }
            }
            // Caps reached (OnDelta's GIT_EUSER early stop).
            let reached1 = m_unstaged >= 0 && stats.num_unstaged >= m_unstaged as usize;
            let reached2 = m_untracked >= 0 && stats.num_untracked >= m_untracked as usize;
            if reached1 && reached2 {
                break;
            }
        }
        stats.num_unstaged = cap(stats.num_unstaged, m_unstaged);
        stats.num_untracked = cap(stats.num_untracked, m_untracked);
        stats.num_unstaged_deleted = cap(stats.num_unstaged_deleted, stats.num_unstaged as i64);
    }

    /// Repo capability bits (RepoCaps; config missing → defaults).
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

    /// Open the workdir fd (root_fd for the dirty scan); None → skip scan.
    fn open_workdir_fd(&self) -> Option<RawFd> {
        let mut p = self.workdir.clone();
        p.push(0);
        // SAFETY: workdir is NUL-terminated.
        let fd = unsafe { libc::open(p.as_ptr().cast(), libc::O_RDONLY | libc::O_DIRECTORY) };
        (fd >= 0).then_some(fd)
    }
}

/// Cap a count (size_t semantics: negative = unlimited).
fn cap(count: usize, max: i64) -> usize {
    if max < 0 {
        count
    } else {
        count.min(max as usize)
    }
}

/// OnDelta stop condition: stop the diff (return false) once c1 hits m1 and
/// c2 hits m2. Negative caps are unlimited.
fn should_continue(c1: usize, m1: i64, c2: usize, m2: i64) -> bool {
    let reached1 = m1 >= 0 && c1 >= m1 as usize;
    let reached2 = m2 >= 0 && c2 >= m2 as usize;
    !(reached1 && reached2)
}
