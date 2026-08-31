//! p11k-d：p11k 的常驻守护进程。
//!
//! # M1 阶段：gitstatusd 兼容模式
//!
//! 解析 gitstatus 参数全集，进入请求/响应主循环。目标：现有
//! powerlevel10k 的 `gitstatus.plugin.zsh` 无需修改即可把 p11k-d 当
//! gitstatusd 使用（把 `GITSTATUS_DAEMON` 指向 p11k-d）。
//!
//! 注意：pgid 握手由 zsh 侧在 exec 本进程之前完成（gitstatus.plugin.zsh:411），
//! 本进程只继承 stdin（FIFO 读端）与 stdout（管道写端）直接进入主循环。
//! stdout 只能写协议响应；一切日志走 stderr。
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
//! | 0 | 正常（EOF / 探活失败） |
//! | 10 | 参数错误 |
//! | 11 | `-G` 版本不匹配 |

use p11k_gitstatus::daemon::Daemon;
use p11k_gitstatus::options::{
    EXIT_BAD_ARGS, EXIT_VERSION_MISMATCH, Options, PROTOCOL_VERSION, ParseError, ParseOutcome,
    parse_args,
};
use std::process::ExitCode;

/// 用法文本。对齐原版 options.cc PrintUsage 的参数全集（措辞不必逐字一致）。
const USAGE: &str = "Usage: p11k-d [OPTION]...\n\
Print machine-readable status of the git repos for directories in stdin.\n\
\n\
  -h, --help              display this help and exit\n\
  -V, --version           output version information and exit\n\
  -G, --version-glob=PAT  exit with code 11 unless version matches PAT\n\
  -l, --lock-fd=N         exit when fd N is unlocked (default: -1)\n\
  -p, --parent-pid=N      exit when PID N terminates (default: -1)\n\
  -t, --num-threads=N     use N threads to scan git workdir (default: 1)\n\
  -v, --log-level=LEVEL   one of debug, info, warn, error, fatal (default: info)\n\
  -r, --repo-ttl-seconds=N  close git repositories that haven't been used for N seconds\n\
  -z, --max-commit-summary-length=N  truncate commit summary to N bytes\n\
  -s, --max-num-staged=N   cap staged count (default: 1)\n\
  -u, --max-num-unstaged=N cap unstaged count (default: 1)\n\
  -c, --max-num-conflicted=N  cap conflicted count (default: 1)\n\
  -d, --max-num-untracked=N  cap untracked count (default: 1)\n\
  -m, --dirty-max-index-size=N  report zero dirty files when index exceeds N\n\
  -e, --recurse-untracked-dirs  report untracked files like git status does\n\
  -U, --ignore-status-show-untracked-files  ignore status.showUntrackedFiles\n\
  -W, --ignore-bash-show-untracked-files  ignore bash.showUntrackedFiles\n\
  -D, --ignore-bash-show-dirty-state  ignore bash.showDirtyState\n";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let outcome = match parse_args(&args) {
        Ok(o) => o,
        Err(ParseError::BadArgs(msg)) => {
            eprintln!("gitstatusd: {msg}");
            return ExitCode::from(EXIT_BAD_ARGS as u8);
        }
        Err(ParseError::VersionMismatch { pattern }) => {
            // 对齐原版 options.cc 的措辞
            eprintln!("Version mismatch. Wanted (pattern): {pattern}. Actual: {PROTOCOL_VERSION}.");
            return ExitCode::from(EXIT_VERSION_MISMATCH as u8);
        }
    };
    match outcome {
        ParseOutcome::Help => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        ParseOutcome::Version => {
            println!("{PROTOCOL_VERSION}");
            ExitCode::SUCCESS
        }
        ParseOutcome::Run(options) => run_daemon(options),
    }
}

fn run_daemon(options: Options) -> ExitCode {
    let mut daemon = Daemon::new(options);
    daemon.run(); // run 内部以 exit(0) 结束；正常返回是死代码
    ExitCode::SUCCESS
}
