//! Index and dirty worktree statistics for [`Repo`].
//!
//! Split from `repo.rs`: the resident repo handle and the per-request stat work were in one
//! file, and `get_index_stats` alone mixed the request limits, the staged cache, the dirty scan
//! and the counter assembly. The flow is [`Repo::get_index_stats`].

use crate::index::{Index, RepoCaps};
use crate::options::Options;
use crate::repo::{Repo, copy_entries, has_git_dir, in_index_exact, rel_in_index};
use crate::sys;
use git2::{DiffOptions, Index as GitIndex, Oid};
use std::ffi::OsStr;
use std::os::fd::RawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Staged-diff cache (reused per head_oid, mirroring the original's
/// staged_/conflicted_ atoms).
#[derive(Default, Clone, Copy)]
pub(crate) struct StagedStats {
    pub(crate) staged: usize,
    pub(crate) conflicted: usize,
    pub(crate) staged_new: usize,
    pub(crate) staged_deleted: usize,
    pub(crate) skip_worktree: usize,
    pub(crate) assume_unchanged: usize,
}

/// Index and dirty stats (IndexStats).
#[derive(Default)]
pub(crate) struct IndexStats {
    pub(crate) index_size: usize,
    pub(crate) num_staged: usize,
    pub(crate) num_unstaged: usize,
    pub(crate) num_conflicted: usize,
    pub(crate) num_untracked: usize,
    pub(crate) num_unstaged_deleted: usize,
    pub(crate) num_staged_new: usize,
    pub(crate) num_staged_deleted: usize,
    pub(crate) num_skip_worktree: usize,
    pub(crate) num_assume_unchanged: usize,
}

impl Repo {
    /// Index and dirty stats (repo.cc GetIndexStats).
    ///
    /// Flow: request limits ([`Self::effective_limits`]) → staged diff
    /// ([`Self::staged_stats`], cached by HEAD and the index stat) → dirty candidates
    /// ([`Self::dirty_stats`]) → cap aggregation.
    pub(crate) fn get_index_stats(&mut self) -> IndexStats {
        let limits = self.effective_limits();
        // Fresh git2 Index each time (a cached object lets libgit2's racy write-back rewrite
        // .git/index's mtime through its read path, invalidating the index-tree cache — measured
        // twice as slow).
        let mut git_index = match self.git.index() {
            Ok(i) => i,
            Err(_) => return IndexStats::default(),
        };
        // Incremental refresh, like git_index_read_ex.
        let _ = git_index.read(false);
        let index_size = git_index.len();
        // Index-tree cache: reuse when .git/index is unchanged (the dirs' untracked state
        // persists; readdir pruned by mtime).
        let mut index = self.index_tree_or_rebuild(&git_index, limits.max_num_untracked != 0);
        let mut stats = IndexStats {
            index_size,
            ..Default::default()
        };
        self.staged_stats(&limits, &git_index, &mut stats);
        self.dirty_stats(&limits, &mut index, &mut stats);
        // The counters are final after the dirty scan; log them here so diagnostics show the
        // values the response carries.
        log::debug!("{}", index_stats_line(&stats));
        // Put the tree back into the cache (untracked state persists with it).
        self.index_tree = Some(index);
        stats
    }

    /// The limits for this request: the construction-time copy, with the config switches that
    /// zero counts applied.
    ///
    /// The effect is per request. The original keeps the `Limits` it was handed at Repo
    /// construction and restores them before every request. Writing into `self.limits` instead
    /// makes the zeroing permanent for the lifetime of the process: flipping
    /// `status.showUntrackedFiles` back to true then has no effect.
    fn effective_limits(&self) -> Options {
        let mut limits = self.limits.clone();
        if let Ok(cfg) = self.git.config() {
            let off = |name: &str| cfg.get_bool(name).map(|v| !v).unwrap_or(false);
            if !limits.ignore_status_show_untracked_files && off("status.showUntrackedFiles") {
                limits.max_num_untracked = 0;
            }
            if !limits.ignore_bash_show_untracked_files && off("bash.showUntrackedFiles") {
                limits.max_num_untracked = 0;
            }
            if !limits.ignore_bash_show_dirty_state && off("bash.showDirtyState") {
                limits.max_num_staged = 0;
                limits.max_num_unstaged = 0;
                limits.max_num_conflicted = 0;
            }
        }
        limits
    }

    /// Staged and conflicted counts (GetIndexStats' staged branch), cached by HEAD and by the
    /// index stat.
    fn staged_stats(&mut self, limits: &Options, git_index: &GitIndex, stats: &mut IndexStats) {
        // Enabled when the cap is non-zero (size_t semantics: -1 = unlimited).
        let want_staged = limits.max_num_staged != 0 || limits.max_num_conflicted != 0;
        if !want_staged {
            self.staged_head = None;
            self.staged_stats = StagedStats::default();
            self.staged_index_stat = None;
        } else if let Some(head) = self.head_oid {
            // The cache is invalidated when HEAD changes **or the index changes** (e.g. `git add`
            // only touches the index, not HEAD; mirrors the original git_index_read_ex new_index
            // → head_ = {}).
            let index_stat = self.index_stat;
            let index_changed = index_stat != self.staged_index_stat;
            if self.staged_head != Some(head) || index_changed {
                self.staged_head = Some(head);
                self.staged_index_stat = index_stat;
                self.staged_stats = self.compute_staged(git_index, head);
            }
        } else {
            // Empty repo / initial commit: no HEAD tree, staged = all non-intent-to-add entries.
            self.staged_head = None;
            self.staged_index_stat = None;
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
        stats.num_staged = cap(self.staged_stats.staged, limits.max_num_staged);
        stats.num_conflicted = cap(self.staged_stats.conflicted, limits.max_num_conflicted);
        stats.num_staged_new = cap(self.staged_stats.staged_new, stats.num_staged as i64);
        stats.num_staged_deleted = cap(self.staged_stats.staged_deleted, stats.num_staged as i64);
        stats.num_skip_worktree = self.staged_stats.skip_worktree;
        stats.num_assume_unchanged = self.staged_stats.assume_unchanged;
    }

    /// Unstaged and untracked counts (GetIndexStats' candidate + StartDirtyScan).
    fn dirty_stats(&mut self, limits: &Options, index: &mut Index, stats: &mut IndexStats) {
        let dirty_allowed = limits.dirty_max_index_size < 0
            || stats.index_size <= limits.dirty_max_index_size as usize;
        if !dirty_allowed || (limits.max_num_unstaged == 0 && limits.max_num_untracked == 0) {
            return;
        }
        let caps = self.repo_caps();
        let Some(root_fd) = self.open_workdir_fd() else {
            return;
        };
        let opts = crate::scan::ScanOpts {
            include_untracked: limits.max_num_untracked != 0,
            untracked_cache_enabled: self.untracked.enabled(),
        };
        let candidates = index.get_dirty_candidates(root_fd, &caps, &opts);
        // The descriptor is not used after the scan; a close failure leaves nothing to retry.
        if let Err(e) = sys::close(root_fd) {
            log::debug!("close of the workdir fd failed: {e}");
        }
        self.compute_dirty(limits, index, &candidates, stats);
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
    fn index_tree_or_rebuild(&mut self, git_index: &GitIndex, include_untracked: bool) -> Index {
        let mut index_path = self.git.path().to_path_buf();
        index_path.push("index");
        let cur_stat = sys::stat(&index_path)
            .ok()
            .map(|st| (st.st_mtime, st.st_mtime_nsec, st.st_size));
        if let Some(tree) = self.index_tree.take() {
            // The untracked state in the cached tree was gathered under one value of
            // `include_untracked`. Reusing it under the other value reports every untracked
            // file as absent, or scans for nothing.
            if self.index_stat == cur_stat && tree.include_untracked == include_untracked {
                return tree;
            }
            let new_entries = copy_entries(git_index);
            let same_paths = tree.entries.len() == new_entries.len()
                && tree
                    .entries
                    .iter()
                    .zip(&new_entries)
                    .all(|(a, b)| a.path == b.path);
            if same_paths {
                let mut tree = tree;
                tree.update_stats(&new_entries);
                if tree.include_untracked != include_untracked {
                    // The cached untracked state belongs to the other setting.
                    // Drop it, so the scan below reads the dirs again instead of reporting
                    // the stale result.
                    tree.include_untracked = include_untracked;
                    for dir in &mut tree.dirs {
                        dir.st = None;
                        dir.unmatched.clear();
                    }
                }
                self.index_stat = cur_stat;
                return tree;
            }
        }
        let entries = copy_entries(git_index);
        let mut index = Index::from_entries(entries);
        index.include_untracked = include_untracked;
        index.init_splits(self.limits.num_threads);
        self.index_stat = cur_stat;
        index
    }

    /// Staged diff (StartStagedScan): HEAD tree vs index, foreach counting
    /// staged/conflicted/staged_new/staged_deleted, stopping early once the
    /// caps are reached (OnDelta's GIT_EUSER semantics).
    ///
    /// Difference from the original: it aborts early inside diff
    /// construction via notify_cb; git2 does not expose a notify callback, so
    /// the early stop happens in foreach after construction. The default caps
    /// are 1 (`-s`/`-c`), so the walk stops at the first delta, and the
    /// benchmark covers the construction cost of the full diff.
    fn compute_staged(&self, git_index: &GitIndex, head: Oid) -> StagedStats {
        let mut stats = StagedStats::default();
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
    fn compute_dirty(
        &self,
        limits: &Options,
        index: &Index,
        candidates: &[Vec<u8>],
        stats: &mut IndexStats,
    ) {
        if candidates.is_empty() {
            return;
        }
        let m_unstaged = limits.max_num_unstaged;
        let m_untracked = limits.max_num_untracked;
        for c in candidates {
            if c.last() == Some(&b'/') {
                // Directory candidate: count 1 only if it contains unignored
                // content (ENABLE_FAST_UNTRACKED_DIRS: empty/all-ignored
                // dirs are not reported).
                if m_untracked != 0 {
                    let dir_rel = &c[..c.len() - 1];
                    self.count_untracked_below(
                        dir_rel,
                        index,
                        stats,
                        limits.recurse_untracked_dirs,
                        m_untracked,
                    );
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

    /// Count the untracked entries below a directory candidate.
    ///
    /// Default (`-e` off): the directory counts as one, as long as it holds at least one
    /// unignored entry that is not in the index (`ENABLE_FAST_UNTRACKED_DIRS` in the original).
    /// With `-e/--recurse-untracked-dirs`: every unignored file below the directory counts,
    /// which is the original's `GIT_DIFF_RECURSE_UNTRACKED_DIRS` (`gitstatus/repo.cc:276`).
    ///
    /// Stops as soon as the cap is reached, so a large untracked tree cannot make the prompt
    /// scan it all when the count is capped anyway.
    fn count_untracked_below(
        &self,
        dir_rel: &[u8],
        index: &Index,
        stats: &mut IndexStats,
        recurse: bool,
        cap: i64,
    ) {
        let reached = |stats: &IndexStats| cap >= 0 && stats.num_untracked >= cap as usize;
        let mut dir_abs = self.workdir.clone();
        dir_abs.push(b'/');
        dir_abs.extend_from_slice(dir_rel);
        // A nested repository is one untracked entry for git. Do not descend into it.
        if has_git_dir(Path::new(OsStr::from_bytes(&dir_abs))) {
            let ignored = self
                .git
                .status_should_ignore(Path::new(OsStr::from_bytes(dir_rel)))
                .unwrap_or(false);
            if !ignored {
                stats.num_untracked += 1;
            }
            return;
        }
        let Ok(rd) = std::fs::read_dir(Path::new(OsStr::from_bytes(&dir_abs))) else {
            return;
        };
        for e in rd.flatten() {
            let mut rel = dir_rel.to_vec();
            rel.push(b'/');
            rel.extend_from_slice(e.file_name().as_bytes());
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !rel_in_index(index, &rel, is_dir) {
                // Untracked means unignored and not in the index; tracked files are unignored
                // too, so the index check above is what excludes them.
                let ignored = self
                    .git
                    .status_should_ignore(Path::new(OsStr::from_bytes(&rel)))
                    .unwrap_or(false);
                if !ignored {
                    if !recurse {
                        // FAST semantics: one unignored entry makes the directory count as one.
                        stats.num_untracked += 1;
                        return;
                    }
                    if !is_dir {
                        stats.num_untracked += 1;
                        if reached(stats) {
                            return;
                        }
                        continue;
                    }
                }
                if ignored {
                    // An ignored directory hides everything below it.
                    continue;
                }
            }
            // Directories are descended into whether or not the index knows them: a tracked
            // directory can still hold untracked files. A directory never counts by itself,
            // because git tracks files only.
            if recurse && is_dir {
                // git reports a nested repository as one untracked entry (`?? nested/`) and never
                // descends into it. A submodule is an index entry (gitlink), not an untracked
                // entry.
                if has_git_dir(&e.path()) {
                    if !in_index_exact(index, &rel) {
                        stats.num_untracked += 1;
                        if reached(stats) {
                            return;
                        }
                    }
                    continue;
                }
                self.count_untracked_below(&rel, index, stats, true, cap);
                if reached(stats) {
                    return;
                }
            }
        }
    }

    /// Repo capability bits (RepoCaps; config missing → defaults).
    fn repo_caps(&self) -> RepoCaps {
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
        sys::open_dir(Path::new(OsStr::from_bytes(&self.workdir))).ok()
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

/// One-line summary of the final index counters for the debug log.
fn index_stats_line(stats: &IndexStats) -> String {
    format!(
        "index stats: size={} staged={} unstaged={} conflicted={} untracked={}",
        stats.index_size,
        stats.num_staged,
        stats.num_unstaged,
        stats.num_conflicted,
        stats.num_untracked
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The debug line reports the counters it is given. The caller runs it after the dirty scan,
    /// so those values are final.
    #[test]
    fn index_stats_line_reports_the_counters() {
        let stats = IndexStats {
            index_size: 3,
            num_staged: 1,
            num_unstaged: 2,
            num_conflicted: 4,
            num_untracked: 5,
            ..Default::default()
        };
        let line = index_stats_line(&stats);
        assert!(line.contains("size=3"), "got {line}");
        assert!(line.contains("staged=1"), "got {line}");
        assert!(line.contains("unstaged=2"), "got {line}");
        assert!(line.contains("conflicted=4"), "got {line}");
        assert!(line.contains("untracked=5"), "got {line}");
    }
}
