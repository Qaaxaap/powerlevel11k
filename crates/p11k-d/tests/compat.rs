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
