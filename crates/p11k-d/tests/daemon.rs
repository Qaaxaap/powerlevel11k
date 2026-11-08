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

/// Wait for the child, killing it after `limit`. A regression must fail the test instead of
/// hanging the job: cargo test has no per-test timeout.
fn wait_or_kill(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("try_wait p11k-d") {
            return status;
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let status = child.wait().expect("wait after kill");
            panic!("p11k-d did not exit within {limit:?} (status: {status:?})");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
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

/// A response that cannot be written because the shell closed the pipe stops the daemon
/// quietly: the stream went away with the shell, and the next shell starts a fresh daemon.
#[test]
fn a_closed_stdout_exits_quietly() {
    // The read end must be closed before the daemon starts. `Stdio::piped()` keeps the read
    // end open in this process, and another test spawning a child at the same moment forks a
    // short-lived copy of it: a write that lands in that window succeeds, the daemon then
    // waits for a request that never comes, and the test hangs. A pipe with no reader from the
    // start makes every write fail with EPIPE, whatever else the test process does.
    let (read_end, write_end) = std::io::pipe().expect("create the response pipe");
    drop(read_end);
    let mut child = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
        .stdin(Stdio::piped())
        .stdout(Stdio::from(write_end))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn p11k-d");
    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(b"}hello\x1f\x1e").expect("write request");
    stdin.flush().unwrap();
    let status = wait_or_kill(&mut child, Duration::from_secs(5));
    assert_eq!(
        status.code(),
        Some(0),
        "a closed response pipe means the shell exited"
    );
}

// ---- Boundary cases: malformed messages, half messages, options, non-repos ----

/// A malformed request is caught and logged; the daemon keeps serving the next request.
#[test]
fn malformed_request_keeps_the_daemon_serving() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    // Missing dir field: parse_request panics (the original's VERIFY) and the panic is caught.
    stdin.write_all(b"garbage\x1e").unwrap();
    // Too many fields: a second malformed shape.
    stdin.write_all(b"bad\x1f\x1f1\x1fextra\x1e").unwrap();
    // A valid non-repo request after the bad ones must still get an answer.
    stdin.write_all(b"ok\x1f\x1e").unwrap();
    stdin.flush().unwrap();
    let resp = read_exact(&mut stdout, 5);
    assert_eq!(
        resp, b"ok\x1f0\x1e",
        "the daemon must keep serving after a panic"
    );
    drop(stdin);
    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(0),
        "EOF after bad requests still exits 0"
    );
}

/// Half a message followed by EOF is a shell that went away: exit 0 without a panic.
#[test]
fn half_message_then_eof_exits_zero() {
    let (mut child, mut stdin, _stdout) = spawn(&[]);
    stdin.write_all(b"id\x1f/tmp").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let status = wait_or_kill(&mut child, Duration::from_secs(5));
    assert_eq!(status.code(), Some(0));
}

/// A request for a directory that does not exist is a non-repo answer, like an empty dir.
#[test]
fn nonexistent_dir_returns_non_repo() {
    let (mut child, mut stdin, mut stdout) = spawn(&[]);
    stdin.write_all(b"id\x1f/nonexistent-p11k-dir\x1e").unwrap();
    stdin.flush().unwrap();
    let resp = read_exact(&mut stdout, 5);
    assert_eq!(resp, b"id\x1f0\x1e");
    drop(stdin);
    child.wait().unwrap();
}

/// Option values: invalid values are exit 10; valid ones start the daemon and EOF exits 0.
#[test]
fn option_values_are_validated() {
    for bad in [["-t", "0"], ["-l", "notanint"], ["-r", "notanint"]] {
        let status = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
            .args(bad)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(10), "bad option {bad:?} must exit 10");
    }
    for good in [vec!["-t", "4"], vec!["-r", "-1"], vec!["-l", "3"]] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_p11k-d"))
            .args(&good)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(child.stdin.take());
        let status = wait_or_kill(&mut child, Duration::from_secs(5));
        assert_eq!(
            status.code(),
            Some(0),
            "valid option {good:?} must exit 0 on EOF"
        );
    }
}
