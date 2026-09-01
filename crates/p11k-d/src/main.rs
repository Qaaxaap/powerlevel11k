//! p11k-d — the p11k resident daemon.
//!
//! Runs in gitstatusd-compatible mode: parses the full gitstatus argument
//! set and serves the request/response loop, so existing powerlevel10k
//! `gitstatus.plugin.zsh` can use it as gitstatusd via `GITSTATUS_DAEMON`.
//!
//! The pgid handshake is done by the zsh side before exec'ing this process;
//! the daemon inherits stdin (FIFO read end) and stdout (pipe write end) and
//! goes straight into the main loop. stdout is reserved for protocol
//! responses; all logging goes to stderr.
//!
//! # Exit codes
//!
//! | Code | Meaning |
//! |---|---|
//! | 0 | Normal (EOF / liveness failed) |
//! | 10 | Bad arguments |
//! | 11 | `-G` version mismatch |

use p11k_gitstatus::daemon::Daemon;
use p11k_gitstatus::options::{
    EXIT_BAD_ARGS, EXIT_VERSION_MISMATCH, Options, PROTOCOL_VERSION, ParseError, ParseOutcome,
    parse_args,
};
use std::process::ExitCode;

/// Mirrors options.cc PrintUsage (wording need not match verbatim).
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
    daemon.run(); // exits with 0 inside; reaching here is dead code
    ExitCode::SUCCESS
}
