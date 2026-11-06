//! Integration: a malformed request must not kill the resident daemon.
//!
//! `parse_request` panics on a malformed message, mirroring the original's VERIFY. The original
//! catches the exception, logs it and keeps serving. The daemon used to call `parse_request`
//! outside `catch_unwind`, so one malformed message ended the process.

use p11k_gitstatus::daemon::Daemon;
use p11k_gitstatus::options::Options;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::thread;

#[test]
fn a_malformed_request_does_not_kill_the_daemon() {
    // Two socket pairs stand in for the FIFO/pipe pair the shell gives the daemon: one for
    // requests, one for responses. The server ends must stay open for the whole test, because a
    // closed fd would look like EOF and the daemon exits on EOF.
    let (mut client_in, server_in) = UnixStream::pair().unwrap();
    let (server_out, mut client_out) = UnixStream::pair().unwrap();

    let mut daemon = Daemon::new(Options::default());
    daemon.stdin_fd = server_in.as_raw_fd();
    daemon.stdout_fd = server_out.as_raw_fd();
    thread::spawn(move || {
        // Keep the server endpoints alive for as long as the loop runs.
        let _server_in = server_in;
        let _server_out = server_out;
        daemon.run();
    });

    // A lone MSG_SEP is an empty message: `parse_request` panics on it.
    client_in.write_all(b"\x1e").unwrap();
    // A valid handshake must still get its response after the panic.
    client_in.write_all(b"}hello\x1f\x1e").unwrap();

    let mut response = [0u8; 9];
    client_out
        .read_exact(&mut response)
        .expect("the daemon should still answer after a malformed request");
    assert_eq!(&response, b"}hello\x1f0\x1e");

    // Do not close the request side: EOF makes the daemon exit, and that exit belongs to the
    // real shell, not to this test.
    std::mem::forget(client_in);
}
