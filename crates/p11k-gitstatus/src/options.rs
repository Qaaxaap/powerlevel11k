//! 命令行参数（对齐 gitstatusd `options.cc:62-243` / `options.h:29-72`）。
//!
//! # 退出码
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 成功，或 EOF 正常退出 |
//! | 10 | 参数错误（打印用法后退出） |
//! | 11 | `-G` 版本 glob 不匹配 |
//!
//! zsh 侧 `gitstatus_start` 把 `-s/-u/-c/-d/-m/-e/-U/-W/-D` 直接透传，
//! 并默认追加 `-t 2*cpu`（上限 32）。因此参数解析必须容忍任意顺序、
//! `--long` 与 `-x` 两种写法、以及 `-xVALUE` 与 `-x VALUE` 两种赋值形式。

/// 参数字典。默认值必须与原版完全一致（见各字段注释）。
#[derive(Debug, Clone)]
pub struct Options {
    /// `-l/--lock-fd=N`：锁文件描述符探活（默认 -1 = 不启用）。
    /// 主循环 select 超时（1s）时 `fcntl(F_GETLK)` 检查该 fd 上的锁
    /// 是否仍被持有；不被持有说明父进程已死，daemon 应退出。
    pub lock_fd: i32,
    /// `-p/--parent-pid=N`：父进程探活（默认 -1）。超时后 `kill(pid, 0)`
    /// 失败即退出。bash 版传 `--parent-pid=$$`，zsh 版不传（靠 EOF + kill）。
    pub parent_pid: i32,
    /// `-t/--num-threads=N`：工作区扫描线程数（默认 1；zsh 设 2*cpu 上限 32）。
    pub num_threads: usize,
    /// `-v/--log-level`：日志级别。
    pub log_level: LogLevel,
    /// `-r/--repo-ttl-seconds`：闲置仓库 LRU 关闭秒数（默认 3600；负值=永不过期）。
    pub repo_ttl_seconds: i64,
    /// `-z/--max-commit-summary-length`：commit summary 截断字节数（默认 256）。
    pub max_commit_summary_length: usize,
    /// `-s`：staged 计数上限（默认 1；负值=无限）。
    pub max_num_staged: i64,
    /// `-u`：unstaged 计数上限（默认 1；负值=无限）。
    pub max_num_unstaged: i64,
    /// `-c`：conflicted 计数上限（默认 1；负值=无限）。
    pub max_num_conflicted: i64,
    /// `-d`：untracked 计数上限（默认 1；负值=无限）。
    pub max_num_untracked: i64,
    /// `-m/--dirty-max-index-size`：index 条目数超过此值时
    /// unstaged/untracked/conflicted 直接报 0（默认 -1 = 不启用）。
    pub dirty_max_index_size: i64,
    /// `-e`：递归统计 untracked 目录（默认只报告目录本身，不展开）。
    pub recurse_untracked_dirs: bool,
    /// `-U`：忽略 `status.showUntrackedFiles` 配置。
    pub ignore_status_show_untracked_files: bool,
    /// `-W`：忽略 `bash.showUntrackedFiles` 配置。
    pub ignore_bash_show_untracked_files: bool,
    /// `-D`：忽略 `bash.showDirtyState` 配置。
    pub ignore_bash_show_dirty_state: bool,
    /// `-G/--version-glob`：版本 fnmatch 匹配串；当前版本不匹配则 exit 11。
    /// zsh 侧传 `build.info` 里的 `gitstatus_version`（如 `v1.5.5`）。
    pub version_glob: Option<String>,
}

/// 日志级别（`-v` 的取值，大小写不敏感）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// 成功或 EOF 正常退出。
pub const EXIT_OK: i32 = 0;
/// 参数错误。
pub const EXIT_BAD_ARGS: i32 = 10;
/// `-G` 版本 glob 不匹配。
pub const EXIT_VERSION_MISMATCH: i32 = 11;

/// 解析命令行参数（不含 argv[0]）。
///
/// 错误时返回 `Err(错误描述)`，调用方打印用法后以 [`EXIT_BAD_ARGS`] 退出。
///
/// 实现要点：
/// - 支持 `-x` / `--long` / `-xVALUE` / `-x VALUE` / `--long=VALUE` 全部形式。
/// - `-h` 打印帮助（到 stdout）并返回特殊标记，`-V/--version` 打印版本——
///   与原版一样，这两个选项让 main 直接走快速路径退出 0。
/// - 布尔开关（`-e/-U/-W/-D`）不接受参数。
/// - 数字参数解析失败按参数错误处理（exit 10），不要 panic。
pub fn parse_args(args: &[String]) -> Result<Options, String> {
    let _ = args;
    todo!("实现：按上表解析全部参数与默认值；未知参数/坏数字报 Err")
}

/// 当前实现的协议版本串（原版为 `v1.5.5`，p11k 复刻后使用相同值，
/// 以便 `-G v1.5.5` 校验通过；也用于 `-V` 输出）。
pub const PROTOCOL_VERSION: &str = "v1.5.5";

/// `-G` 的 fnmatch 校验：版本串与 glob 匹配即通过，否则返回 false
/// （调用方以 [`EXIT_VERSION_MISMATCH`] 退出）。
///
/// 实现要点：原版用 C 的 fnmatch；Rust 侧可用等价的手写通配匹配
/// （只支持 `*` `?` 与字符类即可），或用 `glob` crate——注意行为要
/// 与原版 fnmatch(FNM_PATHNAME 未设置) 一致。
pub fn version_matches(version: &str, glob: &str) -> bool {
    let _ = (version, glob);
    todo!("实现：fnmatch 等价匹配（* ? [..]），不匹配返回 false")
}
