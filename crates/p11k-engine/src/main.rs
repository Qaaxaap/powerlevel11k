//! p11k engine: pty host + terminal rendering layer.
//!
//! Chain of responsibility:
//! 1. Start a themeless shell with portable-pty (ZDOTDIR points at a temp dir the engine
//!    generates; .zshrc sets only the prompt skeleton and the announce hooks).
//! 2. Pass through: pty output → real terminal stdout; real terminal stdin → pty. The
//!    engine does not parse ANSI from the pty: command output, zle, the prompt string and
//!    completion menus are all rendered by the shell, untouched, except that the
//!    zero-width prompt-ready signal is stripped out.
//! 3. The prompt window is painted in two announce-driven strokes:
//!    - `h` (precmd announce): emit the OSC133A marker + touch ack to let the shell print the
//!      prompt string; the header is not drawn here.
//!    - the prompt-ready signal, written by the shell inside the prompt string: move up over
//!      the header's blank rows and backfill the real header. The prefix already travelled in
//!      that same string, so the line editor drew it with the buffer in place.
//!
//! Geometry protocol: the ack the engine writes on each precmd carries the header height, and the
//! shell prints that many blank rows followed by the zero-width `OSC 1337;p11k;p` signal and
//! `$P11K_PREFIX_OK`/`$P11K_PREFIX_ERR`, so the prompt geometry counts the header rows too and a
//! reload can change their number.
//! precmd writes announce then polls ack, ensuring the prompt string is printed first and
//! the header backfilled after.
//!
//! Deployment: add the exec launch to the user rc and use P11K_ENGINE to avoid recursion
//! `[[ -z "$P11K_ENGINE" ]] && exec p11k --shell zsh` (see README).
//! - Normal (P11K_ENGINE unset): set `P11K_ENGINE=1` when spawning the inner shell;
//! - Recursive (P11K_ENGINE set): an unguarded bootstrap line in the user rc recursed;
//!   start a clean shell and print a repair hint without loading the user rc.
//!
//! The inner shell is orphaned via double-fork (it leaves the engine process tree). When
//! the engine exits, master closes → slave hangs up → the inner shell gets SIGHUP and
//! exits. While drawing header/prefix the engine emits OSC 133 A/B prompt markers; GUI
//! close confirmation uses them to tell the cursor is at the prompt and stops asking.

mod cli;
mod config;
mod dir_shorten;
mod engine;
mod geometry;
mod git;
mod i18n;
mod log;
mod model;
mod paths;
mod pattern;
mod presets;
mod protocol;
mod render;
mod session;
mod state;
mod templates;
mod theme;
mod wake;
mod window;
mod wizard;

use cli::{Command, apply_log_level, parse, reload_engine};
use i18n::{t, t_args};

/// The shell to fall back to after a recursive launch: `$SHELL`, or zsh when it is unset.
fn clean_shell() -> std::path::PathBuf {
    std::env::var_os("SHELL")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "zsh".into())
}

/// The user's shell started without its startup files, so the recursive bootstrap line in the
/// rc cannot run again. Each shell spells "skip the startup files" differently.
fn clean_shell_command(shell: &std::path::Path) -> std::process::Command {
    let mut cmd = std::process::Command::new(shell);
    match shell.file_name().and_then(|n| n.to_str()).unwrap_or("") {
        "bash" => {
            cmd.arg("--noprofile");
            cmd.arg("--norc");
        }
        "fish" => {
            cmd.arg("--no-config");
        }
        "pwsh" => {
            cmd.arg("-NoProfile");
            cmd.arg("-NoLogo");
        }
        // zsh and any other shell that follows `sh`'s `-f`.
        _ => {
            cmd.arg("-f");
        }
    }
    cmd
}

fn main() -> anyhow::Result<()> {
    // Rust starts every program with SIGPIPE ignored, so a write to a closed pipe comes back
    // as EPIPE and `println!` panics: `p11k --help | head -3` printed a backtrace instead of
    // just stopping. Dying on the signal is what every other Unix tool does, and the engine's
    // own stdout is a terminal, which is not affected either way.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    // Text is translated by locale (English by default; Chinese in locales/zh-CN.ftl).
    i18n::init();
    // clap parses once here. `--version` and `--help` are handled before any terminal state is
    // touched, so they work in pipes and environments without a tty (CI, version checks).
    let cli = parse();
    // Rotate an oversized diagnostics log before the engine starts appending to it, then apply
    // `--log-level` (the default is info).
    log::init();
    apply_log_level();
    // `p11k configure`: enter the interactive theme wizard without spawning a shell.
    // `p11k reload`: ask the engine of this session to re-read a theme file.
    match &cli.command {
        Some(Command::Configure) => return wizard::run(),
        Some(Command::Reload { config }) => return reload_engine(config.clone()),
        None => {}
    }

    // Recursion check: P11K_ENGINE set means this engine is a redundant instance started
    // again by the inner shell's rc bootstrap (the user rc bootstrap line is unguarded or
    // wrong). Degrade to exec'ing a clean shell and tell the user to fix it.
    if std::env::var_os("P11K_ENGINE").is_some() {
        let shell = clean_shell();
        let name = shell
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| shell.display().to_string());
        eprintln!(
            "p11k: {}\n\
             p11k: {}\n\
             p11k:   [[ -z \"$P11K_ENGINE\" ]] && exec p11k --shell {}",
            t("main-recursive-launch"),
            t("main-bootstrap-hint"),
            name
        );
        use std::os::unix::process::CommandExt;
        let err = clean_shell_command(&shell).exec();
        eprintln!(
            "p11k: {}",
            t_args("main-cannot-exec-shell", &[("detail", &err.to_string())])
        );
        std::process::exit(1);
    }

    engine::run()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args_for(shell: &str) -> Vec<String> {
        let cmd = clean_shell_command(std::path::Path::new(shell));
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_clean_fallback_skips_each_shells_startup_files() {
        assert_eq!(args_for("/bin/zsh"), ["-f"]);
        assert_eq!(args_for("zsh"), ["-f"]);
        assert_eq!(args_for("/usr/bin/bash"), ["--noprofile", "--norc"]);
        assert_eq!(args_for("/usr/bin/fish"), ["--no-config"]);
        assert_eq!(args_for("/usr/bin/pwsh"), ["-NoProfile", "-NoLogo"]);
        // An unknown shell gets the portable `-f`.
        assert_eq!(args_for("/usr/local/bin/dash"), ["-f"]);
    }

    #[test]
    fn the_clean_fallback_runs_the_requested_program() {
        let cmd = clean_shell_command(std::path::Path::new("/usr/bin/fish"));
        assert_eq!(cmd.get_program(), "/usr/bin/fish");
    }
}
