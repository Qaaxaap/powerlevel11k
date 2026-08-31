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
impl Default for Options {
    /// 全部默认值，与原版 `options.cc:58-79` 保持一致。
    fn default() -> Self {
        Self {
            lock_fd: -1,
            parent_pid: -1,
            num_threads: 1,
            log_level: LogLevel::Info,
            repo_ttl_seconds: 3600,
            max_commit_summary_length: 256,
            max_num_staged: 1,
            max_num_unstaged: 1,
            max_num_conflicted: 1,
            max_num_untracked: 1,
            dirty_max_index_size: -1,
            recurse_untracked_dirs: false,
            ignore_status_show_untracked_files: false,
            ignore_bash_show_untracked_files: false,
            ignore_bash_show_dirty_state: false,
            version_glob: None,
        }
    }
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
impl LogLevel {
    /// 与 libgit2 的 `git_trace_level_t` 对应。
    pub fn as_git_trace_level(self) -> i32 {
        match self {
            Self::Debug => 5,
            Self::Info => 4,
            Self::Warn => 3,
            Self::Error => 2,
            Self::Fatal => 1,
        }
    }
}

/// 成功或 EOF 正常退出。
pub const EXIT_OK: i32 = 0;
/// 参数错误。
pub const EXIT_BAD_ARGS: i32 = 10;
/// `-G` 版本 glob 不匹配。
pub const EXIT_VERSION_MISMATCH: i32 = 11;

/// 解析命令行参数（不含 argv[0]）。
///
/// `Err(BadArgs)` 时调用方打印错误与用法后以 [`EXIT_BAD_ARGS`] 退出；
/// `Err(VersionMismatch)` 时以 [`EXIT_VERSION_MISMATCH`] 退出。
///
/// 实现要点：
/// - 支持 `-x` / `--long` / `-xVALUE` / `-x VALUE` / `--long=VALUE` 全部形式；
///   与原版 getopt 一样，取值短选项的值可作为下一个参数（`-t 4`）。
/// - `-h`/`-V` 走 [`ParseOutcome::Help`]/[`ParseOutcome::Version`] 快速路径。
/// - 布尔开关（`-e/-U/-W/-D` 及其 long 形式）不接受参数。
/// - 数字参数解析失败按参数错误处理（exit 10），不要 panic。
/// - `-G` 在解析期立即校验版本（对齐原版 options.cc：不匹配 exit 11）。
/// - 原版 getopt 的 GNU 特性（选项重排、`--` 分隔符）不做支持：zsh 侧
///   不会传位置参数，出现位置参数一律报错。
#[derive(Debug)]
pub enum ParseOutcome {
    /// 用户要求打印帮助（`-h`/`--help`），main 打印用法后退出 0。
    Help,
    /// 用户要求打印版本（`-V`/`--version`），main 打印版本后退出 0。
    Version,
    /// 解析完成，进入 daemon 主流程。
    Run(Options),
}

/// 参数解析的错误，映射到不同的退出码。
#[derive(Debug)]
pub enum ParseError {
    /// 参数错误 → 退出码 [`EXIT_BAD_ARGS`]。
    BadArgs(String),
    /// `-G` 版本 glob 不匹配 → 退出码 [`EXIT_VERSION_MISMATCH`]。
    VersionMismatch {
        /// 用户传入的 glob 模式。
        pattern: String,
    },
}

/// parse_short_opt / parse_long_opt 共用的内部错误；
/// [`ParseError::VersionMismatch`] 只能由 `-G` 分支触发。
enum OptError {
    Bad(String),
    VersionMismatch(String),
}

impl From<OptError> for ParseError {
    fn from(e: OptError) -> Self {
        match e {
            OptError::Bad(s) => ParseError::BadArgs(s),
            OptError::VersionMismatch(p) => ParseError::VersionMismatch { pattern: p },
        }
    }
}

pub fn parse_args(args: &[String]) -> Result<ParseOutcome, ParseError> {
    let mut options = Options::default();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "-h" || arg == "--help" {
            return Ok(ParseOutcome::Help);
        }
        if arg == "-V" || arg == "--version" {
            return Ok(ParseOutcome::Version);
        }
        if let Some(long) = arg.strip_prefix("--") {
            // 注意：布尔型 long 选项（--recurse-untracked-dirs 等）不带值。
            if let Some(eq) = long.find('=') {
                let name = &long[..eq];
                let value = &long[eq + 1..];
                if long_opt_takes_value(name) {
                    parse_long_opt(&mut options, name, Some(value))?;
                } else {
                    return Err(ParseError::BadArgs(format!(
                        "option --{name} does not take an argument"
                    )));
                }
            } else if long_opt_takes_value(long) {
                return Err(ParseError::BadArgs(format!(
                    "option --{long} requires an argument"
                )));
            } else {
                parse_long_opt(&mut options, long, None)?;
            }
        } else if arg.starts_with('-') && arg.len() > 1 {
            let name = &arg[1..2];
            if arg.len() > 2 {
                // -xVALUE 形式（-t4、-Gv1.5.5）；布尔选项带值会在解析层被拒绝。
                parse_short_opt(&mut options, name, Some(&arg[2..]))?;
            } else if short_opt_takes_value(name) {
                // -x VALUE 形式：值在下一个参数。
                i += 1;
                if i >= args.len() {
                    return Err(ParseError::BadArgs(format!(
                        "option -{name} requires an argument"
                    )));
                }
                parse_short_opt(&mut options, name, Some(&args[i]))?;
            } else {
                // 布尔短选项（-e/-U/-W/-D）。
                parse_short_opt(&mut options, name, None)?;
            }
        } else {
            // 对齐原版措辞（options.cc 的 "unexpected positional argument"）。
            return Err(ParseError::BadArgs(format!(
                "unexpected positional argument: {arg}"
            )));
        }
        i += 1;
    }
    Ok(ParseOutcome::Run(options))
}

/// 该短选项是否要求值。带值：l p t v r z s u c d m G；布尔：e U W D。
fn short_opt_takes_value(name: &str) -> bool {
    matches!(
        name,
        "l" | "p" | "t" | "v" | "r" | "z" | "s" | "u" | "c" | "d" | "m" | "G"
    )
}

/// 该 long 选项是否要求值。布尔型 long（对应 -e/-U/-W/-D）返回 false。
fn long_opt_takes_value(name: &str) -> bool {
    matches!(
        name,
        "version-glob"
            | "lock-fd"
            | "parent-pid"
            | "num-threads"
            | "log-level"
            | "repo-ttl-seconds"
            | "max-commit-summary-length"
            | "max-num-staged"
            | "max-num-unstaged"
            | "max-num-conflicted"
            | "max-num-untracked"
            | "dirty-max-index-size"
    )
}

fn parse_short_opt(options: &mut Options, name: &str, value: Option<&str>) -> Result<(), OptError> {
    let opt = format!("-{name}");
    match name {
        "l" => options.lock_fd = parse_int(value, &opt)?,
        "p" => options.parent_pid = parse_int(value, &opt)?,
        "t" => {
            let n: usize = parse_int(value, &opt)?;
            if n == 0 {
                // 对齐原版 options.cc：num_threads 必须 > 0。
                return Err(OptError::Bad("invalid number of threads: 0".to_string()));
            }
            options.num_threads = n;
        }
        "v" => options.log_level = parse_log_level(value, &opt)?,
        "r" => options.repo_ttl_seconds = parse_int(value, &opt)?,
        "z" => options.max_commit_summary_length = parse_size(value, &opt)?,
        "s" => options.max_num_staged = parse_limit(value, &opt)?,
        "u" => options.max_num_unstaged = parse_limit(value, &opt)?,
        "c" => options.max_num_conflicted = parse_limit(value, &opt)?,
        "d" => options.max_num_untracked = parse_limit(value, &opt)?,
        "m" => options.dirty_max_index_size = parse_limit(value, &opt)?,
        "G" => {
            let pattern = take_value(value, &opt)?;
            // 对齐原版"解析期校验"（options.cc case 'G'）：立即 fnmatch，
            // 不匹配走 exit 11 而非 10。
            if !version_matches(PROTOCOL_VERSION, pattern) {
                return Err(OptError::VersionMismatch(pattern.to_string()));
            }
            options.version_glob = Some(pattern.to_string());
        }
        "e" => {
            reject_value(value, &opt)?;
            options.recurse_untracked_dirs = true;
        }
        "U" => {
            reject_value(value, &opt)?;
            options.ignore_status_show_untracked_files = true;
        }
        "W" => {
            reject_value(value, &opt)?;
            options.ignore_bash_show_untracked_files = true;
        }
        "D" => {
            reject_value(value, &opt)?;
            options.ignore_bash_show_dirty_state = true;
        }
        _ => return Err(OptError::Bad(format!("unrecognized option: -{name}"))),
    }
    Ok(())
}

fn parse_long_opt(options: &mut Options, name: &str, value: Option<&str>) -> Result<(), OptError> {
    let opt = format!("--{name}");
    match name {
        "lock-fd" => options.lock_fd = parse_int(value, &opt)?,
        "parent-pid" => options.parent_pid = parse_int(value, &opt)?,
        "num-threads" => {
            let n: usize = parse_int(value, &opt)?;
            if n == 0 {
                return Err(OptError::Bad("invalid number of threads: 0".to_string()));
            }
            options.num_threads = n;
        }
        "log-level" => options.log_level = parse_log_level(value, &opt)?,
        "repo-ttl-seconds" => options.repo_ttl_seconds = parse_int(value, &opt)?,
        "max-commit-summary-length" => options.max_commit_summary_length = parse_size(value, &opt)?,
        "max-num-staged" => options.max_num_staged = parse_limit(value, &opt)?,
        "max-num-unstaged" => options.max_num_unstaged = parse_limit(value, &opt)?,
        "max-num-conflicted" => options.max_num_conflicted = parse_limit(value, &opt)?,
        "max-num-untracked" => options.max_num_untracked = parse_limit(value, &opt)?,
        "dirty-max-index-size" => options.dirty_max_index_size = parse_limit(value, &opt)?,
        "version-glob" => {
            let pattern = take_value(value, &opt)?;
            // 同 -G：解析期校验。
            if !version_matches(PROTOCOL_VERSION, pattern) {
                return Err(OptError::VersionMismatch(pattern.to_string()));
            }
            options.version_glob = Some(pattern.to_string());
        }
        "recurse-untracked-dirs" => {
            no_value_ok(value, &opt)?;
            options.recurse_untracked_dirs = true;
        }
        "ignore-status-show-untracked-files" => {
            no_value_ok(value, &opt)?;
            options.ignore_status_show_untracked_files = true;
        }
        "ignore-bash-show-untracked-files" => {
            no_value_ok(value, &opt)?;
            options.ignore_bash_show_untracked_files = true;
        }
        "ignore-bash-show-dirty-state" => {
            no_value_ok(value, &opt)?;
            options.ignore_bash_show_dirty_state = true;
        }
        _ => return Err(OptError::Bad(format!("unrecognized option: --{name}"))),
    }
    Ok(())
}

/// 带值选项取值：None 报缺参，Some 解析失败报"非整数"。
///
/// 对齐原版 `strtol`（ParseLong）：允许前导空白（`" 4"`），拒绝尾随垃圾
/// （`"4x"`）。Rust 的 `parse` 不接受前导空白，故先 trim。
fn parse_int<T: std::str::FromStr>(value: Option<&str>, opt: &str) -> Result<T, OptError> {
    let v = value.ok_or_else(|| OptError::Bad(format!("option {opt} requires an argument")))?;
    v.trim()
        .parse()
        .map_err(|_| OptError::Bad(format!("not an integer: {v}")))
}

/// 对齐原版 ParseSizeT（options.cc:56-59）：解析为 i64，
/// 负数统一映射为 -1（原版存 size_t，-1 = SIZE_MAX）。
fn parse_limit(value: Option<&str>, opt: &str) -> Result<i64, OptError> {
    let n: i64 = parse_int(value, opt)?;
    Ok(if n < 0 { -1 } else { n })
}

/// 对齐原版 ParseSizeT 的 size_t 语义：负数映射为 usize::MAX
/// （`-z` 的 summary 长度上限，负值 = 不截断）。
fn parse_size(value: Option<&str>, opt: &str) -> Result<usize, OptError> {
    let n: i64 = parse_int(value, opt)?;
    Ok(if n < 0 { usize::MAX } else { n as usize })
}

/// 布尔短选项拒绝带值（`-efoo` 应报错，不静默忽略）。
fn reject_value(value: Option<&str>, opt: &str) -> Result<(), OptError> {
    match value {
        Some(_) => Err(OptError::Bad(format!(
            "option {opt} does not take an argument"
        ))),
        None => Ok(()),
    }
}

/// 布尔 long 选项兜底：正常情况下主循环已拦截 `--flag=value`，
/// 这里防御性再查一次。
fn no_value_ok(value: Option<&str>, opt: &str) -> Result<(), OptError> {
    match value {
        Some(_) => Err(OptError::Bad(format!(
            "option {opt} does not take an argument"
        ))),
        None => Ok(()),
    }
}

/// 字符串取值（`-G` 专用）。
fn take_value<'a>(value: Option<&'a str>, opt: &str) -> Result<&'a str, OptError> {
    value.ok_or_else(|| OptError::Bad(format!("option {opt} requires an argument")))
}

/// `-v` 取值：大小写不敏感，debug/info/warn/error/fatal。
/// 对齐原版错误措辞 "invalid log level: X"。
fn parse_log_level(value: Option<&str>, opt: &str) -> Result<LogLevel, OptError> {
    let v = value.ok_or_else(|| OptError::Bad(format!("option {opt} requires an argument")))?;
    match v.to_ascii_lowercase().as_str() {
        "debug" => Ok(LogLevel::Debug),
        "info" => Ok(LogLevel::Info),
        "warn" => Ok(LogLevel::Warn),
        "error" => Ok(LogLevel::Error),
        "fatal" => Ok(LogLevel::Fatal),
        _ => Err(OptError::Bad(format!("invalid log level: {v}"))),
    }
}

/// 当前实现的协议版本串。对齐 p10k v1.20.0 内嵌 gitstatus 的版本
/// （实测官方二进制为 v1.5.4）：zsh 侧以 build.info 的版本串做 `-G`
/// 校验，p11k 必须声称同版本才能通过，否则直接 exit 11。
pub const PROTOCOL_VERSION: &str = "v1.5.4";

/// `-G` 的 fnmatch 校验：版本串与 glob 匹配即通过，否则返回 false
/// （调用方以 [`EXIT_VERSION_MISMATCH`] 退出）。
///
/// 实现要点：等价 glibc `fnmatch(pattern, version, 0)`（flags=0）——
/// `*`/`?` 可匹配 `/`，`\` 转义生效，无前导 `.` 特殊规则。
/// 版本串只有 `v1.5.5` 这类短串，回溯式递归的复杂度完全可接受。
pub fn version_matches(version: &str, glob: &str) -> bool {
    fnmatch(glob.as_bytes(), version.as_bytes())
}

fn fnmatch(pat: &[u8], text: &[u8]) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => {
            // * 匹配任意长度（含 0 个字符）；flags=0 下也匹配 '/'
            (0..=text.len()).any(|i| fnmatch(rest, &text[i..]))
        }
        Some((b'?', rest)) => !text.is_empty() && fnmatch(rest, &text[1..]),
        Some((b'[', rest)) => match parse_class(rest) {
            // 合法字符类：取文本首字符判定后继续
            Some((class, consumed)) => {
                let Some(&c) = text.first() else { return false };
                class.matches(c) && fnmatch(&rest[consumed..], &text[1..])
            }
            // '[' 后不是合法字符类（缺右括号等）：按字面量 '[' 处理（glibc 行为）
            None => text.first() == Some(&b'[') && fnmatch(rest, &text[1..]),
        },
        Some((b'\\', rest)) => match rest.split_first() {
            // \x 匹配字面量 x；模式以 \ 结尾时 \ 按字面量处理（glibc 行为）
            Some((c, rest2)) => text.first() == Some(c) && fnmatch(rest2, &text[1..]),
            None => text == *b"\\",
        },
        Some((c, rest)) => text.first() == Some(c) && fnmatch(rest, &text[1..]),
    }
}

/// 解析 '[' 之后的字符类内容（`pat` 是 '[' 之后的切片）。
/// 返回（类定义、消费字节数：含闭合 ']'）；不合法返回 None。
struct CharClass {
    /// 成员 (lo, hi) 闭区间。
    ranges: Vec<(u8, u8)>,
    /// `[!...]` / `[^...]` 取反（`^` 是 GNU 扩展，glibc 支持）。
    negate: bool,
}

impl CharClass {
    fn matches(&self, c: u8) -> bool {
        let hit = self.ranges.iter().any(|&(lo, hi)| c >= lo && c <= hi);
        hit != self.negate
    }
}

fn parse_class(pat: &[u8]) -> Option<(CharClass, usize)> {
    let mut i = 0;
    let negate = matches!(pat.first(), Some(b'!') | Some(b'^'));
    if negate {
        i += 1;
    }
    let mut ranges: Vec<(u8, u8)> = Vec::new();
    let mut closed = false;
    while i < pat.len() {
        let c = pat[i];
        if c == b']' && !ranges.is_empty() {
            closed = true;
            i += 1;
            break;
        }
        if c == b']' {
            // "[]..."：']' 紧跟 '['/'[!' 后是字面量成员而非终结符
            ranges.push((b']', b']'));
        } else if c == b'\\' && i + 1 < pat.len() {
            // 类内转义
            i += 1;
            ranges.push((pat[i], pat[i]));
        } else if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            // 范围 a-z；'-' 在开头/结尾按字面量
            ranges.push((c, pat[i + 2]));
            i += 2;
        } else {
            ranges.push((c, c));
        }
        i += 1;
    }
    if !closed {
        return None;
    }
    Some((CharClass { ranges, negate }, i))
}
