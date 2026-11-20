//! Engine startup: load the theme, choose the shell, open the pty and start the inner shell.

use std::io::{Read, Write};
use std::os::fd::RawFd;
use std::path::PathBuf;
use std::process;

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

use crate::cli::{Shell, detect_shell, load_config, theme_file};
use crate::config::Config;
use crate::geometry::Geometry;
use crate::i18n::{t, t_args};
use crate::log::info;
use crate::render::InputPrefix;
use crate::state::{RawTerminal, StateDir, tty_size};

/// The parts of a started engine that the event loop needs.
pub struct Session {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub prefix: InputPrefix,
    pub geometry: Geometry,
    pub state: StateDir,
    pub shell: Shell,
    pub master: Box<dyn MasterPty + Send>,
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub master_fd: RawFd,
    pub size: (u16, u16, u16, u16),
    pub raw: RawTerminal,
}

impl Session {
    /// Load the theme, start the inner shell and return what the event loop needs.
    pub fn start() -> anyhow::Result<Self> {
        // The config comes first: it may name the shell, and it carries the frame and the input-line
        // prefix the templates hand to the shell.
        let mut config = load_config();
        match config.apply_locale_fallback() {
            Ok(warnings) => {
                for warning in warnings {
                    eprintln!("p11k: {warning}");
                }
            }
            Err(error) => {
                eprintln!(
                    "p11k: {}",
                    t_args("config-cannot-render", &[("error", &error)])
                );
                process::exit(1);
            }
        }
        let shell = detect_shell(&config);
        warn_on_degraded_terminal(shell);
        // The file the theme came from, kept for `p11k reload` (None for a preset or the built-in
        // theme; the engine then has nothing to re-read). Made absolute, because the reload
        // command reads it from wherever the shell happens to be, not from the starting directory.
        // A path that cannot be resolved is kept as it is: reloading then reports the real error.
        let config_path = theme_file().map(|p| p.canonicalize().unwrap_or(p));
        let prefix = crate::render::input_prefix(&config, None, crate::model::ViMode::Unreported);
        // The prompt skeleton each shell prints: `header_rows` blank rows for the header, then the
        // zero-width prompt-ready signal. Handed over as data so no template carries it. The engine
        // fills the blank rows when the signal arrives; the prefix travels in the same prompt
        // string, which keeps its styling in the theme and its width unconstrained. Counting the
        // header rows inside the prompt is also what lets zsh fold transient cleanly with
        // `reset-prompt`.
        //
        // The signal has to be written by the shell: each shell marks non-printing sequences
        // differently (`%{...%}` in zsh, `\[...\]` in bash), and the engine handing over a bare OSC
        // would make the line editor count it as visible width.
        let geometry = Geometry::new(&config, shell);
        let state = StateDir::create(shell)?;
        // Keep the theme file this session uses where `p11k reload` can read it. An empty value
        // records a session without a theme file (a preset or the built-in theme).
        state.write_current_config(config_path.as_deref())?;
        info(&format!(
            "engine start: dir={} announce={} ack={} header_rows={} shell={:?}",
            state.dir.display(),
            state.announce.display(),
            state.ack.display(),
            geometry.header_rows,
            shell
        ));

        // The real terminal (the pty stdin lives on) must be in raw mode: turn off
        // ISIG/ICANON/ECHO so every byte is passed through to the shell in the pty. Otherwise
        // ^C is turned into SIGINT by the kernel on the real terminal side and kills the engine
        // (the shell never sees it), and the kernel also echoes input, doubling every character.
        // The shell's own terminal (portable-pty's slave) has its termios managed by the shell.
        let raw = RawTerminal::enter(libc::STDIN_FILENO)?;

        let (rows, cols, xpix, ypix) = tty_size().unwrap_or((24, 80, 0, 0));
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows,
            cols,
            pixel_width: xpix,
            pixel_height: ypix,
        })?;

        let mut cmd = CommandBuilder::new(shell.name());
        match shell {
            // zsh: ZDOTDIR points at the engine dir, which holds the .zshrc it reads.
            Shell::Zsh => {
                cmd.env("ZDOTDIR", &state.dir);
            }
            // bash: --rcfile selects the protocol layer (only interactive bash reads rcfile).
            Shell::Bash => {
                cmd.arg("--rcfile");
                cmd.arg(&state.rc);
            }
            Shell::Fish => {
                // fish redirects via XDG_CONFIG_HOME and reads $XDG_CONFIG_HOME/fish/config.fish.
                cmd.env("XDG_CONFIG_HOME", &state.dir);
            }
            Shell::Pwsh => {
                // pwsh has no --rcfile/ZDOTDIR: -NoProfile disables the default profile and
                // -Command dot-sources the engine's protocol script; -NoExit keeps it interactive.
                cmd.arg("-NoProfile");
                cmd.arg("-NoLogo");
                cmd.arg("-NoExit");
                cmd.arg("-Command");
                cmd.arg(format!(". '{}'", state.rc.display()));
            }
        }
        cmd.env("P11K_ANNOUNCE", &state.announce);
        cmd.env("P11K_ACK", &state.ack);
        // Set while the instant header is on screen. The first prompt prints its own header rows;
        // this flag keeps the instant-phase resize path and the relaxed-layout blank rows separate
        // from the first prompt.
        if config.instant_prompt {
            cmd.env("P11K_INSTANT", "1");
        }
        // Both states of the input-line prefix. zle/readline draw the user's buffer immediately after
        // the prompt string, so the prefix has to be part of it — painting it afterwards would
        // overwrite the front of the buffer on every completion redraw.
        cmd.env("P11K_PREFIX_OK", &geometry.prefix_ok);
        cmd.env("P11K_PREFIX_ERR", &geometry.prefix_err);
        // `p11k reload` recomputes the geometry of a new theme and compares it with what this shell
        // was handed; it needs the shell to pick the right prefix syntax.
        cmd.env("P11K_SHELL", shell.name());
        // `p11k reload` runs in the inner shell: it needs to find this engine and the file the
        // theme came from. An empty P11K_CONFIG means the theme is a preset or the built-in one,
        // and there is nothing to re-read.
        cmd.env("P11K_ENGINE_PID", process::id().to_string());
        // `p11k reload` proves it talks to this session, not to whatever process now owns the old
        // pid: the token here matches the one in `session`, and the session directory holds the
        // current theme path and the reload request.
        cmd.env("P11K_ENGINE_TOKEN", &state.token);
        cmd.env("P11K_ENGINE_SESSION", &state.dir);
        if let Some(path) = &config_path {
            cmd.env("P11K_CONFIG", path);
        } else {
            cmd.env_remove("P11K_CONFIG");
        }
        // portable-pty's spawn_command defaults current_dir to HOME; set the engine's startup
        // cwd explicitly so the inner shell lands where the user ran `exec p11k`.
        cmd.cwd(std::env::current_dir()?);
        // Recursion flag: if the inner shell or a child execs p11k again, the entry check
        // degrades it.
        cmd.env("P11K_ENGINE", "1");
        // transient is zsh-only: hand the shell the single-line prompt the engine precomputes
        // (zsh %F escapes); zle-line-finish swaps PROMPT + reset-prompt to fold it in sync.
        if config.layout.transient_prompt.enabled() && shell == Shell::Zsh {
            cmd.env(
                "P11K_TRANSIENT_PROMPT",
                crate::render::transient_prompt_zsh(&config),
            );
        } else {
            cmd.env("P11K_TRANSIENT_PROMPT", "");
        }

        // double-fork orphaning: the inner shell leaves the engine process tree (its parent
        // becomes init). When closing a window kitty looks at the descendants of its child; an
        // orphan is not in that tree → no "confirm close" dialog. The engine still reads and
        // writes the master fd (orphaning does not break it); when the engine exits, master
        // closes → slave hangs up → the inner shell gets SIGHUP and exits, so there is no PID
        // to record and kill.
        let mid = unsafe { libc::fork() };
        if mid < 0 {
            anyhow::bail!("fork failed: {}", std::io::Error::last_os_error());
        }
        if mid == 0 {
            // Middle process: spawn the inner shell (parent = this process), then exit at once
            // so it is orphaned. Only async-signal-safe work is allowed here: the engine is
            // multi-threaded, so calling malloc after fork (println and the translation lookup
            // both do) risks deadlock. The error code goes back to the parent to print.
            let status = match pair.slave.spawn_command(cmd) {
                Ok(_) => 0,
                Err(_) => 1,
            };
            unsafe { libc::_exit(status) };
        }
        // Engine: reap the middle process and drop the slave reference (the inner shell has
        // taken it over).
        let mut _st = 0;
        unsafe { libc::waitpid(mid, &mut _st, 0) };
        // A non-zero middle process means the inner shell failed to start. Continuing would run an
        // engine with no shell: no prompt and no commands, with only "the shell produced no output"
        // upstream. Report the failure and exit instead of carrying on.
        if _st != 0 {
            anyhow::bail!("{}", t("engine-inner-shell-failed"));
        }
        drop(pair.slave);

        let reader = pair.master.try_clone_reader()?;
        let master_fd = pair
            .master
            .as_raw_fd()
            .expect("pty master fd is available on unix");
        let writer = pair.master.take_writer()?;

        Ok(Self {
            config,
            config_path,
            prefix,
            geometry,
            state,
            shell,
            master: pair.master,
            reader,
            writer,
            master_fd,
            size: (rows, cols, xpix, ypix),
            raw,
        })
    }
}

/// zsh, pwsh and fish consult terminfo when they redraw the prompt. With no usable entry the
/// prompt rows can be repainted in the wrong place and colours lose their frame sequence. bash's
/// readline kept the input and the redraw correct in the same test, so it stays quiet. An empty
/// or `dumb` TERM is the heuristic for a missing entry; a set TERM whose terminfo entry is
/// missing is not detected.
fn terminal_is_usable(shell: Shell, term: Option<&str>) -> bool {
    if shell == Shell::Bash {
        return true;
    }
    matches!(term, Some(value) if !value.is_empty() && !value.eq_ignore_ascii_case("dumb"))
}

fn warn_on_degraded_terminal(shell: Shell) {
    let term = std::env::var("TERM").ok();
    if !terminal_is_usable(shell, term.as_deref()) {
        let name = term
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unset".into());
        eprintln!(
            "p11k: {}",
            t_args("engine-terminal-degraded", &[("term", &name)])
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zle, PSReadLine and fish consult terminfo for prompt redraws, so they warn.
    #[test]
    fn shells_that_need_terminfo_warn_when_it_is_missing() {
        for shell in [Shell::Zsh, Shell::Pwsh, Shell::Fish] {
            assert!(!terminal_is_usable(shell, None));
            assert!(!terminal_is_usable(shell, Some("")));
            assert!(!terminal_is_usable(shell, Some("dumb")));
            assert!(!terminal_is_usable(shell, Some("DUMB")));
            assert!(terminal_is_usable(shell, Some("xterm-256color")));
        }
    }

    /// bash kept its input and redraw correct in the same test, so it stays quiet.
    #[test]
    fn bash_keeps_its_own_redraw() {
        assert!(terminal_is_usable(Shell::Bash, None));
        assert!(terminal_is_usable(Shell::Bash, Some("dumb")));
    }
}
