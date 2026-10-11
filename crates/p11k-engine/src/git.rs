//! The asynchronous git status worker: the main loop asks, the worker answers.

use std::sync::mpsc;

use p11k_gitstatus::{options::Options, protocol::field, repo::RepoCache};

use crate::config::Config;
use crate::theme::GitStatus;

pub struct GitRequest {
    pub generation: u64,
    pub cwd: String,
}

pub struct GitResult {
    pub generation: u64,
    pub status: Option<GitStatus>,
}

/// Compute the git status of the current directory with p11k-gitstatus (None when not a
/// repo).
///
/// `RepoCache` is reused within the process: repo handles are cached by gitdir and
/// `build_fields` reuses the staged-diff cache (while HEAD is unchanged) and libgit2's
/// internal caches, avoiding a full rescan on every prompt.
///
/// A repo whose workdir matches `disabled_workdir` is reported as if it did not exist
/// (p10k `VCS_DISABLED_WORKDIR_PATTERN`). The test comes before `build_fields` so an
/// ignored repo also skips the dirty scan.
pub fn git_status(cache: &mut RepoCache, cwd: &str, disabled_workdir: &str) -> Option<GitStatus> {
    let repo = cache.get_or_open(cwd.as_bytes(), false)?;
    let workdir = String::from_utf8_lossy(&repo.workdir).into_owned();
    if crate::render::workdir_is_disabled(disabled_workdir, &workdir) {
        return None;
    }
    let f = repo.build_fields(false);
    let index_size = parse_field(&f[field::INDEX_SIZE]);
    Some(GitStatus {
        branch: String::from_utf8_lossy(&f[field::LOCAL_BRANCH]).into_owned(),
        commit: String::from_utf8_lossy(&f[field::COMMIT]).into_owned(),
        staged: parse_field(&f[field::NUM_STAGED]),
        unstaged: parse_field(&f[field::NUM_UNSTAGED]),
        conflicted: parse_field(&f[field::NUM_CONFLICTED]),
        untracked: parse_field(&f[field::NUM_UNTRACKED]),
        ahead: parse_field(&f[field::COMMITS_AHEAD]),
        behind: parse_field(&f[field::COMMITS_BEHIND]),
        push_ahead: parse_field(&f[field::PUSH_COMMITS_AHEAD]),
        push_behind: parse_field(&f[field::PUSH_COMMITS_BEHIND]),
        stashes: parse_field(&f[field::STASHES]),
        action: String::from_utf8_lossy(&f[field::ACTION]).into_owned(),
        tag: String::from_utf8_lossy(&f[field::TAG]).into_owned(),
        remote_branch: String::from_utf8_lossy(&f[field::REMOTE_BRANCH]).into_owned(),
        commit_summary: String::from_utf8_lossy(&f[field::COMMIT_SUMMARY]).into_owned(),
        index_size,
        remote_url: String::from_utf8_lossy(&f[field::REMOTE_URL]).into_owned(),
    })
}

/// Start the git worker. It owns the `RepoCache`; the main loop talks to it over channels.
///
/// The call follows p10k conventions: the count caps and the dirty skip threshold come from
/// vcs segment properties (in p10k the global `POWERLEVEL9K_VCS_*_MAX_NUM` and
/// `POWERLEVEL9K_VCS_MAX_INDEX_SIZE_DIRTY`, -1 = unlimited). `p11k reload` respawns the
/// worker so a changed theme takes effect there too, at the cost of the warm repo cache.
pub fn spawn_git_worker(
    config: &Config,
    req_rx: mpsc::Receiver<GitRequest>,
    res_tx: mpsc::Sender<GitResult>,
) {
    let opts = Options {
        max_num_staged: vcs_int_prop(config, "max-num-staged", -1),
        max_num_unstaged: vcs_int_prop(config, "max-num-unstaged", -1),
        max_num_conflicted: vcs_int_prop(config, "max-num-conflicted", -1),
        max_num_untracked: vcs_int_prop(config, "max-num-untracked", -1),
        dirty_max_index_size: vcs_int_prop(config, "max-index-size-dirty", -1),
        num_threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(32),
        ..Default::default()
    };
    let disabled_workdir = vcs_str_prop(config, "disabled-workdir-pattern");
    std::thread::spawn(move || {
        let mut cache = RepoCache::new(&opts);
        while let Ok(req) = req_rx.recv() {
            // Drain the queue and handle only the latest request (drop backlogged old cwds).
            let mut latest = req;
            while let Ok(newer) = req_rx.try_recv() {
                latest = newer;
            }
            let status = git_status(&mut cache, &latest.cwd, &disabled_workdir);
            if res_tx
                .send(GitResult {
                    generation: latest.generation,
                    status,
                })
                .is_err()
            {
                break; // the main loop exited, channel closed
            }
        }
    });
}

/// Read an integer property on the `vcs` segment (p10k's global `POWERLEVEL9K_VCS_*`
/// parameters land on the vcs segment in KDL); use `default` when missing or invalid.
fn vcs_int_prop(config: &Config, key: &str, default: i64) -> i64 {
    match config.segment("vcs").prop(key) {
        Some(crate::config::Prop::Int(n)) => *n,
        _ => default,
    }
}

/// Read a string property on the `vcs` segment; an unset one is the empty string, which is
/// also p10k's default for these parameters.
fn vcs_str_prop(config: &Config, key: &str) -> String {
    match config.segment("vcs").prop(key) {
        Some(crate::config::Prop::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Fields are decimal strings after SafePrint; parse them as usize, treating failure as 0.
fn parse_field(b: &[u8]) -> usize {
    std::str::from_utf8(b)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
