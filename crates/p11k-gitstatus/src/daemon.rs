use crate::options::Options;
use crate::protocol::{self, Request, Response};
use crate::repo::RepoCache;
use std::panic::AssertUnwindSafe;
use std::process::exit;

/// Resident daemon state.
///
/// stdin/stdout are raw fds (stdin = FIFO read end, stdout = pipe write end).
/// The pgid handshake is done by the zsh side before exec'ing this process;
/// stdout must carry only protocol responses.
pub struct Daemon {
    pub options: Options,
    pub stdin_fd: i32,
    pub stdout_fd: i32,
    cache: RepoCache,
}

impl Daemon {
    pub fn new(options: Options) -> Daemon {
        // `-v/--log-level` sets the minimum level the daemon reports (gitstatusd's
        // `g_min_log_level`). `env_logger` writes to stderr, which matters here: stdout carries
        // the protocol and any other writer would corrupt the responses. Initialising twice is
        // not an error; tests may build more than one daemon.
        let _ = env_logger::Builder::new()
            .filter_level(options.log_level.as_filter())
            .format_timestamp(None)
            .try_init();
        Daemon {
            cache: RepoCache::new(&options),
            options,
            stdin_fd: 0,
            stdout_fd: 1,
        }
    }

    /// Main loop: read request → process → write response, until EOF or
    /// liveness fails.
    ///
    /// Requests run one at a time, in order. The original answers them from a single loop
    /// (`gitstatus.cc`), and the protocol tests depend on that order. `-t` sizes the scan
    /// thread pool inside the git status core, not the request count.
    ///
    /// - The buffer is split on MSG_SEP; one read may carry several messages.
    /// - With no complete message buffered, poll stdin with a 1s timeout and
    ///   run the `-l`/`-p` liveness check plus TTL eviction.
    /// - read returning 0 = EOF (zsh exited, closing the FIFO write end).
    /// - A response that cannot be written stops the daemon: the stream is at an arbitrary
    ///   offset and the shell restarts it. A closed pipe means the shell is gone, so that case
    ///   exits quietly.
    pub fn run(&mut self) -> ! {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            // Complete message buffered?
            if let Some(pos) = buf.iter().position(|&b| b == protocol::MSG_SEP) {
                let msg: Vec<u8> = buf.drain(..=pos).collect();
                let bytes = &msg[..msg.len() - 1]; // strip MSG_SEP
                let req = protocol::parse_request(bytes);
                if let Err(err) = self.process_request(req) {
                    match err {
                        // The shell exited and closed the pipe: the original exits quietly.
                        RequestError::Write(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                            exit(0)
                        }
                        RequestError::Write(e) => {
                            eprintln!("gitstatusd: write: {e}");
                            exit(1);
                        }
                        RequestError::Panic => {
                            eprintln!("gitstatusd: error processing request");
                            exit(1);
                        }
                    }
                }
                continue;
            }
            // Poll stdin, 1s timeout.
            let mut pfd = libc::pollfd {
                fd: self.stdin_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: pollfd layout is fixed by libc; 1s matches the original select.
            let n = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue; // EINTR: retry
                }
                eprintln!("gitstatusd: poll: {err}");
                exit(0);
            }
            if n == 0 {
                if !self.liveness_ok() {
                    exit(0);
                }
                self.evict_expired();
                continue;
            }
            // Readable.
            let mut chunk = [0u8; 256];
            // SAFETY: writes into a stack array; nread bounds the slice.
            let nread =
                unsafe { libc::read(self.stdin_fd, chunk.as_mut_ptr().cast(), chunk.len()) };
            if nread < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                eprintln!("gitstatusd: read: {err}");
                exit(0);
            }
            if nread == 0 {
                // EOF: zsh closed the FIFO write end.
                exit(0);
            }
            buf.extend_from_slice(&chunk[..nread as usize]);
        }
    }

    /// Handle one request and write the response.
    ///
    /// A panicking request is caught so the daemon reports what happened; the caller decides
    /// whether the loop can continue. An empty dir yields a non-repo response (the `}hello`
    /// handshake also takes this path); otherwise open the repo — success gives a repo
    /// response, failure gives a non-repo response.
    fn process_request(&mut self, req: Request) -> Result<(), RequestError> {
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
            let resp = match self.cache.get_or_open(&req.dir, req.dir_is_gitdir) {
                Some(repo) => Response {
                    id: req.id.clone(),
                    is_repo: true,
                    fields: Some(repo.build_fields(req.skip_index)),
                },
                None => Response {
                    id: req.id.clone(),
                    is_repo: false,
                    fields: None,
                },
            };
            self.write_response(&resp)
        }));
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(RequestError::Write(err)),
            Err(_) => Err(RequestError::Panic),
        }
    }

    /// Write a serialized response to stdout (short writes, EINTR).
    ///
    /// A failed write leaves the stream at an arbitrary offset, so the caller stops the daemon
    /// instead of trying to continue.
    fn write_response(&self, resp: &Response) -> std::io::Result<()> {
        let bytes = protocol::serialize_response(resp);
        write_all(self.stdout_fd, &bytes)
    }

    fn evict_expired(&mut self) {
        self.cache.evict_expired();
    }

    /// Liveness probe (`-l` lock fd + `-p` parent pid). Any failure → false,
    /// and the caller exits 0.
    fn liveness_ok(&self) -> bool {
        if self.options.lock_fd >= 0 {
            let mut fl = libc::flock {
                l_type: libc::F_RDLCK as _,
                l_whence: libc::SEEK_SET as _,
                l_start: 0,
                l_len: 0,
                l_pid: 0,
            };
            // SAFETY: fl is a valid flock; F_GETLK fills it.
            let locked = unsafe { libc::fcntl(self.options.lock_fd, libc::F_GETLK, &mut fl) } == 0
                && fl.l_type != libc::F_UNLCK as _;
            if !locked {
                return false;
            }
        }
        if self.options.parent_pid >= 0 {
            // SAFETY: signal-0 probe, standard usage.
            if unsafe { libc::kill(self.options.parent_pid, 0) } != 0 {
                return false;
            }
        }
        true
    }
}

/// Why a request could not be answered.
enum RequestError {
    /// The response could not be written completely.
    Write(std::io::Error),
    /// The handler panicked; the response may be partial.
    Panic,
}

/// Write every byte to `fd` (short writes, EINTR).
fn write_all(fd: i32, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        // SAFETY: writes a bounded slice; the return value bounds the advance.
        let n = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        bytes = &bytes[n as usize..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A failed write must come back as an error; the main loop alone decides how to stop.
    /// `ResponseGuard` called `exit(1)` from `Drop`, so the caller never saw this error.
    #[test]
    fn a_failed_write_is_returned() {
        let mut fds = [0i32; 2];
        // SAFETY: pipe() writes two fds into the array.
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        // SAFETY: close the read end so the write fails with EPIPE.
        unsafe { libc::close(fds[0]) };
        let mut daemon = Daemon::new(Options::default());
        daemon.stdout_fd = fds[1];
        let resp = Response {
            id: b"1".to_vec(),
            is_repo: false,
            fields: None,
        };
        let err = daemon
            .write_response(&resp)
            .expect_err("a closed pipe must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::BrokenPipe);
        // SAFETY: close the write end.
        unsafe { libc::close(fds[1]) };
    }
}
