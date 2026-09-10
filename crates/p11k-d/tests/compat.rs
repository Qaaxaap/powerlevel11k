//! 与原版 gitstatusd 的对拍（differential）测试。
//!
//! # 前置条件
//!
//! 环境变量 `GITSTATUSD_PATH` 指向原版 C++ gitstatusd 二进制。
//! 未设置时本文件的测试整体跳过（`--ignored` 标记，CI 默认不跑）。
//! 官方预编译二进制（如 `~/.cache/gitstatus/gitstatusd-linux-x86_64`）
//! 的 SafePrint 保留 >0x7F 字节，与 p11k 一致（见协议模块文档）。
//!
//! # 方法
//!
//! 对同一仓库状态向两个 daemon 喂完全相同的请求字节流，
//! 逐字节比较响应。协议要求完全一致，无豁免字段。

use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// 原版二进制路径（GITSTATUSD_PATH）。
fn gitstatusd() -> String {
    std::env::var("GITSTATUSD_PATH").expect("GITSTATUSD_PATH not set")
}

/// 对某个二进制发一段请求，返回全部响应字节。
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
    drop(stdin); // EOF → daemon 正常退出
    let mut out = Vec::new();
    child.stdout.take().unwrap().read_to_end(&mut out).unwrap();
    child.wait().unwrap();
    out
}

/// 对拍：请求字节喂给两端，响应逐字节一致。
fn assert_identical(orig_req: &[u8], p11k_req: &[u8], ctx: &str) {
    let a = ask(&gitstatusd(), orig_req);
    let b = ask(env!("CARGO_BIN_EXE_p11k-d"), p11k_req);
    assert_eq!(a, b, "对拍不一致：{ctx}\n原版: {a:?}\np11k: {b:?}");
}

/// 用 git2 建一个状态确定的仓库：2 提交、tag、stash、untracked 文件、
/// 已修改与已删除的文件。返回仓库路径。
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

    // 第二次提交（供 ahead/behind 与 stash 使用）
    std::fs::write(dir.join("a"), b"1\n2").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(std::path::Path::new("a")).unwrap();
    let tree_id = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_id).unwrap();
    let parent = repo.head().unwrap().peel_to_commit().unwrap();
    repo.commit(Some("HEAD"), &sig, &sig, "second", &tree, &[&parent])
        .unwrap();

    // 工作区状态：a 已修改、b 已删除、d untracked
    std::fs::write(dir.join("a"), b"1\n2\n3").unwrap();
    std::fs::remove_file(dir.join("b")).unwrap();
    std::fs::write(dir.join("d"), b"4").unwrap();

    dir.to_str().unwrap().to_string()
}

/// 对拍：多状态仓库的完整请求（无 diff 字段 + diff='1' 跳过 index）。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_multi_state_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = make_repo(tmp.path());
    // 无 diff 字段（全算）
    assert_identical(
        format!("id1\x1f{repo}\x1e").as_bytes(),
        format!("id1\x1f{repo}\x1e").as_bytes(),
        "multi-state, full diff",
    );
    // diff='1'（跳过 index）
    assert_identical(
        format!("id2\x1f{repo}\x1f1\x1e").as_bytes(),
        format!("id2\x1f{repo}\x1f1\x1e").as_bytes(),
        "multi-state, skip index",
    );
}

/// 对拍：握手与非仓库目录。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
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

/// 对拍：GIT_DIR 前缀（from_dotgit）与空仓库。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_gitdir_and_empty_repo() {
    // 空仓库（无提交）
    let tmp = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(tmp.path()).unwrap();
    let gitdir = repo.path().to_str().unwrap();
    assert_identical(
        format!("id\x1f:{gitdir}\x1e").as_bytes(),
        format!("id\x1f:{gitdir}\x1e").as_bytes(),
        "empty repo via GIT_DIR",
    );
    // GIT_DIR 请求带 diff 标志
    assert_identical(
        format!("id\x1f:{gitdir}\x1f1\x1e").as_bytes(),
        format!("id\x1f:{gitdir}\x1f1\x1e").as_bytes(),
        "empty repo via GIT_DIR, skip index",
    );
}

/// 对拍：upstream 相关的三种边界（原版在 remote 解析不出 URL、或
/// upstream ref 不存在时，remote_branch/name/url 三个字段全空）。
///
/// 手动探测原版时发现的差异：p11k 之前只看 `branch.<n>.remote` 配置就上报，
/// 于是"remote 未配置""url 为空""tracking ref 不存在"三种情况下都会多报。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
fn differential_upstream_edge_cases() {
    /// 造一个本地分支 local-name，tracking refs/heads/tracking-test。
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
        // 建 remote-tracking ref 并把当前分支挪到 local-name / 指向它。
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

    // (1) 没有 remote 配置：ref 在，但 remote 解析不出来。
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

    // (2) remote 在但 url 是空串（libgit2 不认 `remote("origin", "")`，
    //     所以先建好再直接把配置值改成空串）。原版照常上报 branch/name，
    //     url 字段留空——空 url 不是判据。
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

    // (3) 判据是 fetch refspec：只有 url、没有 remote.<name>.fetch 时，
    //     原版把 remote 当作不存在（三个字段全空）。
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

    // (4) remote 与 url 都正常，但 tracking ref 被删掉。
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

/// 对拍：EOF 退出码（两端都应 exit 0）。
#[test]
#[ignore = "需要 GITSTATUSD_PATH 指向原版 gitstatusd"]
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
