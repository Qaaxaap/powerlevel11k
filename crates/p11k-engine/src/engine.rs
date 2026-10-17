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
use crate::git::{GitRequest, GitResult, spawn_git_worker};
use crate::i18n::t;
use crate::log::log;
use crate::protocol::{AnnMsg, SignalScanner, drain_announce, is_terminal_reply};
use crate::state::{RawTerminal, StateDir, tty_size};
use crate::theme;
use crate::theme::{GitStatus, HeaderInfo};
use crate::window::paint_prompt;
use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// Visible text of a rendered prefix: the ANSI colouring is dropped so the shell can put it in
/// its prompt string without knowing which byte runs are non-printing. Wrapping the whole
/// prefix in zsh's `%{...%}` would make it zero-width and start the buffer at column 0.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        // CSI: ESC [ ... final byte in 0x40..=0x7e.
        let it = chars.by_ref();
        if it.next() == Some('[') {
            for c in it.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    break;
                }
            }
        }
    }
    out
}

pub fn run() -> anyhow::Result<()> {
    // The config comes first: it may name the shell, and it decides the input-line prefix
    // width, from which the placeholder is built.
    let mut config = load_config();
    let shell = detect_shell(&config);
    // The file the theme came from, kept for `p11k reload` (None for a preset or the built-in
    // theme; the engine then has nothing to re-read). Made absolute, because the reload
    // command reads it from wherever the shell happens to be, not from the starting directory.
    // A path that cannot be resolved is kept as it is: reloading then reports the real error.
    let config_path = theme_file().map(|p| p.canonicalize().unwrap_or(p));
    // Every prompt_char state (normal/ERROR) must be the same width.
    // Unequal widths are a config error: report on the real terminal, then exec a clean shell.
    if let Err(e) = crate::render::check_prompt_char_widths(&config) {
        eprintln!("p11k: {}{e}", t("config error: "));
        eprintln!(
            "p11k: {}",
            t(
                "every prompt_char state must be the same width (the prompt width is fixed at startup). Fix the config and start p11k again."
            )
        );
        use std::os::unix::process::CommandExt;
        let err = match shell {
            Shell::Zsh => std::process::Command::new("zsh").arg("-f").exec(),
            Shell::Bash => std::process::Command::new("bash")
                .args(["--noprofile", "--norc"])
                .exec(),
            Shell::Fish => std::process::Command::new("fish").arg("--no-config").exec(),
            Shell::Pwsh => std::process::Command::new("pwsh")
                .args(["-NoProfile", "-NoLogo"])
                .exec(),
        };
        eprintln!("p11k: {}{err}", t("cannot exec a clean shell: "));
        std::process::exit(1);
    }
    let mut prefix = crate::render::input_prefix(&config, None);
    // The placeholder carries header_rows newlines so the shell's prompt geometry counts
    // the header rows too; only then can zsh fold transient cleanly with reset-prompt.
    // The prompt skeleton each shell prints: `header_rows` blank rows for the header, then the
    // zero-width prompt-ready signal. Handed over as data so no template carries it. The engine
    // fills the blank rows and draws the input-line prefix when the signal arrives, which keeps
    // the prefix's styling in the theme and its width unconstrained.
    let header_rows = config.layout.left.len().max(config.layout.right.len());
    // The blank rows the prompt opens with; the templates append the zero-width signal and the
    // prefix. The signal has to be written by the shell: each shell marks non-printing
    // sequences differently (`%{...%}` in zsh, `\[...\]` in bash) and the engine handing over a
    // bare OSC would make the line editor count it as visible width.
    let prompt_lead = "\n".repeat(header_rows);
    let state = StateDir::create(shell)?;
    log(&format!(
        "engine start: dir={} announce={} ack={} prompt_lead={:?} shell={:?}",
        state.dir.display(),
        state.announce.display(),
        state.ack.display(),
        prompt_lead,
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
    // The prompt skeleton each shell prints: `header_rows` blank rows. The templates append the
    // zero-width signal and the prefix (the engine cannot: each shell has its own syntax for
    // marking non-printing sequences).
    cmd.env("P11K_PROMPT_LEAD", &prompt_lead);
    // Set while the instant header is on screen: the first prompt must not print blank rows for
    // the header, the engine overwrites the instant header in place instead.
    cmd.env("P11K_INSTANT", "1");
    // Both states of the input-line prefix, as visible text. zle/readline draw the user's buffer
    // immediately after the prompt string, so the prefix has to be part of it — painting it
    // afterwards would overwrite the front of the buffer on every completion redraw.
    let err_info = HeaderInfo {
        exit_code: Some(1),
        cwd: String::new(),
        exec_seconds: 0.0,
        jobs: 0,
        history: 0,
        pipestatus: Vec::new(),
    };
    // zsh gets its own prompt syntax (`%F{...}`, zero width for zle); the other shells get the
    // visible text, since their line editors count bare ANSI as width.
    let (prefix_ok, prefix_err) = if shell == Shell::Zsh {
        (
            crate::render::prefix_text_zsh(&config, None),
            crate::render::prefix_text_zsh(&config, Some("ERROR")),
        )
    } else {
        (
            strip_ansi(&crate::render::input_prefix(&config, None).text),
            strip_ansi(&crate::render::input_prefix(&config, Some(&err_info)).text),
        )
    };
    cmd.env("P11K_PREFIX_OK", &prefix_ok);
    cmd.env("P11K_PREFIX_ERR", &prefix_err);
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
    if config.layout.transient_prompt && shell == Shell::Zsh {
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
    // Whether the cursor is on the input line (after the p announce, before the user
    // presses enter). Async results repaint only then, otherwise redraw's \e[1A lands on
    // command output.
    let mut at_prompt = false;
    // Time of the last enter, used by the next precmd to compute command duration.
    let mut last_enter: Option<std::time::Instant> = None;
    // Defer the prompt repaint after a resize: wait for zle's repainted placeholder+buffer
    // to pass through (~60ms) before painting the prefix, so the prompt is not painted
    // before zle repaints (and gets covered by the placeholder) or ahead of the buffer.
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

    // instant header: don't wait for the inner shell to finish loading a slow user rc; draw
    // a placeholder header + prompt from the engine cwd right away so the prompt is visible
    // as soon as the window opens, then swap in the real state after the first inner precmd.
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
    let mut instant_widths =
        theme::render_header_cfg(&mut stdout, cols as usize, &config, &instant_info, None)?;
    // Draw the instant prefix without the save/restore-cursor dance: there is no placeholder
    // here, so restoring the cursor would put it back at column 0 and the user's typing would
    // overwrite the prefix. With a plain `\r` the cursor ends up right after it.
    write!(stdout, "\r{}", prefix.text)?;
    write!(stdout, "\x1b]133;B\x07")?;
    stdout.flush()?;
    let mut instant_drawn = true;
    /// Terminal rows the instant header actually occupies at the given column count;
    /// content wider than the terminal wraps onto more rows.
    fn instant_rows_at(widths: &[usize], cols: u16) -> usize {
        let c = cols.max(1) as usize;
        widths.iter().map(|w| w.div_ceil(c).max(1)).sum()
    }
    // What the inner shell passed through to the terminal during the instant phase: byte
    // count + newline count. Zero newlines means the only header on screen is ours (safe to
    // erase in place and repaint); otherwise the rc really printed something, those lines
    // sit below ours and must be kept.
    let mut instant_bytes = 0usize;
    let mut instant_newlines = 0usize;

    let mut scanner = SignalScanner::new();

    loop {
        // `p11k reload`: read the theme file again and repaint. A file that no longer parses
        // keeps the theme already in use (the reload command reports that error itself); the
        // git worker is respawned so the vcs properties follow the new theme too.
        if RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            match config_path.as_deref().map(load_config_file) {
                Some(Ok(new_config)) => {
                    config = new_config;
                    // The input-line prefix is written into the shell's PROMPT at startup, so
                    // its width is fixed: a theme that changes the prompt char width would
                    // desync the placeholder. Keep the old prefix and say so.
                    let new_prefix = crate::render::input_prefix(&config, None);
                    if new_prefix.width == prefix.width {
                        prefix = new_prefix;
                    } else {
                        log(&format!(
                            "reload: keeping the old prompt prefix (width {} -> {})",
                            prefix.width, new_prefix.width
                        ));
                    }
                    let (tx, rx) = mpsc::channel::<GitRequest>();
                    req_tx = tx;
                    spawn_git_worker(&config, rx, res_tx.clone());
                    log("reload: theme reloaded");
                    if at_prompt {
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_header_cfg(
                            &mut stdout,
                            last_size.1 as usize,
                            &config,
                            current_info.as_ref().unwrap_or(&instant_info),
                            vcs,
                        )?;
                        stdout.flush()?;
                    }
                }
                Some(Err(e)) => log(&format!("reload: keeping the current theme: {e}")),
                None => log("reload: this session has no theme file"),
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
                log(&format!("resized pty to {}x{} ({}x{} px)", r, c, xp, yp));
                // During the instant phase no shell `r` announce triggers a repaint yet, so
                // the screen still holds the copy drawn at the old column count (possibly
                // wrapped, no longer in place); repaint it here proactively. Clear and redraw
                // only when the instant phase produced no other output: after a terminal
                // reflow "move up N rows" is unreliable, and this is the only safe option.
                if instant_drawn && instant_newlines == 0 {
                    write!(stdout, "\x1b[2J\x1b[H")?;
                    instant_widths = theme::render_header_cfg(
                        &mut stdout,
                        c as usize,
                        &config,
                        &instant_info,
                        None,
                    )?;
                    // Same as the initial instant draw: no placeholder, so keep the cursor after
                    // the prefix instead of restoring it to column 0.
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
                    log("resize: nudge pwsh with an empty line so it redraws");
                    let _ = writer.write_all(b"\r");
                    let _ = writer.flush();
                    last_pwsh_nudge = Some(std::time::Instant::now());
                } else if shell == Shell::Pwsh && at_prompt {
                    log(&format!(
                        "resize: no pwsh nudge (typed={typed_since_prompt}, last_input={:?})",
                        String::from_utf8_lossy(&last_input)
                    ));
                }
            }
        }

        // prompt window (announce-driven). Handled before poll/pass-through: in fish's
        // resize the `r` announce and the placeholder output are nearly simultaneous, so
        // drain first to arm pending, then byte-match the placeholder while passing the pty
        // through (otherwise the placeholder passes first and pending is set too late).
        // - `h` (precmd announce, shell waiting for ack): draw header, touch ack to release.
        // - `p` (zle-line-init announce): back to line start, paint the prefix over the placeholder.
        // - `r` (resize announce): repaint the prompt window.
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
                    log(&format!(
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
                    // Emit the prompt-start marker before the shell's multi-line placeholder;
                    // the header is not drawn here, or it would advance the cursor twice on
                    // top of the placeholder's own newlines.
                    theme::prompt_start(&mut stdout)?;
                    stdout.flush()?;
                    File::create(&state.ack)?; // release precmd → the shell prints the prompt
                    // Compute git in the background and repaint the header asynchronously when
                    // it finishes.
                    git_gen += 1;
                    let _ = req_tx.send(GitRequest {
                        generation: git_gen,
                        cwd: info.cwd.clone(),
                    });
                    current_info = Some(info);
                    log("drew header, acked");
                }
                AnnMsg::Prompt => {
                    log("p: fill header + draw prompt prefix");
                    // zle has rendered the multi-line placeholder (header_rows blank lines +
                    // the placeholder) and the cursor sits after it: move up header rows to
                    // backfill the real header, then return to line start and overwrite the
                    // placeholder with the prefix.
                    paint_prompt(
                        &mut stdout,
                        &config,
                        last_size.1 as usize,
                        current_info.as_ref(),
                        &instant_info,
                        last_vcs.as_ref(),
                    )?;
                    stdout.flush()?;
                    at_prompt = true; // input line ready
                    typed_since_prompt = false;
                }
                AnnMsg::Resize => {
                    // The size check at the top of the loop already resized the pty.
                    // zsh: zle repaints the placeholder prompt and the backfill goes through
                    // the `p` announce; repaint the header here and defer the prefix by ~60ms
                    // (until zle's repainted placeholder+buffer has passed through).
                    // Other shells: after a resize the shell prints the placeholder from
                    // scratch (with new leading newlines) and the marker-matching path above
                    // backfills it — the placeholder is not out yet, so drawing now would land
                    // on the previous screen and leave the placeholder uncovered.
                    if at_prompt {
                        if shell == Shell::Zsh {
                            log("r: redraw header, defer prompt");
                            let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                            theme::redraw_header_cfg(
                                &mut stdout,
                                last_size.1 as usize,
                                &config,
                                current_info.as_ref().unwrap_or(&instant_info),
                                vcs,
                            )?;
                            stdout.flush()?;
                            resize_prompt_at = Some(
                                std::time::Instant::now() + std::time::Duration::from_millis(60),
                            );
                        } else {
                            log("r: wait for the shell to reprint the prompt");
                        }
                    } else {
                        log("r: skip redraw (not at prompt)");
                    }
                }
                AnnMsg::VimMode(mode) => {
                    *crate::render::CURRENT_VI_MODE.lock().unwrap() = mode;
                    if at_prompt {
                        log("v: update mode, redraw header");
                        let vcs = last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_header_cfg(
                            &mut stdout,
                            last_size.1 as usize,
                            &config,
                            current_info.as_ref().unwrap_or(&instant_info),
                            vcs,
                        )?;
                        stdout.flush()?;
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
                        // The instant header is still on screen above the input line: step over
                        // its rows first, so the real header replaces it in place instead of
                        // being painted below it.
                        if instant_drawn {
                            write!(stdout, "\x1b[1A")?;
                        }
                        paint_prompt(
                            &mut stdout,
                            &config,
                            last_size.1 as usize,
                            current_info.as_ref(),
                            &instant_info,
                            last_vcs.as_ref(),
                        )?;
                        instant_drawn = false;
                        at_prompt = true;
                        typed_since_prompt = false;
                    }
                    stdout.write_all(&scan.after)?;
                    // Instant-phase bookkeeping: when the shell printed something during init
                    // (rc echo/warning), the rows we drew have been pushed down and must not be
                    // erased later.
                    if instant_drawn {
                        instant_bytes += n;
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
                theme::redraw_header_cfg(&mut stdout, last_size.1 as usize, &config, info, vcs)?;
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
            log("deferred prompt drawn");
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
