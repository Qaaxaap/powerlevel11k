use crate::options::Options;
use crate::protocol::{self, Request, Response};
use crate::repo::RepoCache;
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
    /// - The buffer is split on MSG_SEP; one read may carry several messages.
    /// - With no complete message buffered, poll stdin with a 1s timeout and
    ///   run the `-l`/`-p` liveness check plus TTL eviction.
    /// - read returning 0 = EOF (zsh exited, closing the FIFO write end).
    /// - A failing request must not kill the daemon.
    pub fn run(&mut self) {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            // Complete message buffered?
            if let Some(pos) = buf.iter().position(|&b| b == protocol::MSG_SEP) {
                let msg: Vec<u8> = buf.drain(..=pos).collect();
                let bytes = &msg[..msg.len() - 1]; // strip MSG_SEP
                let req = protocol::parse_request(bytes);
                self.process_request(req);
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
                self.cache.evict_expired();
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
    /// A panicking request is caught and logged so the daemon keeps serving.
    /// An empty dir yields a non-repo response (the `}hello` handshake also
    /// takes this path); otherwise open the repo — success gives a repo
    /// response, failure gives a non-repo response.
    fn process_request(&mut self, req: Request) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
            self.write_response(&resp);
        }));
        if result.is_err() {
            eprintln!("gitstatusd: error processing request");
        }
    }

    /// Write a serialized response to stdout in a loop (short writes, EINTR).
    ///
    /// EPIPE (zsh died) → exit 0. The Rust runtime ignores SIGPIPE, so EPIPE
    /// is handled explicitly to match the original's default disposition.
    fn write_response(&self, resp: &Response) {
        let bytes = protocol::serialize_response(resp);
        let mut off = 0;
        while off < bytes.len() {
            // SAFETY: writes bytes[off..], length is bounded.
            let n = unsafe {
                libc::write(
                    self.stdout_fd,
                    bytes[off..].as_ptr().cast(),
                    bytes.len() - off,
                )
            };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                match err.kind() {
                    std::io::ErrorKind::Interrupted => continue,
                    std::io::ErrorKind::BrokenPipe => exit(0),
                    _ => {
                        eprintln!("gitstatusd: write: {err}");
                        exit(0);
                    }
                }
            }
            off += n as usize;
        }
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
