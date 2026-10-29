//! Integration: the `.git/config` switches that zero the counts are per request, not permanent.
//!
//! `status.showUntrackedFiles` and `bash.showDirtyState` zero the corresponding caps inside
//! `get_index_stats`. The original keeps the `Limits` it gave the repo at construction and
//! restores them before every request. Writing into the repo's own copy instead makes the
//! zeroing permanent. Flipping the config back to `yes` then has no effect inside a resident
//! daemon.
//!
//! Two caches can hide the change as well. That is why the cases drive both directions in one
//! `RepoCache`.
//!
//! - The index-tree cache stores each directory's untracked state, gathered under one value of
//!   `include_untracked`. Reusing it under the other value reports nothing.
//! - The same cache uses `.git/index` as its only key. An unchanged index then keeps the tree
//!   built under the old setting.

use p11k_gitstatus::options::Options;
use p11k_gitstatus::protocol::field;
use p11k_gitstatus::repo::RepoCache;
use std::os::unix::ffi::OsStrExt;

fn run_git(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn num(cache: &mut RepoCache, dir: &[u8], fld: usize) -> usize {
    let repo = cache.get_or_open(dir, false).expect("repo should open");
    let f = repo.build_fields(false);
    String::from_utf8_lossy(&f[fld]).parse().unwrap_or(0)
}

fn init_repo(dir: &std::path::Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "t@t"]);
    run_git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("tracked.txt"), "v1").unwrap();
    run_git(dir, &["add", "tracked.txt"]);
    run_git(dir, &["commit", "-qm", "init"]);
}

#[test]
fn show_untracked_files_switch_is_per_request() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("untracked.txt"), "u").unwrap();

    let mut cache = RepoCache::new(&Options::default());
    let path = dir.path().as_os_str().as_bytes();

    // `git status --porcelain` lists the untracked file here. The switch suppresses it.
    run_git(dir.path(), &["config", "status.showUntrackedFiles", "no"]);
    assert_eq!(
        num(&mut cache, path, field::NUM_UNTRACKED),
        0,
        "showUntrackedFiles=no should zero the untracked count"
    );

    // Flipped back: a resident daemon must see the file again without restarting.
    run_git(dir.path(), &["config", "status.showUntrackedFiles", "yes"]);
    assert_eq!(
        num(&mut cache, path, field::NUM_UNTRACKED),
        1,
        "showUntrackedFiles=yes should count the untracked file again"
    );
}

#[test]
fn dirty_state_switch_is_per_request() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // This case needs no untracked file. The switch zeroes all three dirty caps, so the
    // observable effect lands on the staged and unstaged counters.
    std::fs::write(dir.path().join("tracked.txt"), "v2").unwrap();
    run_git(dir.path(), &["add", "tracked.txt"]);

    let mut cache = RepoCache::new(&Options::default());
    let path = dir.path().as_os_str().as_bytes();

    run_git(dir.path(), &["config", "bash.showDirtyState", "false"]);
    assert_eq!(
        num(&mut cache, path, field::NUM_STAGED),
        0,
        "showDirtyState=false should zero the staged count"
    );

    run_git(dir.path(), &["config", "bash.showDirtyState", "true"]);
    assert_eq!(
        num(&mut cache, path, field::NUM_STAGED),
        1,
        "showDirtyState=true should count the staged change again"
    );
}
