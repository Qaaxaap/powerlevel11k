//! Process-level daemon integration tests: launch the real p11k-d binary and verify
//! the protocol path.
//!
//! Reference baseline: the actual behavior of official gitstatusd v1.5.4 (the same
//! scenarios can be verified by hand with `~/.cache/gitstatus/gitstatusd-linux-x86_64`).

use std::io::{Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

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

/// Read stdout until n bytes have arrived (the response length is known; read byte by byte to avoid blocking).
fn read_exact(stdout: &mut ChildStdout, n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    stdout.read_exact(&mut out).expect("read response");
    out
}

/// Handshake: id=}hello, empty dir → non-repo response }hello\x1f0\x1e.
#[test]
fn handshake_returns_not_a_repo() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    stdin.write_all(b"}hello\x1f\x1e").unwrap();
    stdin.flush().unwrap();
    let resp = read_exact(&mut stdout, 9);
    assert_eq!(resp, b"}hello\x1f0\x1e");
    drop(stdin); // EOF → the daemon exits
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// Batched messages: write several requests at once, responses come back in order.
/// Requests with an empty dir (any id) all answer non-repo — mirroring the original's "no repo found" semantics.
#[test]
fn batched_requests_are_answered_in_order() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    // Three empty-dir requests (response lengths 6/9/7 bytes)
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

#[test]
fn eof_exits_zero() {
    let (mut child, stdin, _stdout) = spawn(&[]);
    drop(stdin);
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
}

/// Liveness: -p points at a nonexistent PID, exits within ~1-2s with code 0 (mirrors the original).
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

/// -G version mismatch → exit code 11.
#[test]
fn version_mismatch_exits_eleven() {
    let status = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .args(["-G", "v0.*"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(11));
}

/// Bad arguments → exit code 10.
#[test]
fn bad_args_exits_ten() {
    let status = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .args(["-x"])
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(10));
}

/// Real-repo end to end: create a temporary git repo (1 commit + 1 untracked file),
/// send a request and assert the 29-field response, workdir/commit/branch name/untracked counts.
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
    // Response length is unknown: read byte by byte until MSG_SEP
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while buf.last() != Some(&0x1e) {
        stdout.read_exact(&mut byte).unwrap();
        buf.push(byte[0]);
    }
    let fields: Vec<&[u8]> = buf.split(|&b| b == 0x1f).collect();
    // id + 1 + 27 data fields
    assert_eq!(fields.len(), 29, "fields: {fields:?}");
    assert_eq!(fields[0], b"id");
    assert_eq!(fields[1], b"1");
    assert_eq!(fields[2], repo.to_str().unwrap().as_bytes()); // workdir
    assert_eq!(fields[3].len(), 40); // commit sha
    assert_eq!(fields[4], b"main"); // local branch
    // Data field index = protocol field constant + 2 (the id and 1 prefix fields)
    assert_eq!(fields[13], b"1"); // num_untracked (u)
    assert_eq!(fields[11], b"0"); // num_unstaged (clean)
    drop(stdin);
    child.wait().unwrap();
}
