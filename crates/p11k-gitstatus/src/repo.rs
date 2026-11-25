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
//!   `.git/index` is unchanged (`Repo::index_tree_or_rebuild`, in
//!   `index_stats`), then one `fstatat` per tracked file (stat refresh) plus
//!   one per directory (untracked-cache mtime check); readdir only on an
//!   untracked-cache miss. Candidates are classified without a git diff
//!   (`Repo::compute_dirty`).
//!   Baseline: the `iai` benchmark counts 3.1 M instructions for a warm
//!   `build_fields` on a 5000-file repository and 0.47 M on 50. Callgrind
//!   counts do not move with the machine, so they judge a change; the
//!   criterion harness reports wall-clock milliseconds for the figure you
//!   quote.
//!
//! # Known simplifications vs the original
//!
//! - The original runs staged scan, dirty scan, and tag query in parallel
//!   (RunAsync + Wait); p11k runs them sequentially. At 5000 files the whole
//!   call is one prompt's cheapest work, and a parallel split would save a
//!   fraction of it.
//! - The original shards `git_diff_tree_to_index` by path ranges; p11k does
//!   one full diff. The measured call covers that diff.
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

use crate::index::{Index, IndexEntry};
use crate::index_stats::{IndexStats, StagedStats};
use crate::options::Options;
use crate::protocol::field;
use crate::untracked_cache::UntrackedCache;
use git2::{Index as GitIndex, Oid, Repository as GitRepository, RepositoryState};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::time::Instant;

/// Copy entries from a git2 Index (paths get a trailing NUL, stat fields
/// taken from git2).
pub(crate) fn copy_entries(git_index: &GitIndex) -> Vec<IndexEntry> {
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

/// Resident state for one repo.
pub struct Repo {
    /// Absolute workdir path (no trailing '/'; byte-faithful).
    pub workdir: Vec<u8>,
    pub(crate) git: GitRepository,
    /// Oid HEAD points to; None for an empty repo (head_target).
    pub(crate) head_oid: Option<Oid>,
    /// Count caps and switches (the original saves Limits at Repo
    /// construction; here the whole Options is kept).
    pub(crate) limits: Options,
    /// HEAD the staged cache corresponds to (head_): recompute when changed.
    pub(crate) staged_head: Option<Oid>,
    /// Staged-diff cache.
    pub(crate) staged_stats: StagedStats,
    /// `.git/index` stat when staged_stats was computed. An index change
    /// invalidates the staged cache even when HEAD didn't move (the
    /// original's `git_index_read_ex` new_index → `head_ = {}`), e.g.
    /// `git add` after commit.
    pub(crate) staged_index_stat: Option<(i64, i64, i64)>,
    /// Untracked-cache probe (background thread).
    pub(crate) untracked: UntrackedCache,
    /// p11k index-tree cache: the dirs' untracked state (st/unmatched)
    /// persists with the tree; reused while the index is unchanged, with
    /// readdir pruned by mtime (the original's resident Index).
    pub(crate) index_tree: Option<Index>,
    /// `.git/index` (mtime_sec, mtime_nsec, size) at last build; rebuild
    /// when it changes.
    pub(crate) index_stat: Option<(i64, i64, i64)>,
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
            return;
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
            staged_index_stat: None,
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
        log::debug!(
            "build_fields: workdir={} skip_index={}",
            String::from_utf8_lossy(&self.workdir),
            skip_index
        );
        // Re-read HEAD on every request (mirrors the original gitstatus.cc calling `Head(repo)` each
        // time): a commit/checkout takes effect immediately, refreshing COMMIT and the staged cache together.
        self.head_oid = self
            .git
            .find_reference("HEAD")
            .ok()
            .and_then(|r| r.resolve().ok())
            .and_then(|r| r.target());
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
        if let Some(oid) = self.head_oid
            && let Ok(commit) = self.git.find_commit(oid)
        {
            f[field::COMMIT_ENCODING] = commit
                .message_encoding()
                .ok()
                .flatten()
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            let mut summary = commit
                .summary()
                .ok()
                .flatten()
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            // Truncate byte-wise (gitstatus.cc:44-46).
            summary.truncate(self.limits.max_commit_summary_length);
            f[field::COMMIT_SUMMARY] = summary;
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
            Err(_) => match head.symbolic_target().ok().flatten() {
                Some(t) if t.starts_with("refs/heads/") => t["refs/heads/".len()..].to_string(),
                _ => String::new(),
            },
        }
    }

    /// Tracking remote (git.cc GetRemote): read `branch.<name>.remote` and
    /// `branch.<name>.merge`; None (all three fields empty) without config
    /// or a local branch.
    ///
    /// The original has one more precondition (measured differentially, see the remote
    /// scenarios in tests/compat.rs): the remote must pass libgit2's `git_remote_lookup`,
    /// i.e. the config must have both `remote.<name>.url` (an empty string counts as
    /// present) and `remote.<name>.fetch`, and the upstream ref
    /// `refs/remotes/<remote>/<branch>` must exist; when any one is missing, the original
    /// leaves all three fields empty (`git_remote_lookup` / `git_branch_upstream` fail).
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
        let url = self.remote_or_none(&remote_name)?;
        let merge = cfg.get_string(&format!("branch.{branch}.merge")).ok()?;
        let short = merge.strip_prefix("refs/heads/")?;
        let ref_name = format!("refs/remotes/{remote_name}/{short}");
        self.git.find_reference(&ref_name).ok()?;
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
    /// Same preconditions as the tracking remote (non-push) path: non-empty URL + upstream ref present.
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
        let url = self.remote_or_none(&remote_name)?;
        let merge = cfg.get_string(&format!("branch.{branch}.merge")).ok()?;
        let short = merge.strip_prefix("refs/heads/")?;
        let ref_name = format!("refs/remotes/{remote_name}/{short}");
        self.git.find_reference(&ref_name).ok()?;
        Some(RemoteInfo {
            name: remote_name,
            branch: short.to_string(),
            url,
            ref_name,
        })
    }

    /// Resolve the remote URL, mirroring the original's `git_remote_lookup`: a remote with
    /// a url but no fetch refspec is treated as absent by the original (measured
    /// differentially), and returns None here too. An empty url is not a failure (the
    /// original still reports branch/name, leaving only the url field empty).
    fn remote_or_none(&self, name: &str) -> Option<String> {
        let remote = self.git.find_remote(name).ok()?;
        let has_fetch = remote
            .fetch_refspecs()
            .ok()?
            .iter()
            .any(|r| matches!(r, Ok(Some(s)) if !s.is_empty()));
        if !has_fetch {
            return None;
        }
        Some(remote.url().unwrap_or("").to_string())
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
                // A lightweight tag points straight at the commit; an annotated tag points at a
                // tag object, so peel it before comparing with HEAD.
                let matches = t
                    .resolve()
                    .ok()
                    .and_then(|r| r.peel(git2::ObjectType::Commit).ok())
                    .map(|c| c.id() == oid)
                    .unwrap_or(false);
                if matches
                    && let Ok(name) = t.shorthand()
                    && best.as_deref().map(|b| name > b).unwrap_or(true)
                {
                    best = Some(name.to_string());
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
}

/// Whether `dir` is a repository of its own: it holds a `.git` directory or file. git counts a
/// nested repository as one untracked entry and never descends into it.
pub(crate) fn has_git_dir(dir: &std::path::Path) -> bool {
    std::fs::symlink_metadata(dir.join(".git")).is_ok()
}

/// Whether a directory path is an index entry itself (a gitlink or submodule). Directory entries
/// are stored without a trailing slash.
pub(crate) fn in_index_exact(index: &Index, rel: &[u8]) -> bool {
    index
        .entries
        .binary_search_by(|entry| entry.path[..entry.path.len() - 1].cmp(rel))
        .is_ok()
}

/// Whether a workdir-relative path is in the index. Directories count as tracked when an entry
/// below them exists, because git stores files only.
pub(crate) fn rel_in_index(index: &Index, rel: &[u8], is_dir: bool) -> bool {
    if is_dir {
        let mut probe = rel.to_vec();
        probe.push(b'/');
        match index
            .entries
            .binary_search_by(|en| en.path[..en.path.len() - 1].cmp(probe.as_slice()))
        {
            Ok(_) => true,
            Err(pos) => {
                pos < index.entries.len() && index.entries[pos].path.starts_with(&probe[..])
            }
        }
    } else {
        index
            .entries
            .binary_search_by(|en| en.path[..en.path.len() - 1].cmp(rel))
            .is_ok()
    }
}
