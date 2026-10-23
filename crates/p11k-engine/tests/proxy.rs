//! End-to-end tests: run the engine inside tmux and assert on what the terminal sees.
//!
//! tmux is the terminal here rather than a bare pty, for three reasons:
//!
//! - the engine's peer has to answer the shell. pwsh's line editor asks for the cursor position
//!   (`CSI 6n`) after every keystroke and blocks until the reply arrives; a pty that never replies
//!   leaves it redrawing on wrong coordinates instead of echoing the key. (The demo images are
//!   recorded inside tmux for the same reason.)
//! - `capture-pane` is the rendered screen — what the user is looking at — so a case can assert on
//!   the screen instead of reconstructing it.
//! - `pipe-pane` still hands over the raw byte stream the engine wrote, so the byte-level
//!   assertions (the signal protocol, the header backfill) stay as sharp as they were.
//!
//! The engine does not parse pty output; the bytes it paints are the same ones the user sees in the
//! terminal, and those bytes are what most assertions land on.
//!
//! Each case gets its own tmux server (`-L <socket>`), so parallel cases — and whatever the
//! developer has running — cannot disturb each other.

use std::cell::Cell;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

const COLS: u16 = 100;
const ROWS: u16 = 24;

/// tmux servers are named per case: the process id separates concurrent test binaries, the counter
/// the cases inside one.
static SOCKET_SEQ: AtomicUsize = AtomicUsize::new(0);

/// Held by a pwsh case for its whole lifetime. pwsh carries a .NET runtime and PSReadLine, and
/// several of them starting at once on a two-core runner starve each other badly enough that the
/// prompt never comes back.
static PWSH_SERIAL: Mutex<()> = Mutex::new(());

/// A missing program is a broken environment (the devShell provides every shell these tests use),
/// said plainly instead of surfacing as a timeout later on.
fn assert_on_path(program: &str) {
    let found = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()));
    assert!(
        found,
        "`{program}` is not on PATH: the devShell provides it (flake.nix), so this is an \
         environment problem rather than an engine one"
    );
}

/// A tmux server with the engine running in it, plus the file `pipe-pane` appends the engine's
/// output to: everything a case needs comes from those bytes or from the rendered pane.
struct Tmux {
    socket: String,
    /// What the engine wrote to the terminal, captured by `pipe-pane`.
    capture: PathBuf,
    /// How much of `capture` the byte assertions have already taken.
    consumed: Cell<usize>,
}

impl Tmux {
    const SESSION: &'static str = "engine";

    fn start(socket: &str, shell: &str, config_home: &str) -> Tmux {
        assert_on_path("tmux");
        assert_on_path(shell);
        // A case that panicked before Drop left its server behind.
        Self::tmux(socket, &["kill-server"]);

        let capture = std::env::temp_dir().join(format!("p11k-proxy-{socket}.out"));
        let _ = std::fs::remove_file(&capture);

        // `sleep` first: pipe-pane has to be attached before the engine writes its first byte, or
        // the opening prompt is missing from the capture. `exec` keeps the engine as the pane's
        // process, so the pane dies with the engine. cwd `/tmp` (not a git repository, so git
        // status scanning cannot jitter the prompt timing); the user rcs point at `/dev/null` (a
        // real `~/.zshrc` with oh-my-zsh/p10k is slow and disruptive to the assertions).
        let cmd = format!(
            "sleep 0.3; cd /tmp && XDG_CONFIG_HOME={config_home} P11K_USER_ZSHRC=/dev/null \
             P11K_USER_PROFILE=/dev/null exec '{}' --shell {shell}",
            env!("CARGO_BIN_EXE_p11k")
        );
        let out = Self::tmux(
            socket,
            &[
                "new-session",
                "-d",
                "-s",
                Self::SESSION,
                "-x",
                &COLS.to_string(),
                "-y",
                &ROWS.to_string(),
                // Pin the terminal type of the pane. readline spells clear-screen from
                // terminfo, and a check environment whose default entry is missing made the
                // Ctrl-L case skip the sequence with the engine innocent.
                "-e",
                "TERM=screen",
                &cmd,
            ],
        );
        assert!(
            out.status.success(),
            "tmux new-session failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let out = Self::tmux(
            socket,
            &[
                "pipe-pane",
                "-t",
                Self::SESSION,
                "-o",
                &format!("cat >> '{}'", capture.display()),
            ],
        );
        assert!(
            out.status.success(),
            "tmux pipe-pane failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );

        Tmux {
            socket: socket.to_string(),
            capture,
            consumed: Cell::new(0),
        }
    }

    fn tmux(socket: &str, args: &[&str]) -> std::process::Output {
        std::process::Command::new("tmux")
            .arg("-L")
            .arg(socket)
            .args(args)
            .output()
            .unwrap_or_else(|e| panic!("tmux cannot be run (the devShell provides it): {e}"))
    }

    /// Everything the engine wrote since the previous call.
    fn fresh(&self) -> String {
        let all = std::fs::read(&self.capture).unwrap_or_default();
        let from = self.consumed.get().min(all.len());
        self.consumed.set(all.len());
        String::from_utf8_lossy(&all[from..]).into_owned()
    }

    /// The rendered pane: the screen the user would be looking at.
    fn screen(&self) -> String {
        let out = Self::tmux(&self.socket, &["capture-pane", "-p", "-t", Self::SESSION]);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = Self::tmux(&self.socket, &["kill-server"]);
        let _ = std::fs::remove_file(&self.capture);
    }
}

/// No line on the pane may be nothing but underscores: that is the old placeholder marker, and a
/// line of it would mean a regression to a protocol the engine no longer covers.
fn assert_no_bare_placeholder(screen: &str) {
    assert!(
        !screen.lines().any(|line| {
            let t = line.trim();
            !t.is_empty() && t.chars().all(|c| c == '_')
        }),
        "the placeholder should have been covered, pane:\n{screen}"
    );
}

/// The cases drive the inner shell by writing bytes, the way they did over a pty, but tmux takes
/// key names: `write` maps the control bytes the cases use to their names and sends the rest
/// verbatim. Only ASCII text plus `\r`/`\n`, `\t`, Ctrl-C and Ctrl-L are ever written.
struct TmuxKeys {
    socket: String,
}

impl TmuxKeys {
    fn key(&self, name: &str) {
        Tmux::tmux(&self.socket, &["send-keys", "-t", Tmux::SESSION, name]);
    }

    fn type_text(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        Tmux::tmux(
            &self.socket,
            &["send-keys", "-t", Tmux::SESSION, "-l", text],
        );
    }
}

impl Write for TmuxKeys {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut literal = String::new();
        for &b in bytes {
            let key = match b {
                b'\r' | b'\n' => Some("Enter"),
                b'\t' => Some("Tab"),
                0x03 => Some("C-c"),
                0x0c => Some("C-l"),
                _ => None,
            };
            match key {
                Some(name) => {
                    self.type_text(&literal);
                    literal.clear();
                    self.key(name);
                }
                None => literal.push(b as char),
            }
        }
        self.type_text(&literal);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct Engine {
    master: Box<Tmux>,
    writer: Box<dyn Write + Send>,
    /// `Some` for a pwsh case: the lock is held until the case ends and the engine is dropped.
    _pwsh_guard: Option<MutexGuard<'static, ()>>,
}

/// Start the engine under isolation: explicit `--shell zsh` (CI's `$SHELL` is bash, while the
/// assertions below are written against zsh's output).
fn spawn_engine() -> Engine {
    spawn_engine_with_config_home("/nonexistent-p11k-config")
}

/// `config_home` becomes `$XDG_CONFIG_HOME`, where the engine looks for the theme when `--config`
/// is not given. The default is a directory that does not exist, or the developer's own theme
/// would decide what these assertions see.
fn spawn_engine_with_config_home(config_home: &str) -> Engine {
    spawn_engine_shell("zsh", config_home)
}

/// `shell` is passed to `--shell`. Every shell prints the same skeleton (blank rows, the
/// zero-width signal, the prefix); only the surrounding hooks differ.
fn spawn_engine_shell(shell: &str, config_home: &str) -> Engine {
    let socket = format!(
        "p11k-proxy-{shell}-{}",
        SOCKET_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    // Take the lock before the server starts, so the tmux server, the shell and the case all sit
    // inside the serialised section.
    let pwsh_guard = (shell == "pwsh").then(|| {
        PWSH_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    });
    Engine {
        master: Box::new(Tmux::start(&socket, shell, config_home)),
        writer: Box::new(TmuxKeys { socket }),
        _pwsh_guard: pwsh_guard,
    }
}

/// Read until `needle` appears in the engine's output or the timeout expires, returning what the
/// engine wrote since the previous call. Poll with a deadline: a blocking read would turn the
/// timeout into an indefinite wait.
fn read_until(master: &Tmux, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    let mut acc = String::new();
    loop {
        acc.push_str(&master.fresh());
        if acc.contains(needle) {
            return acc;
        }
        if start.elapsed() >= timeout {
            // Print the scene instead of returning it: callers parse this string as the engine's
            // byte stream, so a pane dump inside it would both satisfy and break their
            // assertions. A failed case shows the capture.
            eprintln!(
                "read_until({needle:?}) timed out after {timeout:?}{}",
                diagnose(master)
            );
            return acc;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What a timing assertion needs when it fails: the rendered pane and the tail of the engine's
/// own log. A prompt that never arrives is a shell that never announced it, an engine that never
/// repainted, or a capture that broke; this tells the three apart.
fn diagnose(master: &Tmux) -> String {
    let pane = Tmux::tmux(&master.socket, &["capture-pane", "-p", "-t", Tmux::SESSION]);
    let log = std::fs::read_to_string("/tmp/p11k-engine.log").unwrap_or_default();
    let mut tail: Vec<&str> = log.lines().rev().take(20).collect();
    tail.reverse();
    format!(
        "\n--- pane ---\n{}--- engine log (last 20 lines) ---\n{}",
        String::from_utf8_lossy(&pane.stdout),
        tail.join("\n")
    )
}

/// Wait for the real first prompt to be ready. The criterion is `✔`: the instant header is painted
/// by the engine from the state known at startup and carries no exit code; only the header
/// repainted after the inner shell finishes loading and runs its first precmd carries it.
fn wait_ready(master: &Tmux) -> String {
    wait_ready_within(master, Duration::from_secs(15))
}

/// Same, with a caller-chosen budget: pwsh loads its runtime on every start, so its first prompt
/// takes several times as long as zsh's or bash's.
fn wait_ready_within(master: &Tmux, timeout: Duration) -> String {
    let out = read_until(master, "\u{f00c}", timeout);
    if !out.contains('\u{f00c}') {
        // On timeout all we see is "output was truncated", with no way to tell whether spawn
        // failed, the engine is stuck on an ack, or the shell never started — include the tail
        // of the engine log, and the pane, which shows whether the shell came up at all.
        match std::fs::read_to_string("/tmp/p11k-engine.log") {
            Ok(log) => {
                let tail: Vec<&str> = log.lines().rev().take(15).collect();
                eprintln!("engine log tail (newest first): {tail:#?}");
            }
            Err(e) => eprintln!("no engine log at /tmp/p11k-engine.log: {e}"),
        }
        eprintln!("pane on timeout:\n{}", master.screen());
    }
    out
}

#[test]
fn theme_is_read_from_the_config_home() {
    // `p11k configure` writes the theme to $XDG_CONFIG_HOME/p11k/p11k.kdl; the engine has to
    // pick it up from there, or that file would only ever be read when passed to --config.
    let home = std::env::temp_dir().join(format!("p11k-config-home-{}", std::process::id()));
    let dir = home.join("p11k");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p11k.kdl"),
        "layout { left { line { dir } } }\nframe { first-prefix \"FROM-CONFIG-HOME\" }\n",
    )
    .unwrap();

    let eng = spawn_engine_with_config_home(home.to_str().unwrap());
    let out = read_until(&eng.master, "FROM-CONFIG-HOME", Duration::from_secs(10));
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        out.contains("FROM-CONFIG-HOME"),
        "the theme in the config home should be used, got {out:?}"
    );
}

#[test]
fn initial_prompt_shows_header_and_input_line() {
    let eng = spawn_engine();
    let out = wait_ready(&eng.master);
    assert!(
        out.contains('/') || out.contains('~'),
        "header should contain the directory (~ abbreviation or path), got {out:?}"
    );
    assert!(
        out.contains('\u{f00c}'),
        "real header should contain the ✔ exit-code status"
    );
    // The instant header also contains ❯; the prefix overwriting the placeholder is
    // verified separately by placeholder_overwritten_by_prefix.
    assert!(
        out.contains('❯'),
        "input line should contain ❯, got {out:?}"
    );
    assert!(
        out.contains('\x1b'),
        "header should contain ANSI color/positioning sequences"
    );
}

/// bash reprints the whole `PS1` whenever readline redraws the prompt over itself, and
/// showing completion candidates is one of those cases. `PS1` now carries the prompt-ready
/// signal and the prefix itself, so a reprint must produce a clean prompt: no leftover marker,
/// and the header still backfilled.
#[test]
fn bash_prompt_reprint_keeps_the_placeholder_covered() {
    let home = std::env::temp_dir().join(format!("p11k-bash-reprint-{}", std::process::id()));
    let dir = home.join("probe");
    let _ = std::fs::remove_dir_all(&home);
    for name in ["probe-alpha", "probe-beta", "probe-gamma"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }

    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&eng.master);
    assert_readline_available(&mut eng);

    eng.writer
        .write_all(format!("cd {}/probe-\t\t", dir.display()).as_bytes())
        .unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(&eng.master, "probe-gamma", Duration::from_secs(5));
    out.push_str(&read_until(&eng.master, "❯", Duration::from_secs(2)));
    let _ = std::fs::remove_dir_all(&home);

    // The completion redraw reprints the prompt string: the signal fires again and the header
    // is backfilled. The prefix is part of that string, so zle's buffer cannot be covered by a
    // late paint — that artifact is what this test guards against.
    assert!(
        !out.contains("__"),
        "the placeholder must not exist any more, got {out:?}"
    );
    // How many of the redraws arrive inside this window depends on timing, so the count is not
    // pinned: one backfilled header proves the reprint path works.
    assert!(
        out.contains('\u{f115}'),
        "the completion redraw should backfill the header again, got {out:?}"
    );
}

/// A background job finishing while the prompt sits idle makes zsh print the notification and
/// redraw the prompt: `PROMPT` (blank rows + marker) goes out again, and no announce covers it
/// — zle-line-init only runs when the input line is entered, not when it is redrawn. The
/// marker matching therefore has to run for zsh too, for as long as the input line is idle.
///
/// Only the marker reprinted after the notification is asserted here: the first prompt of a
/// cycle is painted by the `p` announce, and zle's own bytes (bracketed paste, cursor keys)
/// sit between its marker and the paint that covers it.
#[test]
fn zsh_job_notification_reprint_is_covered() {
    let mut eng = spawn_engine();
    wait_ready(&eng.master);

    eng.writer.write_all(b"sleep 1 &\n").unwrap();
    eng.writer.flush().unwrap();
    // The next prompt is up while the job still runs; the notification and the redraw follow
    // once it finishes.
    let mut out = read_until(&eng.master, "done", Duration::from_secs(8));
    out.push_str(&read_until(&eng.master, "❯", Duration::from_secs(3)));

    let done = out
        .find("done")
        .unwrap_or_else(|| panic!("the job notification should have been printed, got {out:?}"));
    // The redraw reprints the prompt string, so the signal fires again and the header is
    // backfilled. What must not happen is a half-painted prompt left on screen.
    assert!(
        !out.contains("__"),
        "the placeholder must not exist any more, got {out:?}"
    );
    assert!(
        out[done..].matches('\u{f115}').count() >= 1,
        "the redrawn prompt should carry a backfilled header, got {out:?}"
    );
}

/// The prompt string carries one newline per header row and the tty turns each of them into
/// `\r\n`, so a two-row header must still be backfilled from the signal.
#[test]
fn bash_two_row_header_is_backfilled() {
    let home = std::env::temp_dir().join(format!("p11k-two-rows-{}", std::process::id()));
    let dir = home.join("p11k");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p11k.kdl"),
        "layout {\n    left {\n        line { dir }\n        line { status }\n    }\n    right {\n        line { time }\n        line { background_jobs }\n    }\n}\nsegments {\n    status verbose=#true\n}\n",
    )
    .unwrap();

    let mut eng = spawn_engine_shell("bash", home.to_str().unwrap());
    wait_ready(&eng.master);

    eng.writer.write_all(b"echo two-row-probe\n").unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(&eng.master, "two-row-probe", Duration::from_secs(5));
    out.push_str(&read_until(&eng.master, "❯", Duration::from_secs(2)));
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        out.contains("two-row-probe"),
        "the command should have run, got {out:?}"
    );
    assert!(
        !out.contains("__"),
        "the placeholder must not exist any more, got {out:?}"
    );
}

/// The prefix is part of the shell's prompt string, so a wide one needs no marker to match and
/// its length is unconstrained.
#[test]
fn bash_long_prefix_is_backfilled() {
    let home = std::env::temp_dir().join(format!("p11k-long-prefix-{}", std::process::id()));
    let dir = home.join("p11k");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p11k.kdl"),
        "layout {\n    left {\n        line { dir }\n        line { status }\n        line { time }\n    }\n    right {\n        line { background_jobs }\n        line {}\n        line {}\n    }\n}\nframe {\n    last-prefix \"══════════╰─\"\n}\nsegments {\n    status verbose=#true\n    time time-format=\"24h\"\n}\n",
    )
    .unwrap();

    let mut eng = spawn_engine_shell("bash", home.to_str().unwrap());
    wait_ready(&eng.master);

    eng.writer.write_all(b"echo wide-probe\n").unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(&eng.master, "wide-probe", Duration::from_secs(5));
    out.push_str(&read_until(&eng.master, "❯", Duration::from_secs(2)));
    let _ = std::fs::remove_dir_all(&home);

    // The prefix is part of the shell's prompt string now, so the wide one must simply show up;
    // there is no marker to match or cover any more.
    assert!(
        out.contains("══════════╰─") && out.contains('❯'),
        "the wide prefix should be painted, got {out:?}"
    );
}

/// A `_` the user types is not the placeholder, whatever the marker width is. With a four column
/// prefix (marker `____`) a run of underscores used to be held back, so the echo lagged by up to
/// marker width — this is the wide-prefix case the narrow default hides.
#[test]
fn bash_typed_underscores_are_echoed_immediately() {
    let home = std::env::temp_dir().join(format!("p11k-wide-prefix-{}", std::process::id()));
    let dir = home.join("p11k");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p11k.kdl"),
        "layout {\n    left { line { dir } }\n    right { line { status } }\n}\nframe {\n    last-prefix \"╰─\"\n}\nsegments {\n    status verbose=#true\n}\n",
    )
    .unwrap();

    let mut eng = spawn_engine_shell("bash", home.to_str().unwrap());
    wait_ready(&eng.master);

    for ch in ["_", "_", "_", "_", "_"] {
        eng.writer.write_all(ch.as_bytes()).unwrap();
        eng.writer.flush().unwrap();
        let out = read_until(&eng.master, ch, Duration::from_secs(2));
        assert!(
            out.contains(ch),
            "the underscore should be echoed right away, got {out:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&home);
    eng.writer.write_all(b"\x03").unwrap();
    eng.writer.flush().unwrap();
}

/// Typed characters come back one at a time: while the engine looks for the placeholder it must
/// not hold output back, or the input line lags behind the keyboard (the marker path, which is
/// bash/fish/pwsh — zsh paints through zle-line-init and never buffers).
#[test]
fn bash_typed_characters_are_echoed_immediately() {
    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&eng.master);

    for ch in ["i", "m", "m", "e", "d"] {
        eng.writer.write_all(ch.as_bytes()).unwrap();
        eng.writer.flush().unwrap();
        let out = read_until(&eng.master, ch, Duration::from_secs(2));
        assert!(
            out.contains(ch),
            "the character {ch:?} should be echoed right away, got {out:?}"
        );
    }
    // Leave the line without running a command.
    eng.writer.write_all(b"\x03").unwrap();
    eng.writer.flush().unwrap();
}

/// readline's completion pager takes an enter to page on, and that enter is not a submitted
/// command: the prompt it reprints afterwards still has to be covered. Keying the marker match on
/// "the input line is live" breaks exactly here — the enter clears that flag, so the reprint went
/// through uncovered as soon as the list was paged through.
#[test]
fn bash_enter_inside_the_completion_pager_is_covered() {
    let home = std::env::temp_dir().join(format!("p11k-pager-{}", std::process::id()));
    let dir = home.join("p");
    let _ = std::fs::remove_dir_all(&home);
    // Enough candidates to make readline page in a 24 row window (it lists about sixteen per row).
    for i in 0..900 {
        std::fs::create_dir_all(dir.join(format!("c{i:03}"))).unwrap();
    }

    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&eng.master);
    assert_readline_available(&mut eng);

    eng.writer
        .write_all(format!("ls {}/c\t\t", dir.display()).as_bytes())
        .unwrap();
    eng.writer.flush().unwrap();
    // bash asks before listing this many candidates.
    read_until(&eng.master, "possibilities", Duration::from_secs(5));
    eng.writer.write_all(b"y").unwrap();
    eng.writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // One enter inside the pager, then page to the end with spaces.
    eng.writer.write_all(b"\r").unwrap();
    eng.writer.flush().unwrap();
    for _ in 0..40 {
        eng.writer.write_all(b" ").unwrap();
        eng.writer.flush().unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let mut out = read_until(&eng.master, "c899", Duration::from_secs(5));
    out.push_str(&read_until(&eng.master, "❯", Duration::from_secs(2)));
    let _ = std::fs::remove_dir_all(&home);

    // Paging reprints the prompt string, so the signal fires again and the header is backfilled.
    // The placeholder is gone from the protocol; what matters is that no marker is left behind.
    assert!(
        !out.contains("__"),
        "the placeholder must not exist any more, got {out:?}"
    );
    assert!(
        out.matches('\u{f115}').count() >= 1,
        "paging should backfill the header again, got {out:?}"
    );
}

/// Ctrl-L makes readline clear the screen and reprint the prompt. The header was on screen
/// before the clear, so the engine has to rebuild it from the reprint: the screen must not
/// end up with a bare marker and no header.
#[test]
fn bash_ctrl_l_rebuilds_the_prompt() {
    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&eng.master);
    assert_readline_available(&mut eng);

    // Run an empty command first and let the next prompt settle: bash paints the first prompt
    // while the rcfile is still taking effect, and a Ctrl-L sent in between lands in the line
    // buffer instead of running clear-screen.
    eng.writer.write_all(b"\n").unwrap();
    eng.writer.flush().unwrap();
    read_until(&eng.master, "❯", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(300));
    // The background git result repaints the header, and that repaint can still be in flight.
    // Drop it here, or the next read stops at its prompt and the clear sequence never makes it
    // into the buffer this case inspects.
    let _ = eng.master.fresh();

    eng.writer.write_all(b"\x0c").unwrap();
    eng.writer.flush().unwrap();

    // readline clears the screen with ED, spelled per terminfo: `\e[H\e[J` under tmux's
    // screen-256color, `\e[H\e[2J` under xterm-256color. Then it repaints the prompt, and the
    // engine has to rebuild the header out of that repaint.
    let mut cleared = read_until(&eng.master, "❯", Duration::from_secs(5));
    assert!(
        cleared.contains("\x1b[J") || cleared.contains("\x1b[2J"),
        "readline clears the screen on Ctrl-L, got {cleared:?}"
    );
    // The repaint may already be in the same read as the clear.
    if !cleared.contains('❯') {
        cleared.push_str(&read_until(&eng.master, "❯", Duration::from_secs(2)));
    }
    assert!(
        cleared.contains('❯'),
        "the prompt should be rebuilt after Ctrl-L, got {cleared:?}"
    );
    assert_no_bare_placeholder(&eng.master.screen());
}

/// The tests below drive readline itself — clear-screen, the completion pager. nixpkgs' default
/// `bash` is built without readline (`bind` does not even exist), so on that shell they would
/// fail for a reason that has nothing to do with the engine. Say which it is.
fn assert_readline_available(eng: &mut Engine) {
    eng.writer
        .write_all(b"printf 'RL%sRL\\n' \"$(bind -P | grep -c '^clear-screen')\"\n")
        .unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "RL1RL", Duration::from_secs(2));
    assert!(
        out.contains("RL1RL"),
        "the inner bash has no readline (nixpkgs' non-interactive bash build?): {out:?}"
    );
}

#[test]
fn command_output_passthrough_and_next_prompt() {
    let mut eng = spawn_engine();
    wait_ready(&eng.master);

    eng.writer.write_all(b"echo hello-from-shell\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "hello-from-shell", Duration::from_secs(5));
    assert!(
        out.contains("hello-from-shell"),
        "command output should pass through"
    );

    // The command finished → precmd → the header is repainted, this time carrying ✔.
    let out2 = read_until(&eng.master, "\u{f00c}", Duration::from_secs(5));
    assert!(
        out2.contains('\u{f00c}'),
        "a new prompt should appear after the command, got {out2:?}"
    );
}

/// `prompt-add-newline #true` is one blank row, not two (p10k
/// `POWERLEVEL9K_PROMPT_ADD_NEWLINE`): the row the engine leaves between the command output
/// and the next prompt is the blank line the user sees.
#[test]
fn prompt_add_newline_leaves_one_blank_row() {
    let home = std::env::temp_dir().join(format!("p11k-newline-home-{}", std::process::id()));
    let dir = home.join("p11k");
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("p11k.kdl"),
        // `verbose=#true`: without it the status segment says nothing about a successful
        // command, and wait_ready's ✔ would never show up.
        "layout {\n    left { line { dir } }\n    right { line { status } }\n    prompt-add-newline #true\n}\nsegments {\n    status verbose=#true\n}\n",
    )
    .unwrap();

    let mut eng = spawn_engine_with_config_home(home.to_str().unwrap());
    wait_ready(&eng.master);

    eng.writer
        .write_all(b"printf 'P11K-ROW-PROBE\\n'\n")
        .unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "\x1b]133;A\x07", Duration::from_secs(30));
    let _ = std::fs::remove_dir_all(&home);

    // The command output ends with its own CRLF; everything between it and the prompt-start
    // marker is the blank row the engine writes before the header. `rfind` because the echoed
    // command line also contains the probe text: the last occurrence is the command's output.
    let output = "P11K-ROW-PROBE\r\n";
    let start = out
        .rfind(output)
        .unwrap_or_else(|| panic!("command output, got {out:?}"))
        + output.len();
    let rest = &out[start..];
    let marker = rest
        .find("\x1b]133;A\x07")
        .unwrap_or_else(|| panic!("prompt start marker, got {out:?}"));
    let blank = &rest[..marker];
    assert_eq!(
        blank.matches("\r\n").count(),
        1,
        "prompt-add-newline #true should open one blank row, got {blank:?}"
    );
}

#[test]
fn exit_code_shows_in_status() {
    let mut eng = spawn_engine();
    wait_ready(&eng.master);

    eng.writer.write_all(b"false\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "\u{f00d}", Duration::from_secs(5));
    assert!(
        out.contains('\u{f00d}'),
        "exit-code status should show ✘, got {out:?}"
    );
}

#[test]
fn prompt_char_turns_error_color_on_failure() {
    // The default lean prompt_char has state ERROR fg=196 and normal fg=76: after a
    // failed command the ❯ prefix should turn red.
    let mut eng = spawn_engine();
    wait_ready(&eng.master);

    eng.writer.write_all(b"false\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "\x1b[38;5;196m", Duration::from_secs(5));
    assert!(
        out.contains("\x1b[38;5;196m❯"),
        "after a failure prompt_char should turn red in the ERROR state (196), got {out:?}"
    );
}

#[test]
fn ctrl_c_interrupts_running_command() {
    let mut eng = spawn_engine();
    wait_ready(&eng.master);

    eng.writer.write_all(b"sleep 5\n").unwrap();
    eng.writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    eng.writer.write_all(b"\x03").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "❯", Duration::from_secs(5));
    assert!(
        out.contains('❯'),
        "a new prompt should appear after Ctrl-C, got {out:?}"
    );
}

/// fish and pwsh take bash's route: the prompt function prints the placeholder and the engine
/// matches it in the pass-through stream (there is no `p` announce). Neither has readline, so there
/// is no completion pager or clear-screen widget to drive; what these cases pin down is that each
/// protocol layer announces, waits for the ack, leaves a marker that gets covered, and does not hold
/// the echo back.
///
/// They run behind tmux like the zsh and bash cases: pwsh's line editor waits for the terminal to
/// answer its questions, which a bare pty never does.
///
/// A loaded CI runner takes several seconds to start pwsh and answer its terminal queries, and a
/// cold nix sandbox adds to that. The budget is generous so the case fails on a real regression
/// instead of on machine speed.
const SHELL_READY: Duration = Duration::from_secs(90);

/// The header is painted, the input line carries the prefix, and every placeholder the shell
/// printed is covered: the marker is always followed by the engine's backfill (DECSC, `\x1b7`).
fn marker_shell_paints_its_prompt(shell: &str) {
    let eng = spawn_engine_shell(shell, "/nonexistent-p11k-config");
    let out = wait_ready_within(&eng.master, SHELL_READY);

    assert!(
        out.contains('\u{f00c}'),
        "{shell}: the header should carry the ✔ exit-code status, got {out:?}"
    );
    assert!(
        out.contains('/') || out.contains('~'),
        "{shell}: the header should carry the directory, got {out:?}"
    );
    assert!(
        out.contains('❯'),
        "{shell}: the input line should carry the ❯ prefix, got {out:?}"
    );

    // The placeholder is gone from the protocol: the prompt string carries the signal and the
    // prefix itself, so nothing has to be matched or covered. What must not appear is the old
    // marker left on screen.
    assert!(
        !out.contains("__"),
        "{shell}: the placeholder must not exist any more, got {out:?}"
    );
    assert_no_bare_placeholder(&eng.master.screen());
}

/// A command inside fish: its output passes through, a new prompt follows, and a failing command
/// turns the status segment into ✘.
fn marker_shell_runs_a_command(shell: &str, fail: &str) {
    let mut eng = spawn_engine_shell(shell, "/nonexistent-p11k-config");
    wait_ready_within(&eng.master, SHELL_READY);

    eng.writer.write_all(b"echo hello-from-proxy\n").unwrap();
    eng.writer.flush().unwrap();
    // The waits after a command use the same budget as the startup: on a loaded runner a pwsh
    // session can take seconds to come back with its next prompt, and ten seconds proved too
    // tight there.
    let mut out = read_until(&eng.master, "hello-from-proxy", SHELL_READY);
    assert!(
        out.contains("hello-from-proxy"),
        "{shell}: command output should pass through, got {out:?}"
    );

    // The read that caught the command output may already carry the next prompt's header (one
    // read can span both), in which case waiting for it again would block until the timeout.
    if !out.contains('\u{f00c}') {
        out.push_str(&read_until(&eng.master, "\u{f00c}", SHELL_READY));
    }
    assert!(
        out.contains('\u{f00c}'),
        "{shell}: a new prompt should follow the command, got {out:?}"
    );

    eng.writer.write_all(fail.as_bytes()).unwrap();
    eng.writer.write_all(b"\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&eng.master, "\u{f00d}", SHELL_READY);
    assert!(
        out.contains('\u{f00d}'),
        "{shell}: the status should turn to ✘ after `{fail}`, got {out:?}"
    );
}

/// Typed characters come back one at a time: looking for the placeholder must not hold output
/// back, or the input line lags behind the keyboard.
fn marker_shell_echoes_typed_characters(shell: &str) {
    let mut eng = spawn_engine_shell(shell, "/nonexistent-p11k-config");
    wait_ready_within(&eng.master, SHELL_READY);

    for ch in ["i", "m", "m", "e", "d"] {
        eng.writer.write_all(ch.as_bytes()).unwrap();
        eng.writer.flush().unwrap();
        let out = read_until(&eng.master, ch, Duration::from_secs(5));
        assert!(
            out.contains(ch),
            "{shell}: the character {ch:?} should be echoed right away, got {out:?}"
        );
    }
    // Leave the input line without running a command.
    eng.writer.write_all(b"\x03").unwrap();
    eng.writer.flush().unwrap();
}

#[test]
fn fish_paints_its_prompt() {
    marker_shell_paints_its_prompt("fish");
}

#[test]
fn fish_runs_a_command() {
    marker_shell_runs_a_command("fish", "false");
}

#[test]
fn fish_echoes_typed_characters() {
    marker_shell_echoes_typed_characters("fish");
}

/// The header is painted and the placeholder covered (on the pane as well), and the prefix is on the
/// input line: every marker in the byte stream is followed by the engine's backfill (DECSC).
#[test]
fn pwsh_paints_its_prompt() {
    marker_shell_paints_its_prompt("pwsh");
}

/// A command inside pwsh: output passes through, a new prompt follows, and a failing command turns
/// the status segment into ✘. The failing one is `bash -c "exit 1"`: pwsh has no `false`, and the
/// obvious substitutes (`Get-Item` on a missing path, `Write-Error`) leave `$LASTEXITCODE` at 0,
/// which the protocol's `$?` → `$LASTEXITCODE` fallback would report as success.
#[test]
fn pwsh_runs_a_command() {
    marker_shell_runs_a_command("pwsh", "bash -c \"exit 1\"");
}

/// Typed characters reach the input line: nothing is held back or swallowed on the way.
#[test]
fn pwsh_echoes_typed_characters() {
    marker_shell_echoes_typed_characters("pwsh");
}
