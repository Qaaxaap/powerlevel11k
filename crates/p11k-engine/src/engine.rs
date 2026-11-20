//! The engine itself: start the inner shell in a pty and run the event loop that paints the
//! prompt window.
//!
//! The crate-level overview lives in `main.rs`.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Instant;

/// Real terminal SIGWINCH (resize) flag: the handler only sets it (signal-safe); the main
/// loop then syncs the inner pty size → the inner zsh gets SIGWINCH → TRAPWINCH announces r
/// → the engine clears and repaints the prompt window.
static RESIZE_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigwinch(_sig: libc::c_int) {
    RESIZE_FLAG.store(true, Ordering::Relaxed);
    crate::wake::wake();
}

/// `p11k reload` flag: the command signals this engine and the main loop re-reads the theme
/// file. Set from the handler, acted on by the loop (see `AnnMsg` handling).
static RELOAD_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigusr1(_sig: libc::c_int) {
    RELOAD_FLAG.store(true, Ordering::Relaxed);
    crate::wake::wake();
}

use crate::cli::{Shell, load_config_file};
use crate::config::Config;
use crate::geometry::Geometry;
use crate::git::{GitRequest, GitResult, spawn_git_worker};
use crate::log::{debug, info};
use crate::model::{GitStatus, HeaderInfo, ViMode};
use crate::protocol::{AnnMsg, SignalScanner, drain_announce, is_terminal_reply};
use crate::render::InputPrefix;
use crate::state::{RawTerminal, StateDir, tty_size};
use crate::theme;
use crate::window::paint_prompt;
use portable_pty::{MasterPty, PtySize};

/// Where the cursor is relative to the prompt, and whether the user has typed since it
/// appeared. The engine must not repaint the header while a command runs (the step-up would
/// land on command output) and must leave a half-written input line alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PromptState {
    /// A command is running; the cursor is not on the input line.
    Running,
    /// The prompt is on screen and the input line is empty.
    Prompt,
    /// The prompt is on screen and the user has typed.
    Typing,
}

impl PromptState {
    fn at_prompt(self) -> bool {
        matches!(self, PromptState::Prompt | PromptState::Typing)
    }
}

/// The event loop state. The startup values come from [`crate::session::Session`]; the rest
/// lives for the whole session.
struct Engine {
    config: Config,
    config_path: Option<PathBuf>,
    prefix: InputPrefix,
    geometry: Geometry,
    state: StateDir,
    shell: Shell,
    master: Box<dyn MasterPty + Send>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    master_fd: i32,
    last_size: (u16, u16, u16, u16),
    _raw: RawTerminal,
    stdout: io::Stdout,
    req_tx: mpsc::Sender<GitRequest>,
    res_tx: mpsc::Sender<GitResult>,
    res_rx: mpsc::Receiver<GitResult>,
    git_gen: u64,
    last_vcs: Option<(String, Option<GitStatus>)>,
    current_info: Option<HeaderInfo>,
    prompt: PromptState,
    last_enter: Option<Instant>,
    last_prompt_cwd: Option<String>,
    last_header: Option<Vec<crate::render::PaintedLine>>,
    vi_mode: ViMode,
    last_input: Vec<u8>,
    last_pwsh_nudge: Option<Instant>,
    pwsh_prefix: Option<(String, String)>,
    resize_extra_rows: usize,
    instant_info: HeaderInfo,
    instant_drawn: bool,
    instant_newlines: usize,
    announce_fd: i32,
    _announce_keepalive: i32,
    announce_pending: Vec<u8>,
    scanner: SignalScanner,
    signal_pipe: [i32; 2],
}

impl Engine {
    /// Set up the event loop around a started session: signal pipe, async git worker, announce
    /// FIFO and the instant header.
    fn new(session: crate::session::Session) -> anyhow::Result<Self> {
        let crate::session::Session {
            config,
            config_path,
            prefix,
            geometry,
            state,
            shell,
            master,
            reader,
            writer,
            master_fd,
            size,
            raw,
        } = session;

        // Signal pipe: signal handlers and the git worker write a byte so poll wakes immediately
        // instead of on a timeout. Create it and publish the write end before the handlers and the
        // worker start, so a wake that happens during their setup is not dropped on the floor.
        let mut signal_pipe = [0i32; 2];
        // SAFETY: pipe() writes two fds into the array.
        if unsafe { libc::pipe(signal_pipe.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        crate::wake::install(signal_pipe[1]);

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

        // Async git status.
        // A worker thread owns the RepoCache and the main loop sends requests / receives
        // results over channels, so a 1-2s first scan of a large repo never stalls the prompt.
        let (req_tx, req_rx) = mpsc::channel::<GitRequest>();
        let (res_tx, res_rx) = mpsc::channel::<GitResult>();
        spawn_git_worker(&config, req_rx, res_tx.clone());

        // Announce FIFO: a non-blocking read end, plus a write end we keep, so the read end does
        // not see EOF when the shell closes its write end between prompts.
        let announce_fd = open_fifo(&state.announce, libc::O_RDONLY)?;
        let _announce_keepalive = open_fifo(&state.announce, libc::O_WRONLY)?;

        // instant header: don't wait for the inner shell to finish loading a slow user rc; draw a
        // provisional header from the engine cwd right away so the prompt is visible as soon as the
        // window opens, then swap in the real state after the first inner precmd.
        let instant_info = HeaderInfo {
            exit_code: None,
            cwd: std::env::current_dir()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| "~".into()),
            exec_seconds: 0.0,
            exec_ms: None,
            jobs: 0,
            history: 0,
            pipestatus: Vec::new(),
        };

        let mut engine = Self {
            config,
            config_path,
            prefix,
            geometry,
            state,
            shell,
            master,
            reader,
            writer,
            master_fd,
            last_size: size,
            _raw: raw,
            stdout: io::stdout(),
            req_tx,
            res_tx,
            res_rx,
            git_gen: 0,
            last_vcs: None,
            current_info: None,
            prompt: PromptState::Running,
            last_enter: None,
            last_prompt_cwd: None,
            last_header: None,
            vi_mode: ViMode::Unreported,
            last_input: Vec::new(),
            last_pwsh_nudge: None,
            pwsh_prefix: None,
            resize_extra_rows: 0,
            instant_info,
            instant_drawn: false,
            instant_newlines: 0,
            announce_fd,
            _announce_keepalive,
            announce_pending: Vec::new(),
            scanner: SignalScanner::new(),
            signal_pipe,
        };
        if engine.config.instant_prompt {
            engine.draw_instant()?;
        }
        Ok(engine)
    }

    /// Draw the provisional header and the input-line prefix before the shell starts.
    fn draw_instant(&mut self) -> io::Result<()> {
        theme::render_header_cfg(
            &mut self.stdout,
            self.last_size.1 as usize,
            &self.config,
            &self.instant_info,
            None,
            self.vi_mode,
        )?;
        // Draw the instant prefix without the save/restore-cursor dance: the shell has not
        // started yet, so restoring the cursor would put it back at column 0 and the user's
        // typing would overwrite the prefix. With a plain `\r` the cursor ends up right after
        // it.
        write!(self.stdout, "\r{}", self.prefix.text)?;
        write!(self.stdout, "\x1b]133;B\x07")?;
        self.stdout.flush()?;
        self.instant_drawn = true;
        Ok(())
    }
}

/// Start the engine and run the event loop.
pub fn run() -> anyhow::Result<()> {
    let session = crate::session::Session::start()?;
    let mut engine = Engine::new(session)?;
    engine.run()
}

impl Engine {
    fn run(&mut self) -> anyhow::Result<()> {
        loop {
            if !self.handle_reload()? {
                continue;
            }
            self.handle_resize()?;
            self.handle_announce()?;

            let mut fds = [
                libc::pollfd {
                    fd: libc::STDIN_FILENO,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.master_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.announce_fd,
                    events: libc::POLLIN,
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.signal_pipe[0],
                    events: libc::POLLIN,
                    revents: 0,
                },
            ];
            // No timeout: the terminal, the pty, the announce FIFO and the signal pipe cover every
            // wakeup source, so there is nothing to re-check on a clock.
            let n = unsafe { libc::poll(fds.as_mut_ptr(), 4, -1) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err.into());
            }

            // Wake byte from a signal handler; the flags were read at the top of the loop.
            if fds[3].revents & libc::POLLIN != 0 {
                let mut b = [0u8; 64];
                // SAFETY: reads into a stack array; the result is intentionally ignored.
                let _ = unsafe { libc::read(self.signal_pipe[0], b.as_mut_ptr().cast(), b.len()) };
            }

            // Real terminal input → pty.
            // Note: on close poll may report POLLHUP without POLLIN, in which case read returns 0.
            if fds[0].revents & (libc::POLLIN | libc::POLLHUP) != 0 && !self.handle_stdin()? {
                break;
            }

            // pty output → real terminal: pass through and paint the prompt at each signal.
            if fds[1].revents & (libc::POLLIN | libc::POLLHUP) != 0 && !self.handle_pty()? {
                break;
            }

            self.handle_git()?;
        }

        // Engine exit: no explicit kill. reader/writer/master are dropped as the function
        // returns, master fd closes → slave hangs up → the inner shell (if still alive) gets
        // SIGHUP and exits. Being orphaned, even a SIGKILLed engine has its master fd closed by
        // the kernel, so the inner shell still gets SIGHUP in that case too.
        Ok(())
    }

    /// `p11k reload`: read the theme file again and repaint. A file that no longer parses keeps
    /// the theme already in use (the reload command reports that error itself); the git worker is
    /// respawned so the vcs properties follow the new theme too. Returns false when the caller
    /// must skip the rest of this loop pass (the original `continue`).
    fn handle_reload(&mut self) -> anyhow::Result<bool> {
        if !RELOAD_FLAG.swap(false, Ordering::Relaxed) {
            return Ok(true);
        }
        match self.state.take_reload_request() {
            Ok(request) => {
                if request.is_some() {
                    self.config_path = request;
                }
                match self.config_path.as_deref().map(load_config_file) {
                    Some(Ok(mut new_config)) => {
                        match new_config.apply_locale_fallback() {
                            Ok(warnings) => {
                                for warning in warnings {
                                    eprintln!("p11k: {warning}");
                                }
                            }
                            Err(error) => {
                                // The current theme stays in use; the reload command reports the
                                // error of the new theme.
                                info(&format!("reload: keeping the current theme: {error}"));
                                return Ok(false);
                            }
                        }
                        // The shell re-reads the header height from the ack on every prompt, so a
                        // reload that changes it lands on the next one. The prefix travelled to
                        // the shell at startup and can only change with a restart.
                        self.config = new_config;
                        self.prefix = crate::render::input_prefix(&self.config, None, self.vi_mode);
                        let new_geometry = Geometry::new(&self.config, self.shell);
                        if new_geometry.prefix_ok != self.geometry.prefix_ok
                            || new_geometry.prefix_err != self.geometry.prefix_err
                        {
                            info(
                                "reload: the input-line prefix changed; it takes effect after a restart",
                            );
                        }
                        let header_rows_changed =
                            new_geometry.header_rows != self.geometry.header_rows;
                        self.geometry = new_geometry;
                        // Remember the file so a later reload without `--config` re-reads it.
                        self.state
                            .write_current_config(self.config_path.as_deref())?;
                        let (tx, rx) = mpsc::channel::<GitRequest>();
                        self.req_tx = tx;
                        spawn_git_worker(&self.config, rx, self.res_tx.clone());
                        info("reload: theme reloaded");
                        // A changed row count only takes effect on the next prompt, so repainting
                        // now would move the cursor by the old amount.
                        if self.prompt.at_prompt() && !header_rows_changed {
                            let vcs = self.last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                            // The theme changed: paint over whatever the old theme left.
                            self.last_header = None;
                            theme::redraw_header_cfg(
                                &mut self.stdout,
                                self.last_size.1 as usize,
                                &self.config,
                                self.current_info.as_ref().unwrap_or(&self.instant_info),
                                vcs,
                                &mut self.last_header,
                                self.vi_mode,
                            )?;
                            self.stdout.flush()?;
                        }
                    }
                    Some(Err(e)) => info(&format!("reload: keeping the current theme: {e}")),
                    None => info("reload: this session has no theme file"),
                }
            }
            Err(e) => info(&format!("reload: cannot read the request: {e}")),
        }
        Ok(true)
    }

    /// Real-terminal resize: sync the inner pty size. zsh, bash and fish hook SIGWINCH and
    /// announce `r`; pwsh cannot, so it gets one empty enter on shrink.
    fn handle_resize(&mut self) -> anyhow::Result<()> {
        if !RESIZE_FLAG.swap(false, Ordering::Relaxed) {
            return Ok(());
        }
        let (r, c, xp, yp) = tty_size().unwrap_or(self.last_size);
        if (r, c, xp, yp) == self.last_size {
            return Ok(());
        }
        self.master.resize(PtySize {
            rows: r,
            cols: c,
            pixel_width: xp,
            pixel_height: yp,
        })?;
        self.last_size = (r, c, xp, yp);
        // At the new width the terminal may split the old header rows into more rows than the
        // header has; those extra rows sit above the header the shell is about to reprint and the
        // signal repaint would leave them behind.
        if self.prompt.at_prompt() {
            self.resize_extra_rows = reflow_extra_rows(self.last_header.as_deref(), c as usize);
        }
        debug(&format!("resized pty to {}x{} ({}x{} px)", r, c, xp, yp));
        // During the instant phase no shell `r` announce triggers a repaint yet, so the screen
        // still holds the copy drawn at the old column count (possibly wrapped, no longer in
        // place); repaint it here proactively. Clear and redraw only when the instant phase
        // produced no other output: after a terminal reflow "move up N rows" is unreliable, and
        // this is the only safe option.
        if self.instant_drawn && self.instant_newlines == 0 {
            write!(self.stdout, "\x1b[2J\x1b[H")?;
            theme::render_header_cfg(
                &mut self.stdout,
                c as usize,
                &self.config,
                &self.instant_info,
                None,
                self.vi_mode,
            )?;
            // Same as the initial instant draw: keep the cursor after the prefix instead of
            // restoring it to column 0.
            write!(self.stdout, "\r{}", self.prefix.text)?;
            write!(self.stdout, "\x1b]133;B\x07")?;
            self.stdout.flush()?;
        }
        // pwsh cannot hook SIGWINCH and only compares the window size inside its prompt
        // function, so after a shrink the screen still holds the terminal-reflowed old content
        // until the next prompt relayouts it. Send one empty enter on the user's behalf (only
        // while the input line is still empty) so pwsh calls prompt again → announces r → the
        // engine relayouts at the new width.
        if self.shell == Shell::Pwsh
            && self.prompt == PromptState::Prompt
            && self
                .last_pwsh_nudge
                .is_none_or(|t| t.elapsed() > std::time::Duration::from_millis(150))
        {
            debug("resize: nudge pwsh with an empty line so it redraws");
            let _ = self.writer.write_all(b"\r");
            let _ = self.writer.flush();
            self.last_pwsh_nudge = Some(Instant::now());
        } else if self.shell == Shell::Pwsh && self.prompt.at_prompt() {
            debug(&format!(
                "resize: no pwsh nudge (typed={}, last_input_len={})",
                self.prompt == PromptState::Typing,
                self.last_input.len()
            ));
        }
        Ok(())
    }

    /// Handle announce messages: `h` draws the header and releases the shell, `r` repaints the
    /// prompt window, `v` updates the editing mode. The pty is drained first: a `h` message that
    /// arrives while command output still sits in the pty buffer must not paint above it.
    fn handle_announce(&mut self) -> anyhow::Result<()> {
        let pty_ready = {
            let mut probe = libc::pollfd {
                fd: self.master_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            (unsafe { libc::poll(&mut probe, 1, 0) }) > 0 && probe.revents & libc::POLLIN != 0
        };
        if pty_ready {
            return Ok(());
        }
        for msg in drain_announce(self.announce_fd, &mut self.announce_pending) {
            // Sync a size change to the pty and let right alignment use the new width.
            let (r, c, xp, yp) = tty_size().unwrap_or(self.last_size);
            if (r, c, xp, yp) != self.last_size {
                self.master.resize(PtySize {
                    rows: r,
                    cols: c,
                    pixel_width: xp,
                    pixel_height: yp,
                })?;
                self.last_size = (r, c, xp, yp);
            }
            match msg {
                AnnMsg::Header(mut info) => {
                    // New prompt: the command segments run again instead of showing the
                    // previous prompt's values.
                    crate::render::clear_cmd_cache();
                    debug(&format!(
                        "h: exit={:?} cwd={:?} jobs={}",
                        info.exit_code, info.cwd, info.jobs
                    ));
                    // Use the shell's own command duration. The engine timing (last enter → this
                    // precmd) is the fallback when the shell sends no value. Always consume
                    // last_enter. A delayed announce must not give the next command's timing to a
                    // later prompt.
                    let engine_seconds = self
                        .last_enter
                        .take()
                        .map(|t| t.elapsed().as_secs_f64())
                        .unwrap_or(0.0);
                    info.exec_seconds = info
                        .exec_ms
                        .map(|ms| ms as f64 / 1000.0)
                        .unwrap_or(engine_seconds);
                    self.prompt = PromptState::Running; // the next signal paints the input line
                    // The instant header stays flagged until the first signal arrives: that is
                    // where the real header replaces it in place.
                    // The relaxed-layout blank rows are part of the shell's prompt string: the
                    // ack tells the shell how many blank rows to print, so every redraw carries
                    // them.
                    // Emit the prompt-start marker before the shell prints the prompt string; the
                    // header is not drawn here, the signal that follows the blank rows is what
                    // draws it.
                    theme::prompt_start(&mut self.stdout)?;
                    self.stdout.flush()?;
                    // Release precmd and hand the shell the prompt shape: the header height on
                    // the first line, then the prefix for this prompt. The engine picks that
                    // prefix because it is the only side that knows the exit code, the pipeline
                    // statuses and the vi mode; a shell choosing between two strings of its own
                    // could not tell `OK_PIPE` from `OK`.
                    let state_name =
                        crate::render::prompt_state(&self.config, Some(&info), self.vi_mode);
                    let prefix = Geometry::prefix_for(&self.config, self.shell, state_name);
                    if self.shell == Shell::Pwsh {
                        // Keep the visible text next to the coloured form: PSReadLine repaints the
                        // prompt from the visible text only, and the forwarding path replaces that
                        // copy with the coloured one.
                        self.pwsh_prefix =
                            Some((crate::render::strip_ansi(&prefix), prefix.clone()));
                    }
                    // Transient prompt: `same-dir` folds the header only when the command left the
                    // shell where it was (p10k's `always` / `same-dir` split). The engine decides
                    // because it already knows both directories; the shell only reads the answer.
                    let transient_on = transient_folds(
                        self.config.layout.transient_prompt,
                        self.last_prompt_cwd.as_deref(),
                        &info.cwd,
                    );
                    self.last_prompt_cwd = Some(info.cwd.clone());
                    // Without the ack the shell falls back to the last prompt shape after its
                    // timeout: the session stays usable. The log says why the header is missing,
                    // instead of the engine exiting mid-session.
                    let rows = self.geometry.header_rows
                        + if self.instant_drawn {
                            0
                        } else {
                            self.config.layout.prompt_add_newline
                        };
                    if let Err(e) = write_ack(&self.state.ack, rows, &prefix, transient_on) {
                        crate::log::log(
                            crate::log::Level::Error,
                            &format!("cannot write the prompt ack: {e}"),
                        );
                    }
                    // Compute git in the background and repaint the header asynchronously when
                    // it finishes.
                    self.git_gen += 1;
                    let _ = self.req_tx.send(GitRequest {
                        generation: self.git_gen,
                        cwd: info.cwd.clone(),
                    });
                    self.current_info = Some(info);
                    debug("drew header, acked");
                }
                AnnMsg::Resize => {
                    // The size check at the top of the handler already resized the pty.
                    // zsh: zle repaints the prompt string and the backfill goes through the
                    // signal; repaint the header here.
                    // Other shells: after a resize the shell reprints the prompt string from
                    // scratch (with new leading newlines) and the signal path above backfills it;
                    // the new prompt is not out yet, so drawing now would land on the previous
                    // screen and leave it uncovered.
                    if self.prompt.at_prompt() {
                        if self.shell == Shell::Zsh {
                            debug("r: redraw header");
                            let vcs = self.last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                            // A resize can reflow the rows under the header, so paint
                            // unconditionally; the signal that follows repaints again on its own.
                            self.last_header = None;
                            theme::redraw_header_cfg(
                                &mut self.stdout,
                                self.last_size.1 as usize,
                                &self.config,
                                self.current_info.as_ref().unwrap_or(&self.instant_info),
                                vcs,
                                &mut self.last_header,
                                self.vi_mode,
                            )?;
                            self.stdout.flush()?;
                        } else {
                            debug("r: wait for the shell to reprint the prompt");
                        }
                    } else {
                        debug("r: skip redraw (not at prompt)");
                    }
                }
                AnnMsg::VimMode(mode) => {
                    self.vi_mode = ViMode::from_wire(&mode);
                    if self.prompt.at_prompt() {
                        debug("v: update mode, redraw header");
                        let vcs = self.last_vcs.as_ref().and_then(|(_, s)| s.as_ref());
                        theme::redraw_header_cfg(
                            &mut self.stdout,
                            self.last_size.1 as usize,
                            &self.config,
                            self.current_info.as_ref().unwrap_or(&self.instant_info),
                            vcs,
                            &mut self.last_header,
                            self.vi_mode,
                        )?;
                        self.stdout.flush()?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Real-terminal input → pty. Returns false when the terminal closed (stop the loop).
    fn handle_stdin(&mut self) -> anyhow::Result<bool> {
        let mut buf = [0u8; 4096];
        match io::stdin().read(&mut buf) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                // The user pressed enter → the cursor left the input line and async git results
                // no longer repaint the header (otherwise \e[1A lands in the wrong place).
                if buf[..n].iter().any(|&b| b == b'\r' || b == b'\n') {
                    self.prompt = PromptState::Running;
                    self.last_enter = Some(Instant::now()); // command timing starts
                } else if !is_terminal_reply(&buf[..n]) {
                    // Terminal replies (DSR cursor position and the like) are not user input.
                    // Input while a command runs does not touch the prompt state.
                    if self.prompt.at_prompt() {
                        self.prompt = PromptState::Typing;
                    }
                    self.last_input.clear();
                    self.last_input.extend_from_slice(&buf[..n.min(32)]);
                }
                self.writer.write_all(&buf[..n])?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
        Ok(true)
    }

    /// pty output → real terminal: pass every byte through and paint the prompt at each signal.
    /// Returns false when the shell exited (stop the loop).
    fn handle_pty(&mut self) -> anyhow::Result<bool> {
        let mut buf = [0u8; 8192];
        match self.reader.read(&mut buf) {
            Ok(0) => return Ok(false),
            Ok(n) => {
                // The shell prints the prompt as blank rows for the header, then the zero-width
                // signal, then nothing. Everything up to a signal goes out first; the header is
                // painted on top; what follows the signal is forwarded after that. A completion
                // list, Ctrl-L or a pager page reprints the whole prompt, which arrives as
                // another signal in the same read: write its segment before painting it.
                let scan = self.scanner.push(&buf[..n]);
                for segment in &scan.segments {
                    let before =
                        rewrite_pwsh_prefix(segment, self.shell, self.pwsh_prefix.as_ref());
                    self.stdout.write_all(&before)?;
                    if self.resize_extra_rows > 0 {
                        // The reflowed rows sit above the header the shell just reprinted;
                        // clear them before the repaint covers the header itself.
                        let rows = self.geometry.header_rows.max(1) + self.resize_extra_rows;
                        write!(self.stdout, "\x1b7\x1b[{}A", rows)?;
                        for i in 0..self.resize_extra_rows {
                            write!(self.stdout, "\r\x1b[K")?;
                            if i + 1 < self.resize_extra_rows {
                                write!(self.stdout, "\x1b[1B")?;
                            }
                        }
                        write!(self.stdout, "\x1b8")?;
                        self.resize_extra_rows = 0;
                    }
                    // The first prompt prints its own header rows. The real header goes on those
                    // rows; the instant header and the input line below it stay above as history.
                    // The shell just reprinted the prompt, so the header rows are blank again:
                    // clear the cache and let this paint fill them.
                    self.last_header = None;
                    paint_prompt(
                        &mut self.stdout,
                        crate::window::PromptPaint {
                            config: &self.config,
                            rows: self.last_size.1 as usize,
                            current: self.current_info.as_ref(),
                            instant: &self.instant_info,
                            last_vcs: self.last_vcs.as_ref(),
                            vi_mode: self.vi_mode,
                        },
                        &mut self.last_header,
                    )?;
                    self.instant_drawn = false;
                    self.prompt = PromptState::Prompt;
                }
                let after = rewrite_pwsh_prefix(&scan.tail, self.shell, self.pwsh_prefix.as_ref());
                self.stdout.write_all(&after)?;
                // Instant-phase bookkeeping: a newline means the shell printed something during
                // init (rc echo/warning), so the rows we drew have been pushed down.
                if self.instant_drawn {
                    self.instant_newlines += buf[..n].iter().filter(|b| **b == b'\n').count();
                }
                self.stdout.flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
        Ok(true)
    }

    /// Drain async git results. A stale generation is ignored; a current result repaints the
    /// header only while the input line is ready.
    fn handle_git(&mut self) -> anyhow::Result<()> {
        while let Ok(res) = self.res_rx.try_recv() {
            if res.generation != self.git_gen {
                continue; // stale result
            }
            let cwd = self
                .current_info
                .as_ref()
                .map(|i| i.cwd.clone())
                .unwrap_or_default();
            self.last_vcs = Some((cwd, res.status.clone()));
            if self.prompt.at_prompt()
                && let Some(info) = &self.current_info
            {
                let vcs = res.status.as_ref();
                // The rows still hold the paint from the prompt signal. When this result renders
                // the same lines, painting again would write the same bytes.
                theme::redraw_header_cfg(
                    &mut self.stdout,
                    self.last_size.1 as usize,
                    &self.config,
                    info,
                    vcs,
                    &mut self.last_header,
                    self.vi_mode,
                )?;
                self.stdout.flush()?;
            }
            // Input line not ready: last_vcs is already updated and the `p`/marker backfill picks
            // it up, so no extra paint.
        }
        Ok(())
    }
}

/// Open a FIFO non-blocking so the engine never waits on the shell's open.
fn open_fifo(path: &std::path::Path, flags: libc::c_int) -> io::Result<i32> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "fifo path"))?;
    // SAFETY: NUL-terminated path.
    let fd = unsafe { libc::open(c.as_ptr(), flags | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Rows the terminal reflow adds when `lines` (the previous header) are rendered at `cols`:
/// everything above one row per header line. The engine cannot move those rows by "up N rows"
/// after a reflow; the signal repaint clears them instead.
fn reflow_extra_rows(lines: Option<&[crate::render::PaintedLine]>, cols: usize) -> usize {
    let Some(lines) = lines else {
        return 0;
    };
    let cols = cols.max(1);
    let rows: usize = lines
        .iter()
        .map(|line| {
            if line.width == 0 {
                1
            } else {
                line.width.div_ceil(cols)
            }
        })
        .sum();
    rows.saturating_sub(lines.len())
}

/// Whether this prompt folds. `same-dir` compares the directory with the one recorded on the
/// previous prompt, so the first prompt (no record) keeps its header; p10k compares against an
/// empty value and does not fold there either.
fn transient_folds(
    mode: crate::config::TransientPrompt,
    last_cwd: Option<&str>,
    cwd: &str,
) -> bool {
    match mode {
        crate::config::TransientPrompt::Off => false,
        crate::config::TransientPrompt::Always => true,
        crate::config::TransientPrompt::SameDir => last_cwd == Some(cwd),
    }
}

/// PSReadLine repaints the prompt from a copy without the SGR sequences it was handed, so the
/// frame's last prefix loses its colour after the first redraw (PSReadLine issues #986 and
/// #4716: it does not remember the colours of the prompt). Replace that visible copy with the
/// coloured form. The match is the exact visible input-line prefix; command output that happens
/// to contain the same bytes would be rewritten too, but the prefix is the frame's last piece
/// plus the prompt char and that collision is not a practical concern. A prefix split across two
/// pty reads is not rewritten; the reads are 8 KiB and the prefix starts at an output boundary.
fn rewrite_pwsh_prefix(data: &[u8], shell: Shell, prefix: Option<&(String, String)>) -> Vec<u8> {
    let Some((visible, colored)) = prefix else {
        return data.to_vec();
    };
    if shell != Shell::Pwsh || visible.is_empty() {
        return data.to_vec();
    }
    let visible = visible.as_bytes();
    let colored = colored.as_bytes();
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i..].starts_with(visible) {
            out.extend_from_slice(colored);
            i += visible.len();
        } else {
            out.push(data[i]);
            i += 1;
        }
    }
    out
}

/// Write the ack the shell waits for. The ack is line-oriented: header height, prefix, fold
/// flag. A prefix with a line break would shift the following values into the wrong lines, so
/// it is rejected instead of producing an ack the shell reads incorrectly.
fn write_ack(path: &std::path::Path, rows: usize, prefix: &str, fold: bool) -> io::Result<()> {
    if prefix.contains(['\n', '\r']) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the prompt prefix contains a line break; the ack is line oriented",
        ));
    }
    let tmp = path.with_extension("tmp");
    {
        let mut file = File::create(&tmp)?;
        writeln!(file, "{rows}")?;
        writeln!(file, "{prefix}")?;
        writeln!(file, "{}", u8::from(fold))?;
        file.flush()?;
    }
    std::fs::rename(&tmp, path)
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

    /// `same-dir` folds only after the first command: the first prompt has no directory to
    /// compare against, and p10k compares against an empty value, so it keeps its header.
    #[test]
    fn transient_fold_decision() {
        use crate::config::TransientPrompt::{Always, Off, SameDir};
        assert!(!transient_folds(Off, None, "/a"));
        assert!(transient_folds(Always, None, "/a"));
        assert!(
            !transient_folds(SameDir, None, "/a"),
            "the first prompt must keep its header"
        );
        assert!(transient_folds(SameDir, Some("/a"), "/a"));
        assert!(!transient_folds(SameDir, Some("/a"), "/b"));
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

    /// The shell polls for the ack and reads it the moment the file exists. A half-written file
    /// would give it the wrong prompt shape, so the writer goes through a sibling temp file and a
    /// rename.
    #[test]
    fn ack_is_written_atomically() {
        let dir = std::env::temp_dir().join(format!("p11k-ack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ack");

        write_ack(&path, 2, "%F{red}❯ ", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "2\n%F{red}❯ \n1\n");
        assert!(!dir.join("ack.tmp").exists(), "the temp file must be gone");

        write_ack(&path, 1, "❯ ", false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1\n❯ \n0\n");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A prefix with a line break would shift the fold flag into another line, so the ack is
    /// rejected instead of written in a shape the shell would misread.
    #[test]
    fn ack_rejects_a_prefix_with_a_line_break() {
        let dir = std::env::temp_dir().join(format!("p11k-ack-break-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ack");

        for prefix in ["a\nb", "a\rb", "\n"] {
            let err = write_ack(&path, 1, prefix, false).expect_err("a line break is invalid");
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "prefix {prefix:?}");
            assert!(!path.exists(), "no ack file for prefix {prefix:?}");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
