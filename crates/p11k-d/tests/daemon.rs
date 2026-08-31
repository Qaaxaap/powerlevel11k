//! daemon 进程级集成测试：拉起真实 p11k-d 二进制，验证协议链路。
//!
//! 对拍基准：官方 gitstatusd v1.5.4 的实际行为（可手工用
//! `~/.cache/gitstatus/gitstatusd-linux-x86_64` 验证同样场景）。

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// 启动 p11k-d，返回子进程与 stdin/stdout。
fn spawn(args: &[&str]) -> (Child, ChildStdin, ChildStdout) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn p11k-d");
    let stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    (child, stdin, stdout)
}

/// 读 stdout 直到读完 n 字节（响应长度已知，逐字节读避免块等待）。
fn read_exact(stdout: &mut ChildStdout, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    stdout.read_exact(&mut out).expect("read response");
    out
}

/// 握手：id=}hello、dir 为空 → 非仓库响应 }hello\x1f0\x1e。
#[test]
fn handshake_returns_not_a_repo() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    stdin.write_all(b"}hello\x1f\x1e").unwrap();
    stdin.flush().unwrap();
    let resp = read_exact(&mut stdout, 9);
    assert_eq!(resp, b"}hello\x1f0\x1e");
    drop(stdin); // EOF → daemon 退出
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// 批量消息：一次写入多条请求，响应按序返回。
/// 空 dir 请求（含任意 id）都回非仓库——对齐原版"找不到仓库"语义。
#[test]
fn batched_requests_are_answered_in_order() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    // 三条空 dir 请求（响应长度 6/9/7 字节）
    stdin
        .write_all(b"}h1\x1f\x1e}hello\x1f\x1e}xyz\x1f\x1e")
        .unwrap();
    stdin.flush().unwrap();
    let mut buf = [0u8; 22];
    stdout.read_exact(&mut buf).unwrap();
    assert_eq!(&buf[0..6], b"}h1\x1f0\x1e");
    assert_eq!(&buf[6..15], b"}hello\x1f0\x1e");
    assert_eq!(&buf[15..22], b"}xyz\x1f0\x1e");
    drop(stdin);
    child.wait().unwrap();
}

/// EOF：关闭 stdin 后 daemon 正常退出（exit 0）。
#[test]
fn eof_exits_zero() {
    let (mut child, stdin, _stdout) = spawn(&[]);
    drop(stdin);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// 探活：-p 指向不存在的 PID，约 1~2s 内退出且退出码 0（对齐原版）。
#[test]
fn parent_pid_liveness_exits_zero() {
    let (mut child, _stdin, _stdout) = spawn(&["-p", "999999999"]);
    let start = Instant::now();
    let status = child.wait().unwrap();
    let elapsed = start.elapsed();
    assert_eq!(status.code(), Some(0));
    assert!(
        elapsed >= Duration::from_millis(900) && elapsed < Duration::from_secs(5),
        "elapsed: {elapsed:?}"
    );
}

/// -G 版本不匹配 → 退出码 11。
#[test]
fn version_mismatch_exits_eleven() {
    let status = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .args(["-G", "v0.*"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(11));
}

/// 坏参数 → 退出码 10。
#[test]
fn bad_args_exits_ten() {
    let status = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .args(["-x"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(10));
}

/// 真实仓库端到端：建临时 git 仓库（1 提交 + 1 untracked 文件），
/// 发请求断言 29 字段响应、workdir/commit/分支名/untracked 计数。
#[test]
fn real_repo_request_returns_status() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap()
    };
    run(&["init", "-q", "-b", "main"]);
    run(&["config", "user.email", "t@t"]);
    run(&["config", "user.name", "t"]);
    std::fs::write(repo.join("f"), b"x").unwrap();
    run(&["add", "f"]);
    run(&["commit", "-qm", "init"]);
    std::fs::write(repo.join("u"), b"y").unwrap(); // untracked

    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    let req = format!("id\x1f{}\x1e", repo.to_str().unwrap());
    stdin.write_all(req.as_bytes()).unwrap();
    stdin.flush().unwrap();
    // 响应长度不定：逐字节读到 MSG_SEP
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.last() != Some(&0x1e) {
        stdout.read_exact(&mut byte).unwrap();
        buf.push(byte[0]);
    }
    let fields: Vec<&[u8]> = buf.split(|&b| b == 0x1f).collect();
    // id + 1 + 27 数据字段
    assert_eq!(fields.len(), 29, "fields: {fields:?}");
    assert_eq!(fields[0], b"id");
    assert_eq!(fields[1], b"1");
    assert_eq!(fields[2], repo.to_str().unwrap().as_bytes()); // workdir
    assert_eq!(fields[3].len(), 40); // commit sha
    assert_eq!(fields[4], b"main"); // local branch
    // 数据字段索引 = 协议 field 常量 + 2（id、1 两个前缀字段）
    assert_eq!(fields[13], b"1"); // num_untracked（u）
    assert_eq!(fields[11], b"0"); // num_unstaged（干净）
    drop(stdin);
    child.wait().unwrap();
}
