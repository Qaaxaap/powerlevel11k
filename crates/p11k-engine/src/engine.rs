//! The engine itself: start the inner shell in a pty and run the event loop that paints the
//! prompt window.
//!
//! The crate-level overview lives in `main.rs`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

/// Real terminal SIGWINCH (resize) flag: the handler only sets it (signal-safe); the main
/// loop then syncs the inner pty size → the inner zsh gets SIGWINCH → TRAPWINCH announces r
/// → the engine clears and repaints the prompt window.
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_FLAG.store(true, Ordering::Relaxed);
}

/// `p11k reload` flag: the command signals this engine and the main loop re-reads the theme
/// file. Set from the handler, acted on by the loop (see `AnnMsg` handling).
static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigusr1(_sig: libc::c_int) {
    RELOAD_FLAG.store(true, Ordering::Relaxed);
}

use crate::cli::{Shell, detect_shell, load_config, load_config_file, theme_file};
use crate::geometry::Geometry;
use crate::git::{GitRequest, GitResult, spawn_git_worker};
use crate::i18n::t;
use crate::log::{debug, info};
use crate::protocol::{AnnMsg, SignalScanner, drain_announce, is_terminal_reply};
use crate::state::{RawTerminal, StateDir, tty_size};
use crate::theme;
use crate::theme::{GitStatus, HeaderInfo};
use crate::window::paint_prompt;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

pub fn run() -> anyhow::Result<()> {
    // The config comes first: it may name the shell, and it carries the frame and the input-line
    // prefix the templates hand to the shell.
    let mut config = load_config();
    let shell = detect_shell(&config);
    // The file the theme came from, kept for `p11k reload` (None for a preset or the built-in
    // theme; the engine then has nothing to re-read). Made absolute, because the reload
    // command reads it from wherever the shell happens to be, not from the starting directory.
    // A path that cannot be resolved is kept as it is: reloading then reports the real error.
    let config_path = theme_file().map(|p| p.canonicalize().unwrap_or(p));
    let mut prefix = crate::render::input_prefix(&config, None);
    // The prompt skeleton each shell prints: `header_rows` blank rows for the header, then the
    // zero-width prompt-ready signal. Handed over as data so no template carries it. The engine
    // fills the blank rows when the signal arrives; the prefix travels in the same prompt string,
    // which keeps its styling in the theme and its width unconstrained. Counting the header rows
    // inside the prompt is also what lets zsh fold transient cleanly with `reset-prompt`.
    //
    // The signal has to be written by the shell: each shell marks non-printing sequences
    // differently (`%{...%}` in zsh, `\[...\]` in bash), and the engine handing over a bare OSC
    // would make the line editor count it as visible width.
    let mut geometry = Geometry::new(&config, shell);
    let state = StateDir::create(shell)?;
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
    let _raw = RawTerminal::enter(libc::STDIN_FILENO)?;

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
    // Set while the instant header is on screen: the first prompt must not print blank rows for
    // the header, the engine overwrites the instant header in place instead.
    cmd.env("P11K_INSTANT", "1");
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
        anyhow::bail!("fork failed: {}", io::Error::last_os_error());
    }
    if mid == 0 {
        // Middle process: spawn the inner shell (parent = this process), then exit at once
        // so it is orphaned. Only async-signal-safe work is allowed here: the engine is
        // multi-threaded, so calling malloc after fork (println/gettext both do) risks
        // deadlock. The error code goes back to the parent to print.
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
    // A non-zero middle process means the inner shell failed to start. This used to be
    // silent, leaving only "the shell produced no output" upstream with no way to tell a
    // failed spawn from anything else.
    if _st != 0 {
        eprintln!("p11k: {}", t("the inner shell failed to start"));
    }
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader()?;
    let master_fd = pair
        .master
        .as_raw_fd()
        .expect("pty master fd is available on unix");
    let mut writer = pair.master.take_writer()?;

    // Pre-create the announce file so the shell's >> appends never fail.
    File::create(&state.announce)?;

    // Catch SIGWINCH on the real terminal so the engine also notices a resize while the
    // prompt is displayed (sync the inner pty → TRAPWINCH announces r → clear and repaint)
    // instead of waiting for the next prompt.
    unsafe {
        libc::signal(
            libc::SIGWINCH,
            handle_sigwinch as extern "C" fn(libc::c_int) as usize,
        );
        // `p11k reload` wakes the loop the same way a resize does.
        libc::signal(
            libc::SIGUSR1,
            handle_sigusr1 as extern "C" fn(libc::c_int) as usize,
        );
    }

    let mut ann_processed: u64 = 0; // bytes already consumed from the announce file
    let mut last_size = (rows, cols, xpix, ypix);
    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    // Async git status.
    // A worker thread owns the RepoCache and the main loop sends requests / receives
    // results over channels, so a 1-2s first scan of a large repo never stalls the prompt.
    let (mut req_tx, req_rx) = mpsc::channel::<GitRequest>();
    let (res_tx, res_rx) = mpsc::channel::<GitResult>();
    spawn_git_worker(&config, req_rx, res_tx.clone());

    let mut git_gen: u64 = 0; // request sequence number; only the latest result is used
    // Last git status for the current cwd.
    let mut last_vcs: Option<(String, Option<GitStatus>)> = None;
    let mut current_info: Option<HeaderInfo> = None;
    // Whether the cursor is on the input line (after the prompt-ready signal fired, before the
    // user presses enter). Async results repaint only then, otherwise redraw's \e[1A lands on
    // command output.
    let mut at_prompt = false;
    // Time of the last enter, used by the next precmd to compute command duration.
    let mut last_enter: Option<std::time::Instant> = None;
    // Workdir of the previous precmd, for the `same-dir` transient prompt.
    let mut last_prompt_cwd: Option<String> = None;
    // Lines the last header paint wrote. An async git result that changes nothing would paint
    // the same bytes over the same rows, so `redraw_header_cfg` compares against this and skips.
    // Paths that repaint over cleared rows set it to None first.
    let mut last_header: Option<Vec<String>> = None;
    // Defer the prompt repaint after a resize: wait for zle's repainted prompt and buffer to
    // pass through (~60ms) before painting the header, so it is not painted before zle has
    // moved the input line into place.
    let mut resize_prompt_at: Option<std::time::Instant> = None;
    // Whether the user has typed since the current prompt became ready: the empty enter
    // sent to pwsh on shrink runs only while the input line is still empty, otherwise it
    // would submit a half-written command.
    let mut typed_since_prompt = false;
    // Last raw bytes judged to be user input; diagnostics logging only (escaped).
    let mut last_input: Vec<u8> = Vec::new();
    // Time of the last empty enter sent on pwsh's behalf (dragging a window fires SIGWINCH
    // repeatedly, so it must be throttled).
    let mut last_pwsh_nudge: Option<std::time::Instant> = None;

    // instant header: don't wait for the inner shell to finish loading a slow user rc; draw a
    // provisional header from the engine cwd right away so the prompt is visible as soon as the
    // window opens, then swap in the real state after the first inner precmd.
    let instant_info = HeaderInfo {
        exit_code: None,
        cwd: std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| "~".into()),
        exec_seconds: 0.0,
        jobs: 0,
        history: 0,
        pipestatus: Vec::new(),
    };
    // The return value is the display width of each line: if the terminal is narrower than
    // the pty when drawing (a fresh window whose size has not synced yet), content wraps and
    // erasing must convert to actual rows using the column width of that moment, otherwise
    // half a header is left behind.
    theme::render_header_cfg(&mut stdout, cols as usize, &config, &instant_info, None)?;
    // Draw the instant prefix without the save/restore-cursor dance: the shell has not started
    // yet, so restoring the cursor would put it back at column 0 and the user's typing would
    // overwrite the prefix. With a plain `\r` the cursor ends up right after it.
    write!(stdout, "\r{}", prefix.text)?;
    write!(stdout, "\x1b]133;B\x07")?;
    stdout.flush()?;
    let mut instant_drawn = true;
    // What the inner shell passed through during the instant phase. A newline means the rc
    // really printed something; after that the rows we drew have been pushed down.
    let mut instant_newlines = 0usize;

    let mut scanner = SignalScanner::new();

    loop {
        // `p11k reload`: read the theme file again and repaint. A file that no longer parses
        // keeps the theme already in use (the reload command reports that error itself); the
        // git worker is respawned so the vcs properties follow the new theme too.
        if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            match config_path.as_deref().map(load_config_file) {
                Some(Ok(new_config)) => {
                    // The shell re-reads the header height from the ack on every prompt, so a
                    // reload that changes it lands on the next one. The prefix travelled to the
                    // shell at startup and can only change with a restart.
                    config = new_config;
                    prefix = crate::render::input_prefix(&config, None);
                    let new_geometry = Geometry::new(&config, shell);
                    if new_geometry.prefix_ok != geometry.prefix_ok
                        || new_geometry.prefix_err != geometry.prefix_err
                    {
                        info(
                            "reload: the input-line prefix changed; it takes effect after a restart",
                        );
                    }
                    let header_rows_changed = new_geometry.header_rows != geometry.header_rows;
                    geometry = new_geometry;
                    let (tx, rx) = mpsc::channel::<GitRequest>();
                    req_tx = tx;
                    spawn_git_worker(&config, rx, res_tx.clone());
                    info("reload: theme reloaded");
                    // A changed row count only takes effect on the next prompt, so repainting
                    // now would move the cursor by the old amount.
                    if at_prompt && !header_rows_changed {
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        // The theme changed: paint over whatever the old theme left.
                        last_header = None;
                        theme::redraw_header_cfg(
                            &mut stdout,
                            last_size.1 as usize,
                            &config,
                            current_info.as_ref().unwrap_or(&instant_info),
                            vcs,
                            &mut last_header,
                        )?;
                        stdout.flush()?;
                    }
                }
                Some(Err(e)) => info(&format!("reload: keeping the current theme: {e}")),
                None => info("reload: this session has no theme file"),
            }
        }
        // resize signal: sync the inner pty size (including pixels). zsh/bash/fish all
        // register signal hooks (TRAPWINCH / trap WINCH / --on-signal WINCH) that announce
        // `r` on receipt so the engine relayouts at the new width; pwsh cannot hook SIGWINCH
        // and only compares the window size inside its prompt function, so the block below
        // sends it one empty enter on shrink.
        if RESIZE_FLAG.swap(false, Ordering::Relaxed) {
            // Re-enter raw mode: multiplexers like tmux reset the pane pty termios when
            // creating or resizing a pane. Once ECHO is on, what the engine writes to the
            // terminal is echoed back to stdin and forwarded into the pty as input — and a
            // second copy of the header shows up on screen.
            let _ = RawTerminal::enter(libc::STDIN_FILENO);
            let (r, c, xp, yp) = tty_size().unwrap_or(last_size);
            if (r, c, xp, yp) != last_size {
                pair.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: xp,
                    pixel_height: yp,
                })?;
                last_size = (r, c, xp, yp);
                debug(&format!("resized pty to {}x{} ({}x{} px)", r, c, xp, yp));
                // During the instant phase no shell `r` announce triggers a repaint yet, so
                // the screen still holds the copy drawn at the old column count (possibly
                // wrapped, no longer in place); repaint it here proactively. Clear and redraw
                // only when the instant phase produced no other output: after a terminal
                // reflow "move up N rows" is unreliable, and this is the only safe option.
                if instant_drawn && instant_newlines == 0 {
                    write!(stdout, "\x1b[2J\x1b[H")?;
                    theme::render_header_cfg(
                        &mut stdout,
                        c as usize,
                        &config,
                        &instant_info,
                        None,
                    )?;
                    // Same as the initial instant draw: keep the cursor after the prefix instead
                    // of restoring it to column 0.
                    write!(stdout, "\r{}", prefix.text)?;
                    write!(stdout, "\x1b]133;B\x07")?;
                    stdout.flush()?;
                }
                // pwsh cannot hook SIGWINCH and only compares the window size inside its
                // prompt function, so after a shrink the screen still holds the
                // terminal-reflowed old content until the next prompt relayouts it. Send one
                // empty enter on the user's behalf (only while the input line is still empty)
                // so pwsh calls prompt again → announces r → the engine relayouts at the new
                // width.
                if shell == Shell::Pwsh
                    && at_prompt
                    && !typed_since_prompt
                    && last_pwsh_nudge
                        .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(150))
                {
                    debug("resize: nudge pwsh with an empty line so it redraws");
                    let _ = writer.write_all(b"\r");
                    let _ = writer.flush();
                    last_pwsh_nudge = Some(std::time::Instant::now());
                } else if shell == Shell::Pwsh && at_prompt {
                    debug(&format!(
                        "resize: no pwsh nudge (typed={typed_since_prompt}, last_input_len={})",
                        last_input.len()
                    ));
                }
            }
        }

        // prompt window (announce-driven). The announce file is a second channel: a `h` message
        // can arrive while the command output still sits in the pty buffer. Painting the prompt
        // blanks before that output would put them above it, so when the pty already holds data,
        // let the poll below forward it and handle the messages on the next pass.
        // - `h` (precmd announce, shell waiting for ack): draw header, touch ack to release.
        // - `r` (resize announce): repaint the prompt window.
        let pty_ready = {
            let mut probe = libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            (unsafe { libc::poll(&mut probe, 1, 0) }) > 0 && probe.revents & libc::POLLIN != 0
        };
        if !pty_ready {
            for msg in drain_announce(&state.announce, &mut ann_processed) {
                // Sync a size change to the pty and let right alignment use the new width.
                let (r, c, xp, yp) = tty_size().unwrap_or(last_size);
                if (r, c, xp, yp) != last_size {
                    pair.master.resize(PtySize {
                        rows: r,
                        cols: c,
                        pixel_width: xp,
                        pixel_height: yp,
                    })?;
                    last_size = (r, c, xp, yp);
                }
                match msg {
                    AnnMsg::Header(mut info) => {
                        debug(&format!(
                            "h: exit={:?} cwd={:?} jobs={}",
                            info.exit_code, info.cwd, info.jobs
                        ));
                        // Command duration = last enter → this precmd (0 on the first prompt
                        // without last_enter).
                        info.exec_seconds = last_enter
                            .take()
                            .map(|t| t.elapsed().as_secs_f64())
                            .unwrap_or(0.0);
                        at_prompt = false; // new prompt cycle: the next signal paints the input line
                        // The instant header stays flagged until the first signal arrives: that is
                        // where the real header replaces it in place.
                        if !instant_drawn && config.layout.prompt_add_newline > 0 {
                            // Relaxed layout: leave N blank rows between consecutive prompts
                            // (blanked before the header). One CRLF is one blank row; the shell has
                            // already moved the cursor off the command's output row.
                            for _ in 0..config.layout.prompt_add_newline {
                                write!(stdout, "\r\n")?;
                            }
                        }
                        // Emit the prompt-start marker before the shell prints the prompt string;
                        // the header is not drawn here, the signal that follows the blank rows is
                        // what draws it.
                        theme::prompt_start(&mut stdout)?;
                        stdout.flush()?;
                        // Release precmd and hand the shell the prompt shape: the header height on
                        // the first line, then the prefix for this prompt. The engine picks that
                        // prefix because it is the only side that knows the exit code, the pipeline
                        // statuses and the vi mode; a shell choosing between two strings of its own
                        // could not tell `OK_PIPE` from `OK`.
                        let state_name = crate::render::prompt_state(&config, Some(&info));
                        let prefix = Geometry::prefix_for(&config, shell, state_name);
                        // Transient prompt: `same-dir` folds the header only when the command left
                        // the shell where it was (p10k's `always` / `same-dir` split). The engine
                        // decides because it already knows both directories; the shell only reads
                        // the answer.
                        let transient_on = match config.layout.transient_prompt {
                            crate::config::TransientPrompt::Off => false,
                            crate::config::TransientPrompt::Always => true,
                            // The first prompt has no previous directory to compare against, so it
                            // folds the way `always` would.
                            crate::config::TransientPrompt::SameDir => {
                                last_prompt_cwd.as_deref() == Some(info.cwd.as_str())
                                    || last_prompt_cwd.is_none()
                            }
                        };
                        last_prompt_cwd = Some(info.cwd.clone());
                        let mut ack = File::create(&state.ack)?;
                        writeln!(ack, "{}", geometry.header_rows)?;
                        writeln!(ack, "{prefix}")?;
                        writeln!(ack, "{}", u8::from(transient_on))?;
                        // Compute git in the background and repaint the header asynchronously when
                        // it finishes.
                        git_gen += 1;
                        let _ = req_tx.send(GitRequest {
                            generation: git_gen,
                            cwd: info.cwd.clone(),
                        });
                        current_info = Some(info);
                        debug("drew header, acked");
                    }
                    AnnMsg::Resize => {
                        // The size check at the top of the loop already resized the pty.
                        // zsh: zle repaints the prompt string and the backfill goes through the
                        // signal; repaint the header here and defer the prefix by ~60ms (until
                        // zle's repainted prompt and buffer have passed through).
                        // Other shells: after a resize the shell reprints the prompt string from
                        // scratch (with new leading newlines) and the signal path above backfills
                        // it — the new prompt is not out yet, so drawing now would land on the
                        // previous screen and leave it uncovered.
                        if at_prompt {
                            if shell == Shell::Zsh {
                                debug("r: redraw header, defer prompt");
                                let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                                // A resize can reflow the rows under the header, so paint
                                // unconditionally; the signal that follows repaints again on its own.
                                last_header = None;
                                theme::redraw_header_cfg(
                                    &mut stdout,
                                    last_size.1 as usize,
                                    &config,
                                    current_info.as_ref().unwrap_or(&instant_info),
                                    vcs,
                                    &mut last_header,
                                )?;
                                stdout.flush()?;
                                resize_prompt_at = Some(
                                    std::time::Instant::now()
                                        + std::time::Duration::from_millis(60),
                                );
                            } else {
                                debug("r: wait for the shell to reprint the prompt");
                            }
                        } else {
                            debug("r: skip redraw (not at prompt)");
                        }
                    }
                    AnnMsg::VimMode(mode) => {
                        *crate::render::CURRENT_VI_MODE.lock().unwrap() = mode;
                        if at_prompt {
                            debug("v: update mode, redraw header");
                            let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                            theme::redraw_header_cfg(
                                &mut stdout,
                                last_size.1 as usize,
                                &config,
                                current_info.as_ref().unwrap_or(&instant_info),
                                vcs,
                                &mut last_header,
                            )?;
                            stdout.flush()?;
                        }
                    }
                }
            }
        }

        let mut fds = [
            libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: master_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // 5ms timeout, which doubles as the announce file polling cadence.
        let n = unsafe { libc::poll(fds.as_mut_ptr(), 2, 5) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err.into());
        }

        // Real terminal input → pty.
        // Note: on close poll may report POLLHUP without POLLIN, in which case read returns 0.
        if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 4096];
            match stdin.read(&mut buf) {
                Ok(0) => break, // real terminal closed
                Ok(n) => {
                    // The user pressed enter → the cursor left the input line and async git
                    // results no longer repaint the header (otherwise \e[1A lands in the wrong
                    // place).
                    if buf[..n].iter().any(|&b| b == b'\r' || b == b'\n') {
                        at_prompt = false;
                        typed_since_prompt = false;
                        last_enter = Some(std::time::Instant::now()); // command timing starts
                    } else if !is_terminal_reply(&buf[..n]) {
                        // Terminal replies (DSR cursor position and the like) are not user input.
                        typed_since_prompt = true;
                        last_input.clear();
                        last_input.extend_from_slice(&buf[..n.min(32)]);
                    }
                    writer.write_all(&buf[..n])?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        // pty output → real terminal: pass through.
        // The theme is painted in the two announce-driven strokes.
        if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 {
            let mut buf = [0u8; 8192];
            match reader.read(&mut buf) {
                Ok(0) => break, // shell exited
                Ok(n) => {
                    // The shell prints the prompt as blank rows for the header, then the
                    // zero-width signal, then nothing. Everything up to the signal goes out
                    // first; the header and the input-line prefix are painted on top; what
                    // follows the signal is forwarded after that. A completion list, Ctrl-L or a
                    // pager page reprints the whole prompt, which arrives as another signal.
                    let scan = scanner.push(&buf[..n]);
                    stdout.write_all(&scan.before)?;
                    for _ in 0..scan.signals {
                        // The instant header sits above the input line, and the real header has to
                        // replace it in place. bash and fish repaint the prompt over the row the
                        // engine drew its instant prefix on, so stepping up over the header lands
                        // on the instant header. zsh and pwsh open a fresh row instead, so the
                        // prompt sits one row lower and the step has to start one row higher.
                        if instant_drawn && shell.prompt_starts_on_a_new_row() {
                            write!(stdout, "\x1b[1A")?;
                        }
                        // The shell just reprinted the prompt, so the header rows are blank
                        // again: clear the cache and let this paint fill them.
                        last_header = None;
                        paint_prompt(
                            &mut stdout,
                            &config,
                            last_size.1 as usize,
                            current_info.as_ref(),
                            &instant_info,
                            last_vcs.as_ref(),
                            &mut last_header,
                        )?;
                        instant_drawn = false;
                        at_prompt = true;
                        typed_since_prompt = false;
                    }
                    stdout.write_all(&scan.after)?;
                    // Instant-phase bookkeeping: a newline means the shell printed something
                    // during init (rc echo/warning), so the rows we drew have been pushed down.
                    if instant_drawn {
                        instant_newlines += buf[..n].iter().filter(|b| **b == b'\n').count();
                    }
                    stdout.flush()?;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }

        while let Ok(res) = res_rx.try_recv() {
            if res.generation != git_gen {
                continue; // stale result
            }
            let cwd = current_info
                .as_ref()
                .map(|i| i.cwd.clone())
                .unwrap_or_default();
            last_vcs = Some((cwd.clone(), res.status.clone()));
            if at_prompt && let Some(info) = &current_info {
                let vcs = res.status.as_ref();
                // The rows still hold the paint from the prompt signal. When this result
                // renders the same lines, painting again would write the same bytes.
                theme::redraw_header_cfg(
                    &mut stdout,
                    last_size.1 as usize,
                    &config,
                    info,
                    vcs,
                    &mut last_header,
                )?;
                stdout.flush()?;
            }
            // Input line not ready: last_vcs is already updated and the `p`/marker backfill
            // picks it up, so no extra paint.
        }

        // Deferred prompt repaint after a resize.
        if let Some(deadline) = resize_prompt_at
            && std::time::Instant::now() >= deadline
        {
            resize_prompt_at = None;
            let text = crate::render::input_prefix(&config, current_info.as_ref()).text;
            theme::render_prompt(&mut stdout, &text)?;
            stdout.flush()?;
            debug("deferred prompt drawn");
        }
    }

    // Engine exit: no explicit kill. reader/writer/master are dropped as the function
    // returns, master fd closes → slave hangs up → the inner shell (if still alive) gets
    // SIGHUP and exits. Being orphaned, even a SIGKILLed engine has its master fd closed by
    // the kernel, so the inner shell still gets SIGHUP.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::pick_shell;

    #[test]
    fn shell_names_match_the_cli_flag() {
        assert_eq!(Shell::Zsh.name(), "zsh");
        assert_eq!(Shell::Bash.name(), "bash");
        assert_eq!(Shell::Fish.name(), "fish");
        assert_eq!(Shell::Pwsh.name(), "pwsh");
    }

    #[test]
    fn shell_precedence_puts_the_command_line_first() {
        // The three sources name three different shells, so whichever wins is unambiguous.
        assert_eq!(
            pick_shell(Some("bash"), Some("zsh"), "/usr/bin/fish"),
            Some(Shell::Bash)
        );
        assert_eq!(
            pick_shell(None, Some("zsh"), "/usr/bin/fish"),
            Some(Shell::Zsh)
        );
        assert_eq!(pick_shell(None, None, "/usr/bin/fish"), Some(Shell::Fish));
        // `$SHELL` is a path; only the file name counts.
        assert_eq!(pick_shell(None, None, "/bin/bash"), Some(Shell::Bash));
        // A name p11k cannot proxy falls through to the next source instead of failing.
        assert_eq!(
            pick_shell(Some("tcsh"), Some("zsh"), "/usr/bin/fish"),
            Some(Shell::Zsh)
        );
        assert_eq!(
            pick_shell(None, Some("tcsh"), "/usr/bin/fish"),
            Some(Shell::Fish)
        );
        assert_eq!(pick_shell(None, None, "/usr/bin/tcsh"), None);
    }

    /// DSR/DA/focus are terminal replies, not user typing — misjudging them disables pwsh's
    /// shrink nudge entirely (PSReadLine keeps querying the cursor position on a real
    /// terminal).
    #[test]
    fn terminal_replies_are_not_user_typing() {
        assert!(
            is_terminal_reply(b"\x1b[24;1R"),
            "DSR cursor position reply"
        );
        assert!(is_terminal_reply(b"\x1b[1;1R"));
        assert!(is_terminal_reply(b"\x1b[?1;2c"), "device attributes reply");
        assert!(is_terminal_reply(b"\x1b[I"), "focus in");
        assert!(is_terminal_reply(b"\x1b[O"), "focus out");
        assert!(!is_terminal_reply(b"\x1b[A"), "arrow keys are user typing");
        assert!(!is_terminal_reply(b"\x1b[H"));
        assert!(!is_terminal_reply(b"\x1b"), "bare Esc is user typing");
        assert!(
            !is_terminal_reply(b"\x1b[200~"),
            "bracketed paste start marker"
        );
        assert!(!is_terminal_reply(b"a"));
        assert!(!is_terminal_reply(b"\r"));
        assert!(!is_terminal_reply(b""));
    }
}
