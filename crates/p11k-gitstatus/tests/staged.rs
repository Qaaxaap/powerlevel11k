//! Integration: the staged count refreshes after `git add` (cache invalidation
//! when the index changes but HEAD does not).
//!
//! Reproduction path: commit while the daemon is resident → modify a file + git add
//! → query the same repo again. If the staged cache were invalidated by HEAD alone,
//! it would return a stale 0 (the original gitstatusd detects the new_index in
//! `git_index_read_ex` and clears `head_` as soon as the index changes).

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

#[test]
fn staged_refreshes_after_git_add_with_same_head() {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "user.email", "t@t"]);
    run_git(dir.path(), &["config", "user.name", "t"]);
    std::fs::write(dir.path().join("f.txt"), "v1").unwrap();
    run_git(dir.path(), &["add", "f.txt"]);
    run_git(dir.path(), &["commit", "-qm", "c1"]);

    let opts = Options::default();
    let mut cache = RepoCache::new(&opts);
    let dir_bytes: Vec<u8> = dir.path().as_os_str().as_bytes().to_vec();

    assert_eq!(num(&mut cache, &dir_bytes, field::NUM_STAGED), 0, "clean");
    assert_eq!(num(&mut cache, &dir_bytes, field::NUM_UNSTAGED), 0);

    // Modify + git add: the index changes, HEAD does not. Querying again from the same daemon instance must refresh.
    std::fs::write(dir.path().join("f.txt"), "v2").unwrap();
    run_git(dir.path(), &["add", "f.txt"]);

    assert_eq!(
        num(&mut cache, &dir_bytes, field::NUM_STAGED),
        1,
        "staged must refresh to 1 after git add"
    );

    // commit: HEAD moves. Querying staged again from the same daemon should reset to 0, and COMMIT should be the new commit.
    run_git(dir.path(), &["commit", "-qm", "c2"]);
    assert_eq!(
        num(&mut cache, &dir_bytes, field::NUM_STAGED),
        0,
        "staged must reset to 0 after commit"
    );
    let repo = cache
        .get_or_open(&dir_bytes, false)
        .expect("repo should open");
    let f = repo.build_fields(false);
    let commit = String::from_utf8_lossy(&f[field::COMMIT]);
    let head = run_git_out(dir.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        commit.trim(),
        head.trim(),
        "COMMIT field must refresh to the new HEAD after commit"
    );
}

fn run_git_out(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).into_owned()
}
