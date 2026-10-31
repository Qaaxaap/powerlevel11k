//! Fixtures shared by the two benchmark harnesses.
//!
//! The repositories live under a fixed name in the temporary directory, so a benchmark run needs
//! no environment variable and no repository outside this one, and the previous run's directory
//! is reused instead of leaking. The previous benchmark skeleton was deleted because it depended
//! on both an environment variable and an uncommitted setup script.

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use p11k_gitstatus::options::Options;
use p11k_gitstatus::repo::RepoCache;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        // Fixed clock and branch name, so every run builds the same commit: the instruction
        // counts must not drift with the date or with the host's `init.defaultBranch`.
        .env("GIT_AUTHOR_DATE", "2020-01-01T00:00:00 +0000")
        .env("GIT_COMMITTER_DATE", "2020-01-01T00:00:00 +0000")
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A committed repository with `files` tracked files spread over 20 directories, optionally
/// carrying five modified files and five untracked ones.
fn fixture(dir: &Path, files: usize, dirty: bool) {
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "user.email", "bench@example.com"]);
    git(dir, &["config", "user.name", "bench"]);

    let mut tracked = Vec::with_capacity(files);
    for i in 0..files {
        let sub = dir.join(format!("d{}", i % 20));
        std::fs::create_dir_all(&sub).expect("subdir");
        let path = sub.join(format!("f{i}.rs"));
        std::fs::write(&path, format!("pub fn f{i}() {{}}\n")).expect("file");
        tracked.push(path);
    }
    git(dir, &["add", "."]);
    git(dir, &["commit", "-qm", "fixture"]);

    if dirty {
        for path in tracked.iter().take(5) {
            std::fs::write(path, "// modified\n").expect("modify");
        }
        for i in 0..5 {
            let path = dir.join(format!("new{i}.txt"));
            std::fs::write(path, "untracked\n").expect("untracked");
        }
    }
}

/// A repository at a fixed temporary path with a warm cache entry: the first call opened the repo
/// and scanned it, so the measured calls only pay for `build_fields` itself.
///
/// The directory is rebuilt from scratch on every run and stays on disk afterwards. Neither
/// deleting nor keeping it belongs in the measurement, and `remove_dir_all` on a 5000-file tree
/// would dominate one.
pub struct Prepared {
    pub cache: RepoCache,
    pub path: Vec<u8>,
}

impl Prepared {
    pub fn new(name: &str, files: usize, dirty: bool) -> Prepared {
        let root = std::env::temp_dir().join(format!("p11k-bench-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("bench directory");
        fixture(&root, files, dirty);

        let mut cache = RepoCache::new(&Options::default());
        let path = root.as_os_str().as_bytes().to_vec();
        let _ = cache
            .get_or_open(&path, false)
            .expect("repo opens")
            .build_fields(false);
        Prepared { cache, path }
    }
}
