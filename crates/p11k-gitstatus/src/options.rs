//! Command-line arguments (mirrors gitstatusd `options.cc:62-243` /
//! `options.h:29-72`).
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | Success, or normal EOF exit |
//! | 10 | Bad arguments (usage printed, then exit) |
//! | 11 | `-G` version glob mismatch |
//!
//! The zsh side passes `-s/-u/-c/-d/-m/-e/-U/-W/-D` straight through and
//! appends `-t 2*cpu` (capped at 32) by default. Parsing must accept any
//! order, both `--long` and `-x`, and both `-xVALUE` and `-x VALUE`.

/// Option dictionary. Defaults must match the original exactly.
#[derive(Debug, Clone)]
pub struct Options {
    /// `-l/--lock-fd=N`: lock fd liveness (default -1 = disabled). On each
    /// poll timeout the daemon checks the lock is still held via
    /// `fcntl(F_GETLK)`; if not, the parent died and the daemon exits.
    pub lock_fd: i32,
    /// `-p/--parent-pid=N`: parent liveness (default -1). Exits when
    /// `kill(pid, 0)` fails on timeout. bash passes `--parent-pid=$$`; zsh
    /// relies on EOF + kill instead.
    pub parent_pid: i32,
    /// `-t/--num-threads=N`: worktree scan threads (default 1; zsh sets
    /// 2*cpu, capped at 32).
    pub num_threads: usize,
    /// `-v/--log-level`: log level.
    pub log_level: LogLevel,
    /// `-r/--repo-ttl-seconds`: LRU close time for idle repos
    /// (default 3600; negative = never expire).
    pub repo_ttl_seconds: i64,
    /// `-z/--max-commit-summary-length`: commit summary truncation in bytes
    /// (default 256).
    pub max_commit_summary_length: usize,
    /// `-s`: staged count cap (default 1; negative = unlimited).
    pub max_num_staged: i64,
    /// `-u`: unstaged count cap (default 1; negative = unlimited).
    pub max_num_unstaged: i64,
    /// `-c`: conflicted count cap (default 1; negative = unlimited).
    pub max_num_conflicted: i64,
    /// `-d`: untracked count cap (default 1; negative = unlimited).
    pub max_num_untracked: i64,
    /// `-m/--dirty-max-index-size`: report zero dirty files when the index
    /// exceeds this many entries (default -1 = disabled).
    pub dirty_max_index_size: i64,
    /// `-e`: recurse into untracked directories (default reports the
    /// directory itself only).
    pub recurse_untracked_dirs: bool,
    /// `-U`: ignore `status.showUntrackedFiles`.
    pub ignore_status_show_untracked_files: bool,
    /// `-W`: ignore `bash.showUntrackedFiles`.
    pub ignore_bash_show_untracked_files: bool,
    /// `-D`: ignore `bash.showDirtyState`.
    pub ignore_bash_show_dirty_state: bool,
    /// `-G/--version-glob`: version fnmatch pattern; exit 11 on mismatch.
    /// The zsh side passes the build.info version (e.g. `v1.5.5`).
    pub version_glob: Option<String>,
}
impl Default for Options {
    /// All defaults, matching `options.cc:58-79`.
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

/// Log level (`-v` values, case-insensitive).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}
impl LogLevel {
    /// Maps to libgit2's `git_trace_level_t`.
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

/// Success or normal EOF exit.
pub const EXIT_OK: i32 = 0;
/// Bad arguments.
pub const EXIT_BAD_ARGS: i32 = 10;
/// `-G` version glob mismatch.
pub const EXIT_VERSION_MISMATCH: i32 = 11;

/// Parse command-line arguments (excluding argv[0]).
///
/// `Err(BadArgs)` → caller prints the error and usage, exits with
/// [`EXIT_BAD_ARGS`]; `Err(VersionMismatch)` → exits with
/// [`EXIT_VERSION_MISMATCH`].
///
/// Notes:
/// - Supports `-x` / `--long` / `-xVALUE` / `-x VALUE` / `--long=VALUE`.
///   Like the original getopt, a short option's value may be the next arg.
/// - `-h`/`-V` take the [`ParseOutcome::Help`]/[`ParseOutcome::Version`]
///   fast paths.
/// - Boolean switches (`-e/-U/-W/-D` and their long forms) take no value.
/// - Numeric parse failures are argument errors (exit 10), never panics.
/// - `-G` validates the version during parsing (mismatch → exit 11).
/// - GNU getopt features (option permutation, `--`) are unsupported: the
///   zsh side never passes positional args.
#[derive(Debug)]
pub enum ParseOutcome {
    /// User asked for help (`-h`/`--help`); main prints usage, exits 0.
    Help,
    /// User asked for the version (`-V`/`--version`); main prints it, exits 0.
    Version,
    /// Parsing done; enter the daemon main loop.
    Run(Options),
}

/// Argument parse errors, mapped to exit codes.
#[derive(Debug)]
pub enum ParseError {
    /// Argument error → [`EXIT_BAD_ARGS`].
    BadArgs(String),
    /// `-G` version glob mismatch → [`EXIT_VERSION_MISMATCH`].
    VersionMismatch {
        /// The glob pattern the user passed.
        pattern: String,
    },
}

/// Internal error shared by short/long option parsing;
/// [`ParseError::VersionMismatch`] can only come from the `-G` branch.
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
            // Boolean long options (--recurse-untracked-dirs etc.) take no value.
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
                // -xVALUE form (-t4, -Gv1.5.5); boolean options with a value
                // are rejected at parse time.
                parse_short_opt(&mut options, name, Some(&arg[2..]))?;
            } else if short_opt_takes_value(name) {
                // -x VALUE form: value is the next argument.
                i += 1;
                if i >= args.len() {
                    return Err(ParseError::BadArgs(format!(
                        "option -{name} requires an argument"
                    )));
                }
                parse_short_opt(&mut options, name, Some(&args[i]))?;
            } else {
                // Boolean short option (-e/-U/-W/-D).
                parse_short_opt(&mut options, name, None)?;
            }
        } else {
            return Err(ParseError::BadArgs(format!(
                "unexpected positional argument: {arg}"
            )));
        }
        i += 1;
    }
    Ok(ParseOutcome::Run(options))
}

/// Whether the short option takes a value. Value-taking: l p t v r z s u c d
/// m G; boolean: e U W D.
fn short_opt_takes_value(name: &str) -> bool {
    matches!(
        name,
        "l" | "p" | "t" | "v" | "r" | "z" | "s" | "u" | "c" | "d" | "m" | "G"
    )
}

/// Whether the long option takes a value. Boolean longs (corresponding to
/// -e/-U/-W/-D) return false.
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
                // Matches options.cc: num_threads must be > 0.
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
            // Validate during parsing (options.cc case 'G'): mismatch is
            // exit 11, not 10.
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
            // Same as -G: validate during parsing.
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

/// Value-taking options: None means missing arg, parse failure means
/// "not an integer". Mirrors the original `strtol` (ParseLong): leading
/// whitespace is allowed (`" 4"`), trailing garbage is rejected (`"4x"`).
/// Rust's `parse` rejects leading whitespace, so trim first.
fn parse_int<T: std::str::FromStr>(value: Option<&str>, opt: &str) -> Result<T, OptError> {
    let v = value.ok_or_else(|| OptError::Bad(format!("option {opt} requires an argument")))?;
    v.trim()
        .parse()
        .map_err(|_| OptError::Bad(format!("not an integer: {v}")))
}

/// Mirrors ParseSizeT (options.cc:56-59): parse as i64, map negatives to -1
/// (the original stores size_t, where -1 = SIZE_MAX).
fn parse_limit(value: Option<&str>, opt: &str) -> Result<i64, OptError> {
    let n: i64 = parse_int(value, opt)?;
    Ok(if n < 0 { -1 } else { n })
}

/// Mirrors ParseSizeT's size_t semantics: negatives map to usize::MAX
/// (`-z` summary length cap; negative = no truncation).
fn parse_size(value: Option<&str>, opt: &str) -> Result<usize, OptError> {
    let n: i64 = parse_int(value, opt)?;
    Ok(if n < 0 { usize::MAX } else { n as usize })
}

/// Boolean short options reject a value (`-efoo` is an error, not ignored).
fn reject_value(value: Option<&str>, opt: &str) -> Result<(), OptError> {
    match value {
        Some(_) => Err(OptError::Bad(format!(
            "option {opt} does not take an argument"
        ))),
        None => Ok(()),
    }
}

/// Defensive re-check for boolean long options; the main loop already
/// intercepts `--flag=value`.
fn no_value_ok(value: Option<&str>, opt: &str) -> Result<(), OptError> {
    match value {
        Some(_) => Err(OptError::Bad(format!(
            "option {opt} does not take an argument"
        ))),
        None => Ok(()),
    }
}

/// String value (`-G` only).
fn take_value<'a>(value: Option<&'a str>, opt: &str) -> Result<&'a str, OptError> {
    value.ok_or_else(|| OptError::Bad(format!("option {opt} requires an argument")))
}

/// `-v` values, case-insensitive: debug/info/warn/error/fatal.
/// Matches the original error wording "invalid log level: X".
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

/// The protocol version this daemon claims. Must match the version the zsh
/// side passes to `-G` (from build.info, measured v1.5.5 in the user's
/// environment), otherwise it exits 11. Note p10k release v1.20.0's
/// build.info may say v1.5.4; the claimed version matches only one
/// build.info (the original binary has the same constraint).
pub const PROTOCOL_VERSION: &str = "v1.5.5";

/// `-G` fnmatch check: the version matches the glob, or the caller exits
/// with [`EXIT_VERSION_MISMATCH`]. Equivalent to glibc
/// `fnmatch(pattern, version, 0)` (flags=0): `*`/`?` match `/`, `\`
/// escapes, no leading-dot special-casing. Version strings are short
/// (`v1.5.5`), so backtracking recursion is fine.
pub fn version_matches(version: &str, glob: &str) -> bool {
    fnmatch(glob.as_bytes(), version.as_bytes())
}

fn fnmatch(pat: &[u8], text: &[u8]) -> bool {
    match pat.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => {
            // * matches any length (including 0); with flags=0 it also
            // matches '/'.
            (0..=text.len()).any(|i| fnmatch(rest, &text[i..]))
        }
        Some((b'?', rest)) => !text.is_empty() && fnmatch(rest, &text[1..]),
        Some((b'[', rest)) => match parse_class(rest) {
            // Valid class: match the first text byte, then continue.
            Some((class, consumed)) => {
                let Some(&c) = text.first() else { return false };
                class.matches(c) && fnmatch(&rest[consumed..], &text[1..])
            }
            // '[' without a valid class (e.g. no closing bracket) is a
            // literal '[' (glibc behavior).
            None => text.first() == Some(&b'[') && fnmatch(rest, &text[1..]),
        },
        Some((b'\\', rest)) => match rest.split_first() {
            // \x matches literal x; a trailing \ in the pattern is literal
            // (glibc behavior).
            Some((c, rest2)) => text.first() == Some(c) && fnmatch(rest2, &text[1..]),
            None => text == *b"\\",
        },
        Some((c, rest)) => text.first() == Some(c) && fnmatch(rest, &text[1..]),
    }
}

/// Parse a character class following '[' (`pat` is the slice after '[').
/// Returns (class, bytes consumed incl. closing ']'); None if invalid.
struct CharClass {
    /// Members as closed (lo, hi) ranges.
    ranges: Vec<(u8, u8)>,
    /// `[!...]` / `[^...]` negation (`^` is a GNU extension, glibc supports it).
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
            // "[]...": ']' right after '[' / '[!' is a literal member, not
            // the terminator.
            ranges.push((b']', b']'));
        } else if c == b'\\' && i + 1 < pat.len() {
            // Escape inside the class.
            i += 1;
            ranges.push((pat[i], pat[i]));
        } else if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            // Range a-z; '-' at start/end is literal.
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
