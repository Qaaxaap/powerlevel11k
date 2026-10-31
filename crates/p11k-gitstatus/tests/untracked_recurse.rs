//! Integration: `-e/--recurse-untracked-dirs` changes how a directory candidate is counted.
//!
//! Default (`ENABLE_FAST_UNTRACKED_DIRS` in the original): a directory that holds any unignored
//! entry not in the index counts as **one** untracked item, because the prompt only needs the
//! count and scanning the whole tree would cost more than it buys.
//!
//! With `-e` the original sets `GIT_DIFF_RECURSE_UNTRACKED_DIRS` (`gitstatus/repo.cc:276`) and
//! every such entry counts. `p11k-d` parsed the flag but never read it, so both settings returned
//! the fast count.

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

fn untracked(cache: &mut RepoCache, dir: &[u8]) -> usize {
    let repo = cache.get_or_open(dir, false).expect("repo should open");
    let f = repo.build_fields(false);
    String::from_utf8_lossy(&f[field::NUM_UNTRACKED])
        .parse()
        .unwrap_or(0)
}

fn init_repo(dir: &std::path::Path) {
    run_git(dir, &["init", "-q"]);
    run_git(dir, &["config", "user.email", "t@t"]);
    run_git(dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("tracked.txt"), "v1").unwrap();
    run_git(dir, &["add", "tracked.txt"]);
    run_git(dir, &["commit", "-qm", "init"]);
}

/// Three untracked files inside one untracked directory: `new/a.txt`, `new/b.txt`, `new/c.txt`.
fn untracked_tree(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("new")).unwrap();
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(dir.join("new").join(name), "u").unwrap();
    }
}

/// Two untracked files two levels down: `new/inner/a.txt`, `new/inner/b.txt`.
fn nested_untracked_tree(dir: &std::path::Path) {
    let inner = dir.join("new").join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    for name in ["a.txt", "b.txt"] {
        std::fs::write(inner.join(name), "u").unwrap();
    }
}

#[test]
fn fast_mode_counts_an_untracked_directory_as_one() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    untracked_tree(dir.path());

    let mut cache = RepoCache::new(&Options::default());
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        1,
        "without `-e` a directory of untracked files counts as one"
    );
}

#[test]
fn recurse_mode_counts_every_untracked_entry() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    untracked_tree(dir.path());

    // The default cap is 1 (`VCS_MAX_NUM_UNTRACKED`), which would stop the walk after the first
    // entry and hide the difference; a real user of `-e` raises it.
    let opts = Options {
        recurse_untracked_dirs: true,
        max_num_untracked: -1,
        ..Default::default()
    };
    let mut cache = RepoCache::new(&opts);
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        3,
        "with `-e` every untracked entry counts"
    );
}

#[test]
fn recurse_mode_stops_at_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    untracked_tree(dir.path());

    let opts = Options {
        recurse_untracked_dirs: true,
        max_num_untracked: 2,
        ..Default::default()
    };
    let mut cache = RepoCache::new(&opts);
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        2,
        "the cap ends the walk instead of counting past it"
    );
}

#[test]
fn recurse_mode_descends_into_nested_directories() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    nested_untracked_tree(dir.path());

    let mut cache = RepoCache::new(&Options::default());
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        1,
        "without `-e` the whole tree still counts as one"
    );

    let opts = Options {
        recurse_untracked_dirs: true,
        max_num_untracked: -1,
        ..Default::default()
    };
    let mut cache = RepoCache::new(&opts);
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        2,
        "with `-e` the walk reaches files below a nested directory"
    );
}

#[test]
fn recurse_mode_skips_ignored_files_below_the_directory() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let new = dir.path().join("new");
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(new.join(".gitignore"), "*.log\n").unwrap();
    std::fs::write(new.join("kept.txt"), "u").unwrap();
    std::fs::write(new.join("dropped.log"), "u").unwrap();

    let opts = Options {
        recurse_untracked_dirs: true,
        max_num_untracked: -1,
        ..Default::default()
    };
    let mut cache = RepoCache::new(&opts);
    assert_eq!(
        untracked(&mut cache, dir.path().as_os_str().as_bytes()),
        2,
        "the nested `.gitignore` and the kept file count, the ignored file does not"
    );
}
