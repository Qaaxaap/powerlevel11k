//! Benchmark `RepoCache::build_fields`, the call a prompt makes on every precmd.
//!
//! The fixtures are built here in temporary directories, so `cargo bench` needs no environment
//! variable and no repository outside this one. Three states cover the dirty scan: an untouched
//! repository, one with local changes, and a large index.
//!
//! To compare a change:
//!
//! ```text
//! cargo bench -p p11k-gitstatus -- --save-baseline before
//! # make the change
//! cargo bench -p p11k-gitstatus -- --baseline before
//! ```

use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::Command;

use criterion::{Criterion, criterion_group, criterion_main};
use p11k_gitstatus::options::Options;
use p11k_gitstatus::repo::RepoCache;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
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
fn fixture(files: usize, dirty: bool) -> TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["config", "user.email", "bench@example.com"]);
    git(dir.path(), &["config", "user.name", "bench"]);

    let mut tracked = Vec::with_capacity(files);
    for i in 0..files {
        let sub = dir.path().join(format!("d{}", i % 20));
        std::fs::create_dir_all(&sub).expect("subdir");
        let path = sub.join(format!("f{i}.rs"));
        std::fs::write(&path, format!("pub fn f{i}() {{}}\n")).expect("file");
        tracked.push(path);
    }
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "fixture"]);

    if dirty {
        for path in tracked.iter().take(5) {
            std::fs::write(path, "// modified\n").expect("modify");
        }
        for i in 0..5 {
            let path = dir.path().join(format!("new{i}.txt"));
            std::fs::write(path, "untracked\n").expect("untracked");
        }
    }
    dir
}

/// One `build_fields` call on a warm cache entry, which is what a prompt does.
fn bench_state(c: &mut Criterion, name: &str, files: usize, dirty: bool) {
    let dir = fixture(files, dirty);
    let mut cache = RepoCache::new(&Options::default());
    let path = dir.path().as_os_str().as_bytes().to_vec();

    // Open and scan once, so the measured iterations run against a warm cache entry.
    let _ = cache
        .get_or_open(&path, false)
        .expect("repo opens")
        .build_fields(false);

    c.bench_function(name, |b| {
        b.iter(|| {
            let repo = cache.get_or_open(&path, false).expect("repo opens");
            std::hint::black_box(repo.build_fields(false))
        })
    });
}

fn build_fields(c: &mut Criterion) {
    bench_state(c, "build_fields/clean-small", 50, false);
    bench_state(c, "build_fields/dirty-small", 50, true);
    bench_state(c, "build_fields/clean-large", 5000, false);
}

criterion_group!(benches, build_fields);
criterion_main!(benches);
