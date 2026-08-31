//! 进程生命周期与主循环（对齐 `gitstatus.plugin.zsh` 与 `gitstatus.cc`）。
//!
//! # 启动流程（zsh 侧负责，daemon 配合）
//!
//! 1. zsh 生成 `file_prefix = $TMPDIR/gitstatus.$name.$EUID.$pid.$EPOCHSECONDS.$n`。
//! 2. zsh 用 `zsystem flock` 锁住 `<prefix>.lock`，防止并发实例。
//! 3. zsh 在进程替换 `<( )` 内后台启动 daemon：`cd /`，
//!    daemon 的 **stdin 接 FIFO 读端**，**stdout 重定向到与 zsh 的管道**。
//! 4. daemon 启动后第一件事：向 stdout 写 **20 位左对齐的进程组号 pgid**；
//!    zsh 读出后作为 `GITSTATUS_DAEMON_PID`。
//! 5. 之后两条通道定型：zsh→FIFO→daemon stdin（请求），
//!    daemon stdout→pipe→zsh（响应）。FIFO 打开后立即 unlink。
//!
//! # 退出检测（三层冗余）
//!
//! - **EOF**：zsh 退出时关闭 FIFO 写端 → daemon 从 stdin 读到 0 字节 →
//!   正常退出（exit 0）。这是主路径。
//! - **主动 kill**：zsh 的 `zshexit` hook 执行 `kill -- -$daemon_pid`
//!   （杀整个进程组）。
//! - **探活**：主循环 select 超时（1s）时，若设了 `-l` 则
//!   `fcntl(F_GETLK)` 检查锁是否仍被持有；若设了 `-p` 则
//!   `kill(pid, 0)` 检查父进程是否存活。任一失败即退出。
//!
//! # 主循环
//!
//! 循环：select 等 stdin 可读（或 1s 超时）→ 读一条请求 →
//! 处理（见 [`crate::repo`]）→ 写响应。每个迭代顺带清理 TTL 到期的
//! 闲置仓库。请求处理须注意：单请求耗时长会阻塞后续请求，原版靠
//! 多线程分片 + 提前终止控制单次延迟；p11k 至少需要保证 EOF/探活
//! 检查不被长计算饿死。

use crate::options::Options;

/// 常驻 daemon 的运行时状态。
///
/// stdin 与 stdout 是裸 fd（原版语义：stdin=FIFO 读端、stdout=管道写端），
/// 不要在此之上包装任何缓冲层——协议消息边界由 MSG_SEP 决定，
/// 必须逐字节控制写入。
pub struct Daemon {
    /// 启动时解析好的选项。
    pub options: Options,
    /// stdin fd（FIFO 读端）。
    pub stdin_fd: i32,
    /// stdout fd（管道写端）。
    pub stdout_fd: i32,
    // TODO(实现者)：补充字段——
    // - 仓库缓存（crate::repo::RepoCache）
    // - 线程池（index 分片扫描用）
    // - 探活所需的锁/父进程信息（从 options 取）
}

impl Daemon {
    /// 启动握手第一步：向 stdout 写 20 位左对齐的进程组号。
    ///
    /// 格式：`getpgrp()` 的十进制值，左对齐，不足 20 位以空格补齐，
    /// 总长**恰好 20 字节**，写完立即 flush。zsh 侧按固定 20 字节读取，
    /// 多一字节少一字节都会破坏后续握手。
    pub fn handshake_pgid(&self) {
        todo!("实现：libc::getpgrp() → 格式化为 20 字节左对齐 → write 到 stdout_fd")
    }

    /// 主循环：读请求 → 处理 → 写响应，直到 EOF 或探活失败。
    ///
    /// 实现要点：
    /// - 用 `select`（或 poll）等 stdin_fd，超时 1s。
    /// - 超时分支执行 `-l`/`-p` 探活与 TTL 清理。
    /// - 读到 0 字节（EOF）→ 正常返回（main 以 exit 0 结束）。
    /// - 读到的字节可能包含多条消息（zsh 会批量写入），按 MSG_SEP
    ///   切分逐条处理；不完整的尾部消息留在缓冲区等下一轮。
    /// - 握手请求（id=`}hello`、dir 为空）必须回 `}hello` + `0`。
    pub fn run(&mut self) {
        todo!("实现：select 主循环 + 读缓冲 + 消息切分 + 探活/TTL + 请求分发")
    }
}
