//! 集成：`git add` 后 staged 计数刷新（index 变但 HEAD 不变的缓存失效）。
//!
//! 复现路径：daemon 常驻期间 commit → 改文件 + git add → 同 repo 再查。
//! staged 缓存若只按 HEAD 失效，会返回旧的 0（原版 gitstatusd 用
//! `git_index_read_ex` 的 new_index 检测，index 变即清 `head_`）。

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
    let repo = cache.get_or_open(dir, false).expect("repo 应可打开");
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

    assert_eq!(num(&mut cache, &dir_bytes, field::NUM_STAGED), 0, "干净");
    assert_eq!(num(&mut cache, &dir_bytes, field::NUM_UNSTAGED), 0);

    // 修改 + git add：index 变、HEAD 不变。同一 daemon 实例再查必须刷新。
    std::fs::write(dir.path().join("f.txt"), "v2").unwrap();
    run_git(dir.path(), &["add", "f.txt"]);

    assert_eq!(
        num(&mut cache, &dir_bytes, field::NUM_STAGED),
        1,
        "git add 后 staged 应刷新为 1"
    );

    // commit：HEAD 移动。同 daemon 再查 staged 应清 0，COMMIT 字段应为新 commit。
    run_git(dir.path(), &["commit", "-qm", "c2"]);
    assert_eq!(
        num(&mut cache, &dir_bytes, field::NUM_STAGED),
        0,
        "commit 后 staged 应清 0"
    );
    let repo = cache.get_or_open(&dir_bytes, false).expect("repo 应可打开");
    let f = repo.build_fields(false);
    let commit = String::from_utf8_lossy(&f[field::COMMIT]);
    let head = run_git_out(dir.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        commit.trim(),
        head.trim(),
        "COMMIT 字段应刷新为新 HEAD（commit 后）"
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
