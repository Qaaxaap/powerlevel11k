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
mod presets;
mod protocol;
mod render;
mod state;
mod templates;
mod theme;
mod window;
mod wizard;

use cli::{print_help, reload_engine};
use i18n::t;

fn main() -> anyhow::Result<()> {
    // Rust starts every program with SIGPIPE ignored, so a write to a closed pipe comes back
    // as EPIPE and `println!` panics: `p11k --help | head -3` printed a backtrace instead of
    // just stopping. Dying on the signal is what every other Unix tool does, and the engine's
    // own stdout is a terminal, which is not affected either way.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    // Text is translated by locale (English by default; Chinese in po/zh_CN.po).
    i18n::init();
    // `--version`/`--help` are handled before touching any terminal state so they work in
    // pipes and environments without a tty (CI, version checks from scripts).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("p11k {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--help" || a == "-h") {
        print_help();
        return Ok(());
    }
    // `p11k configure`: enter the interactive theme wizard without spawning a shell.
    if std::env::args().any(|a| a == "configure") {
        return wizard::run();
    }
    // `p11k reload`: ask the engine of this session to re-read its theme file.
    if std::env::args().any(|a| a == "reload") {
        return reload_engine();
    }

    // Recursion check: P11K_ENGINE set means this engine is a redundant instance started
    // again by the inner shell's rc bootstrap (the user rc bootstrap line is unguarded or
    // wrong). Degrade to exec'ing a clean shell and tell the user to fix it.
    if std::env::var_os("P11K_ENGINE").is_some() {
        eprintln!(
            "p11k: {}\n\
             p11k: {}\n\
             p11k:   [[ -z \"$P11K_ENGINE\" ]] && exec p11k --shell zsh",
            t("recursive launch detected (p11k started inside a p11k session)"),
            t("make sure the bootstrap line in ~/.zshrc is guarded, e.g.:")
        );
        use std::os::unix::process::CommandExt;
        let err = std::process::Command::new("zsh").arg("-f").exec();
        eprintln!("p11k: {}{err}", t("cannot exec zsh -f: "));
        std::process::exit(1);
    }

    engine::run()
}
