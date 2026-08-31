//! p11k-d：p11k 的常驻守护进程。
//!
//! # M1 阶段：gitstatusd 兼容模式
//!
//! 解析 gitstatus 参数全集，完成 pgid 握手，进入请求/响应主循环。
//! 目标：现有 powerlevel10k 的 `gitstatus.plugin.zsh` 无需修改即可
//! 把 p11k-d 当 gitstatusd 使用（把 `GITSTATUS_DAEMON` 指向 p11k-d）。
//!
//! 启动时序（必须在读任何 stdin 之前完成）：
//! 1. 解析参数；`-h` / `-V` 走快速路径（打印后 exit 0）。
//! 2. `-G` 版本 glob 校验，不匹配 exit 11。
//! 3. 构造 [`p11k_gitstatus::daemon::Daemon`]。
//! 4. `handshake_pgid()`：向 stdout 写 20 字节左对齐 pgid 并 flush。
//! 5. `run()`：主循环，EOF 后正常返回 exit 0。
//!
//! # 后续阶段
//!
//! 在 gitstatus 兼容模式之上增加 p11k 原生协议（渲染引擎、异步分段），
//! 由启动参数/环境变量选择模式，默认仍保持 gitstatus 兼容。
//!
//! # 退出码
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 正常（EOF / 握手后正常退出） |
//! | 10 | 参数错误 |
//! | 11 | `-G` 版本不匹配 |

use std::process::ExitCode;

fn main() -> ExitCode {
    todo!(
        "实现：收集 args → p11k_gitstatus::options::parse_args → \
         -h/-V 快速路径 → -G 校验(exit 11) → 构造 Daemon → \
         handshake_pgid → run → ExitCode::SUCCESS"
    )
}
