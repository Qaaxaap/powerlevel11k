//! Differential tests against the original gitstatusd.
//!
//! # Prerequisites
//!
//! The `GITSTATUSD_PATH` environment variable points at the original C++ gitstatusd
//! binary. When unset, every test in this file is skipped (`--ignored` flag, not run
//! by CI by default). The official prebuilt binary (e.g.
//! `~/.cache/gitstatus/gitstatusd-linux-x86_64`) keeps bytes >0x7F in SafePrint,
//! matching p11k (see the protocol module docs).
//!
//! # Method
//!
//! Feed byte-identical request streams for the same repo state to both daemons, then
//! compare the responses byte for byte. The protocol requires an exact match, with no
//! exempt fields.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn gitstatusd() -> String {
    std::env::var("GITSTATUSD_PATH").expect("GITSTATUSD_PATH not set")
}

fn ask(bin: &str, req: &[u8]) -> Vec<u8> {
    let mut child = Command::new(bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn daemon");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(req).unwrap();
    stdin.flush().unwrap();
    drop(stdin); // EOF → the daemon exits normally
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();
    out
}

/// Differential: feed the request bytes to both binaries and require byte-identical responses.
fn assert_identical(orig_req: &[u8], p11k_req: &[u8], ctx: &str) {
    let a = ask(&gitstatusd(), orig_req);
    let b = ask(env!("CARGO_BIN_EXE_p11k-d"), p11k_req);
    assert_eq!(a, b, "mismatch for {ctx}\noriginal: {a:?}\np11k: {b:?}");
}

/// Build a repo with deterministic state using git2: 2 commits, a tag, an untracked file,
/// and modified and deleted files. Returns the repo path.
fn make_repo(dir: &std::path::Path) -> String {
    let repo = git2::Repository::init(dir).unwrap();
    let mut cfg = repo.config().unwrap();
    cfg.set_str("user.email", "t@t").unwrap();
    cfg.set_str("user.name", "t").unwrap();
    let sig = git2::Signature::now("t", "t@t").unwrap();

    std::fs::write(dir.join("a"), b"1").unwrap();
    std::fs::write(dir.join("b"), b"2").unwrap();
    std::fs::write(dir.join("c"), b"3").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("a")).unwrap();
    index.add_path(std::path::Path::new("b")).unwrap();
    index.add_path(std::path::Path::new("c")).unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
        .unwrap();
    repo.tag(
        "v1.0",
        repo.head().unwrap().peel_to_commit().unwrap().as_object(),
        &sig,
        "tag",
        false,
    )
    .unwrap();

    // Second commit (so HEAD and the tag v1.0 point at different commits)
    std::fs::write(dir.join("a"), b"1\n2").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("a")).unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
        .unwrap();

    // Worktree state: a modified, b deleted, d untracked
    std::fs::write(dir.join("a"), b"1\n2\n3").unwrap();
    std::fs::remove_file(dir.join("b")).unwrap();
    std::fs::write(dir.join("d"), b"4").unwrap();

    dir.to_str().unwrap().to_string()
}

/// Differential: full requests against a multi-state repo (no diff field + diff='1' skipping index).
#[test]
#[ignore = "needs GITSTATUSD_PATH pointing at the original gitstatusd"]
fn differential_multi_state_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = make_repo(tmp.path());
    // No diff field (performs the full index comparison)
    assert_identical(
        format!("id1\x1f{repo}\x1e").as_bytes(),
        format!("id1\x1f{repo}\x1e").as_bytes(),
        "multi-state, full diff",
    );
    // diff='1' (skips the index)
    assert_identical(
        format!("id2\x1f{repo}\x1f1\x1e").as_bytes(),
        format!("id2\x1f{repo}\x1f1\x1e").as_bytes(),
        "multi-state, skip index",
    );
}

/// Differential: handshake and a non-repo directory.
#[test]
#[ignore = "needs GITSTATUSD_PATH pointing at the original gitstatusd"]
fn differential_hello_and_not_a_repo() {
    assert_identical(b"}hello\x1f\x1e", b"}hello\x1f\x1e", "handshake");
    let tmp = tempfile::tempdir().unwrap();
    let not_repo = tmp.path().to_str().unwrap();
    assert_identical(
        format!("id\x1f{not_repo}\x1e").as_bytes(),
        format!("id\x1f{not_repo}\x1e").as_bytes(),
        "not a repo",
    );
}

/// Differential: GIT_DIR prefix (from_dotgit) and an empty repo.
#[test]
#[ignore = "needs GITSTATUSD_PATH pointing at the original gitstatusd"]
fn differential_gitdir_and_empty_repo() {
    // Empty repo (no commits)
    let tmp = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    let gitdir = repo.path().to_str().unwrap();
    assert_identical(
        format!("id\x1f:{gitdir}\x1e").as_bytes(),
        format!("id\x1f:{gitdir}\x1e").as_bytes(),
        "empty repo via GIT_DIR",
    );
    // GIT_DIR request with the diff flag
    assert_identical(
        format!("id\x1f:{gitdir}\x1f1\x1e").as_bytes(),
        format!("id\x1f:{gitdir}\x1f1\x1e").as_bytes(),
        "empty repo via GIT_DIR, skip index",
    );
}

/// Differential: four upstream edge cases (the original leaves remote_branch/name/url
/// all empty when the remote cannot be resolved — unconfigured or lacking a fetch
/// refspec — or the upstream ref is missing).
///
/// A difference found while probing the original by hand: p11k used to report from
/// `branch.<n>.remote` alone, so a missing remote config, a missing fetch refspec, and a
/// missing tracking ref all misreported.
#[test]
#[ignore = "needs GITSTATUSD_PATH pointing at the original gitstatusd"]
fn differential_upstream_edge_cases() {
    /// Create a local branch local-name tracking refs/heads/tracking-test.
    fn make_tracking_repo(dir: &std::path::Path) -> git2::Repository {
        let repo = git2::Repository::init(dir).unwrap();
        let mut cfg = repo.config().unwrap();
        cfg.set_str("user.email", "t@t").unwrap();
        cfg.set_str("user.name", "t").unwrap();
        let sig = git2::Signature::now("t", "t@t").unwrap();
        std::fs::write(dir.join("a"), b"1").unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("a")).unwrap();
        let tree_id = index.write_tree().unwrap();
        {
            let tree = repo.find_tree(tree_id).unwrap();
            repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
                .unwrap();
        }
        let head_id = repo.head().unwrap().peel_to_commit().unwrap().id();
        // Create the remote-tracking ref and switch the current branch to local-name pointing at the same commit.
        repo.reference("refs/remotes/origin/tracking-test", head_id, true, "test")
            .unwrap();
        {
            let head = repo.find_commit(head_id).unwrap();
            repo.branch("local-name", &head, true).unwrap();
        }
        repo.set_head("refs/heads/local-name").unwrap();
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .unwrap();
        repo
    }

    // (1) No remote configured: the ref exists, but the remote cannot be resolved.
    let tmp = tempfile::tempdir().unwrap();
    let repo = make_tracking_repo(tmp.path());
    let path = tmp.path().to_str().unwrap().to_string();
    repo.config()
        .unwrap()
        .set_str("branch.local-name.remote", "origin")
        .unwrap();
    repo.config()
        .unwrap()
        .set_str("branch.local-name.merge", "refs/heads/tracking-test")
        .unwrap();
    assert_identical(
        format!("id\x1f{path}\x1e").as_bytes(),
        format!("id\x1f{path}\x1e").as_bytes(),
        "tracking branch without a configured remote",
    );

    // (2) A remote exists but its url is the empty string (libgit2 rejects `remote("origin", "")`,
    //     so create the remote first and then set the config value to empty). The original still
    //     reports branch/name and leaves the url field empty — an empty url is not the criterion.
    repo.remote("origin", "https://example.invalid/repo.git")
        .unwrap();
    repo.config()
        .unwrap()
        .set_str("remote.origin.url", "")
        .unwrap();
    assert_identical(
        format!("id\x1f{path}\x1e").as_bytes(),
        format!("id\x1f{path}\x1e").as_bytes(),
        "remote with an empty url",
    );

    // (3) The criterion is the fetch refspec: with a url but no remote.<name>.fetch, the
    //     original treats the remote as absent (all three fields empty).
    repo.remote_set_url("origin", "https://example.invalid/repo.git")
        .unwrap();
    repo.config()
        .unwrap()
        .remove("remote.origin.fetch")
        .unwrap();
    assert_identical(
        format!("id\x1f{path}\x1e").as_bytes(),
        format!("id\x1f{path}\x1e").as_bytes(),
        "remote with a url but no fetch refspec",
    );

    // (4) The remote and url are both fine, but the tracking ref is deleted.
    repo.config()
        .unwrap()
        .set_str("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        .unwrap();
    assert_identical(
        format!("id\x1f{path}\x1e").as_bytes(),
        format!("id\x1f{path}\x1e").as_bytes(),
        "remote configured, tracking ref present",
    );
    repo.find_reference("refs/remotes/origin/tracking-test")
        .unwrap()
        .delete()
        .unwrap();
    assert_identical(
        format!("id\x1f{path}\x1e").as_bytes(),
        format!("id\x1f{path}\x1e").as_bytes(),
        "remote configured, tracking ref deleted",
    );
}

/// Differential: EOF exit code (both should exit 0).
#[test]
#[ignore = "needs GITSTATUSD_PATH pointing at the original gitstatusd"]
fn differential_eof_exit_code() {
    for bin in [gitstatusd(), env!("CARGO_BIN_EXE_p11k-d").to_string()] {
        let mut child = Command::new(bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(child.stdin.take());
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(0));
    }
}
