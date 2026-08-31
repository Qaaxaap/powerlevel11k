use crate::options::Options;
use crate::protocol::{self, Request, Response};
use crate::repo::RepoCache;
use std::process::exit;

/// 常驻 daemon 的运行时状态。
///
/// stdin 与 stdout 是裸 fd（原版语义：stdin=FIFO 读端、stdout=管道写端）。
/// 注意：pgid 握手由 zsh 侧在 exec 本进程**之前**完成（gitstatus.plugin.zsh:411），
/// daemon 无需也**不得**向 stdout 写任何协议之外的内容。
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

    /// 主循环：读请求 → 处理 → 写响应，直到 EOF 或探活失败。
    ///
    /// 对齐 request.cc 的 RequestReader + gitstatus.cc 的主循环：
    /// - 读缓冲按 MSG_SEP 切分；一次 read 可能带回多条消息。
    /// - 缓冲无完整消息时 poll stdin，1s 超时：执行 `-l`/`-p` 探活与 TTL 清理。
    /// - read 返回 0 = EOF（zsh 退出关 FIFO 写端）→ exit 0。
    /// - 单请求处理失败不杀 daemon（对齐原版 try/catch + LOG(ERROR)）。
    pub fn run(&mut self) {
        let mut buf: Vec<u8> = Vec::new();
        loop {
            // 1. 缓冲里已有完整消息 → 切出处理
            if let Some(pos) = buf.iter().position(|&b| b == protocol::MSG_SEP) {
                let msg: Vec<u8> = buf.drain(..=pos).collect();
                let bytes = &msg[..msg.len() - 1]; // 去掉 MSG_SEP
                let req = protocol::parse_request(bytes);
                self.process_request(req);
                continue;
            }
            // 2. poll stdin，1s 超时
            let mut pfd = libc::pollfd {
                fd: self.stdin_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: pollfd 布局由 libc crate 保证；1s 超时对齐原版 select 的 timeval{1}。
            let n = unsafe { libc::poll(&mut pfd, 1, 1000) };
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue; // EINTR：重试（原版会直接死，p11k 更稳；行为差异可接受）
                }
                eprintln!("gitstatusd: poll: {err}");
                exit(0);
            }
            if n == 0 {
                if !self.liveness_ok() {
                    exit(0); // 对齐原版：探活失败 exit 0
                }
                self.cache.evict_expired();
                continue;
            }
            // 3. 可读
            let mut chunk = [0u8; 256];
            // SAFETY: 写入栈上数组，nread 检查后取切片。
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
                // EOF：zsh 退出关闭了 FIFO 写端 → 正常退出
                exit(0);
            }
            buf.extend_from_slice(&chunk[..nread as usize]);
        }
    }

    /// 处理一条请求并写响应。
    ///
    /// 对齐原版 ProcessRequest + ResponseWriter：
    /// - 单请求 panic 被捕获（catch_unwind），记错误日志后继续服务，
    ///   对齐原版 `catch (const Exception&) { LOG(ERROR) }`。
    /// - dir 为空的请求 → 非仓库响应（找不到仓库，对齐原版语义；
    ///   握手请求 id=}hello、dir 为空也走这条路径）。
    /// - 其余请求：打开仓库成功 → 仓库响应（build_fields 内的 dirty 字段
    ///   TODO(index) 会 panic 并被捕获，该请求暂无响应）；打不开 → 非仓库。
    fn process_request(&mut self, req: Request) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let resp = match self.cache.get_or_open(&req.dir, req.dir_is_gitdir) {
                Some(repo) => {
                    let fields = repo.build_fields(req.skip_index);
                    Response {
                        id: req.id.clone(),
                        is_repo: true,
                        fields: Some(fields),
                    }
                }
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

    /// 把序列化后的响应完整写入 stdout（循环写，处理短写与 EINTR）。
    ///
    /// EPIPE（zsh 已死）→ exit 0。原版靠 SIGPIPE 默认处置终止进程；
    /// Rust 运行时默认忽略 SIGPIPE，故显式处理 EPIPE，语义等价。
    fn write_response(&self, resp: &Response) {
        let bytes = protocol::serialize_response(resp);
        let mut off = 0;
        while off < bytes.len() {
            // SAFETY: 写入 bytes[off..]，长度受控。
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

    /// 探活（对齐 request.cc IsLockedFd + parent_pid 检查）：
    /// - `-l` 设了锁 fd：fcntl(F_GETLK) 检查锁仍被持有
    /// - `-p` 设了父 pid：kill(pid, 0) 检查父进程存活
    ///
    /// 任一失败返回 false（调用方 exit 0）。
    fn liveness_ok(&self) -> bool {
        if self.options.lock_fd >= 0 {
            let mut fl = libc::flock {
                l_type: libc::F_RDLCK as _,
                l_whence: libc::SEEK_SET as _,
                l_start: 0,
                l_len: 0,
                l_pid: 0,
            };
            // SAFETY: fl 为合法 flock 结构，F_GETLK 填充之。
            let locked = unsafe { libc::fcntl(self.options.lock_fd, libc::F_GETLK, &mut fl) } == 0
                && fl.l_type != libc::F_UNLCK as _;
            if !locked {
                return false;
            }
        }
        if self.options.parent_pid >= 0 {
            // SAFETY: 信号 0 探活，标准用法。
            if unsafe { libc::kill(self.options.parent_pid, 0) } != 0 {
                return false;
            }
        }
        true
    }
}
