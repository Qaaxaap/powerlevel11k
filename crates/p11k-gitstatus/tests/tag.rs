//! Integration: the `tag` field reports the tag that points at HEAD.
//!
//! The original reports the tag name for both lightweight and annotated tags. A lightweight tag
//! points straight at the commit; an annotated tag points at a tag object, which the status code
//! has to peel before comparing with HEAD.

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

fn init_repo(dir: &std::path::Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "t@t"]);
    run_git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("tracked.txt"), "v1").unwrap();
    run_git(dir, &["add", "tracked.txt"]);
    run_git(dir, &["commit", "-qm", "init"]);
}

fn tag(cache: &mut RepoCache, dir: &std::path::Path) -> String {
    let path = dir.as_os_str().as_bytes();
    let repo = cache.get_or_open(path, false).expect("repo should open");
    let fields = repo.build_fields(false);
    String::from_utf8_lossy(&fields[field::TAG]).into_owned()
}

#[test]
fn lightweight_tag_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run_git(dir.path(), &["tag", "v1"]);

    let mut cache = RepoCache::new(&Options::default());
    assert_eq!(tag(&mut cache, dir.path()), "v1");
}

#[test]
fn annotated_tag_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    run_git(dir.path(), &["tag", "-a", "v1", "-m", "release"]);

    let mut cache = RepoCache::new(&Options::default());
    assert_eq!(tag(&mut cache, dir.path()), "v1");
}
