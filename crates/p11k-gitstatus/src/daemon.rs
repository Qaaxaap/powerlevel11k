use crate::options::Options;
use crate::protocol::{self, Request, Response};
use crate::repo::RepoCache;
use std::panic::AssertUnwindSafe;
use std::process::exit;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Like gitstatusd, one request runs on its own thread, and a burst is capped instead of
/// spawning without bound.
const MAX_REQUEST_THREADS: usize = 512;

/// Resident daemon state.
///
/// stdin/stdout are raw fds (stdin = FIFO read end, stdout = pipe write end).
/// The pgid handshake is done by the zsh side before exec'ing this process;
/// stdout must carry only protocol responses.
pub struct Daemon {
    pub options: Options,
    pub stdin_fd: i32,
    pub stdout_fd: i32,
    /// Shared with the request threads. gitstatusd serializes requests under one repo lock,
    /// and so does this: the lock is held across the scan that builds the response.
    cache: Arc<Mutex<RepoCache>>,
    /// Serializes response writes; a request thread owns the stream while it writes.
    write_lock: Arc<Mutex<()>>,
    /// Request threads in flight.
    active: Arc<AtomicUsize>,
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
            cache: Arc::new(Mutex::new(RepoCache::new(&options))),
            write_lock: Arc::new(Mutex::new(())),
            active: Arc::new(AtomicUsize::new(0)),
            options,
            stdin_fd: 0,
            stdout_fd: 1,
        }
    }

    /// Main loop: read request → hand it to a worker → keep reading, until EOF
    /// or liveness fails.
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
                self.handle_request(req);
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
                // EOF: zsh closed the FIFO write end. In-flight requests still have to answer
                // on stdout, so wait for them before exiting.
                while self.active.load(Ordering::Acquire) > 0 {
                    std::thread::sleep(Duration::from_millis(1));
                }
                exit(0);
            }
            buf.extend_from_slice(&chunk[..nread as usize]);
        }
    }

    /// Run one request on its own thread. An empty dir yields a non-repo
    /// response (the `}hello` handshake also takes this path); otherwise open
    /// the repo — success gives a repo response, failure gives a non-repo
    /// response.
    fn handle_request(&mut self, req: Request) {
        // Cap the burst: gitstatusd never runs more than 512 requests at once, and neither
        // does this. The wait is rare and bounded by the scans themselves.
        while self.active.load(Ordering::Acquire) >= MAX_REQUEST_THREADS {
            std::thread::sleep(Duration::from_millis(1));
        }

        let cache = Arc::clone(&self.cache);
        let write_lock = Arc::clone(&self.write_lock);
        let active = Arc::clone(&self.active);
        let stdout_fd = self.stdout_fd;
        active.fetch_add(1, Ordering::AcqRel);
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let resp = {
                    let mut cache = cache
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    match cache.get_or_open(&req.dir, req.dir_is_gitdir) {
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
                    }
                };
                let _writing = write_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                write_response(stdout_fd, &resp)
            }));
            active.fetch_sub(1, Ordering::AcqRel);
            match result {
                Ok(Ok(())) => {}
                // The shell exited and closed the pipe: the original exits quietly.
                Ok(Err(err)) if err.kind() == std::io::ErrorKind::BrokenPipe => exit(0),
                Ok(Err(err)) => {
                    eprintln!("gitstatusd: write: {err}");
                    exit(1);
                }
                Err(_) => {
                    eprintln!("gitstatusd: error processing request");
                    exit(1);
                }
            }
        });
    }

    /// Evict expired repos. A request thread may hold the cache lock while it scans, so this
    /// waits for that scan and no longer.
    fn evict_expired(&self) {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .evict_expired();
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

/// Write a serialized response to stdout in a loop (short writes, EINTR).
///
/// The guard makes the write all-or-nothing: a response that stops midway
/// would desynchronize every later reply on the stream.
fn write_response(fd: i32, resp: &Response) -> std::io::Result<()> {
    let bytes = protocol::serialize_response(resp);
    let mut guard = ResponseGuard::new(fd);
    guard.write_all(&bytes)?;
    guard.complete();
    Ok(())
}

/// Writes one protocol response and kills the daemon if it cannot finish.
///
/// A partial response leaves the stream at an arbitrary offset, so the only
/// correct outcome is to stop: the owning shell restarts the daemon.
struct ResponseGuard {
    fd: i32,
    complete: bool,
}

impl ResponseGuard {
    fn new(fd: i32) -> Self {
        Self {
            fd,
            complete: false,
        }
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> std::io::Result<()> {
        while !bytes.is_empty() {
            // SAFETY: writes a bounded slice; the return value bounds the advance.
            let n = unsafe { libc::write(self.fd, bytes.as_ptr().cast(), bytes.len()) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            bytes = &bytes[n as usize..];
        }
        Ok(())
    }

    fn complete(&mut self) {
        self.complete = true;
    }
}

impl Drop for ResponseGuard {
    fn drop(&mut self) {
        if !self.complete {
            eprintln!("gitstatusd: response aborted mid-write");
            exit(1);
        }
    }
}
