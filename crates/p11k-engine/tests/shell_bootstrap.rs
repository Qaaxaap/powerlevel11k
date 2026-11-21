//! End-to-end tests for the shell bootstrap: the ack timeout, the announce cwd escaping and the
//! recursive-launch / failed-spawn paths.
//!
//! The shells run inside tmux, like the engine tests in `proxy.rs`. A bare pty is not enough:
//! fish asks the terminal about its colours and the DECRQSS state at startup and waits for the
//! answers, which a pty that never replies does not give. tmux answers, so the shell reaches its
//! prompt in the same way it does for the user. `pipe-pane` then hands over the bytes the
//! terminal received, which is what the assertions read.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

const COLS: &str = "120";
const ROWS: &str = "32";
const SESSION: &str = "boot";

static SOCKET_SEQ: AtomicUsize = AtomicUsize::new(0);

/// A missing program is a broken environment (the devShell provides every shell these tests use),
/// said plainly instead of surfacing as a timeout later on.
fn assert_on_path(program: &str) {
    let found = std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()));
    assert!(
        found,
        "`{program}` is not on PATH: the devShell provides it (flake.nix), so this is an \
         environment problem rather than a bootstrap one"
    );
}

/// A temporary directory removed on drop.
struct TestDir(PathBuf);

impl TestDir {
    fn new(tag: &str) -> TestDir {
        let dir = std::env::temp_dir().join(format!(
            "p11k-boot-{tag}-{}-{}",
            std::process::id(),
            SOCKET_SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("the test directory is writable");
        TestDir(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmux(socket: &str, args: &[&str]) -> Output {
    Command::new("tmux")
        .arg("-L")
        .arg(socket)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("tmux cannot be run (the devShell provides it): {e}"))
}

fn new_session(socket: &str, cmd: &str) {
    let _ = tmux(socket, &["kill-server"]);
    let out = tmux(
        socket,
        &[
            "new-session",
            "-d",
            "-s",
            SESSION,
            "-x",
            COLS,
            "-y",
            ROWS,
            "-e",
            "TERM=screen",
            cmd,
        ],
    );
    assert!(
        out.status.success(),
        "tmux new-session failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn pipe_pane(socket: &str, capture: &Path) {
    let _ = std::fs::remove_file(capture);
    let out = tmux(
        socket,
        &[
            "pipe-pane",
            "-t",
            SESSION,
            "-o",
            &format!("cat >> '{}'", capture.display()),
        ],
    );
    assert!(
        out.status.success(),
        "tmux pipe-pane failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn send_keys(socket: &str, text: &str) {
    // Text and the Enter key have to be separate calls: `send-keys -l` treats every argument as
    // a literal string. The pause lets the line editor take the text before the Enter arrives;
    // C-m is the carriage return zle executes, and does not depend on the Enter key name.
    let out = tmux(socket, &["send-keys", "-t", SESSION, "-l", text]);
    assert!(out.status.success(), "tmux send-keys failed");
    std::thread::sleep(Duration::from_millis(300));
    let out = tmux(socket, &["send-keys", "-t", SESSION, "C-m"]);
    assert!(out.status.success(), "tmux send-keys C-m failed");
}

/// Whether the tmux server still has the session; false once the pane's process exited.
fn session_alive(socket: &str) -> bool {
    tmux(socket, &["has-session", "-t", SESSION])
        .status
        .success()
}

/// Wait until the pane's process exits (the session disappears) or the timeout expires.
fn wait_for_session_end(socket: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !session_alive(socket) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn read_capture(capture: &Path) -> String {
    std::fs::read_to_string(capture).unwrap_or_default()
}

/// Poll the captured bytes until `needle` appears or the timeout expires.
fn capture_until(capture: &Path, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = read_capture(capture);
        if text.contains(needle) || start.elapsed() >= timeout {
            return text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The rendered screen of the session (`tmux capture-pane -p`). The screen has no pipe buffer:
/// what the terminal shows is what the shell has already done, which matters for the
/// "the shell still answers" checks.
fn screen(socket: &str) -> String {
    let out = tmux(socket, &["capture-pane", "-p", "-t", SESSION]);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Poll the rendered screen until `needle` shows up `count` times or the timeout expires.
fn screen_until(socket: &str, needle: &str, count: usize, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = screen(socket);
        if text.matches(needle).count() >= count || start.elapsed() >= timeout {
            return text;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A minimal shell environment for one test: the session's state/config homes, the user rc that
/// must not load the developer's own files, and the announce/ack paths.
struct Sandbox {
    socket: String,
    dir: TestDir,
    state: PathBuf,
    config: PathBuf,
    announce: PathBuf,
    ack: PathBuf,
    rc: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let dir = TestDir::new(tag);
        let state = dir.path().join("state");
        let config = dir.path().join("config");
        let announce = dir.path().join("announce");
        let ack = dir.path().join("ack");
        let rc = dir.path().join("rc");
        std::fs::create_dir_all(&state).expect("state dir");
        std::fs::create_dir_all(&config).expect("config dir");
        let socket = format!(
            "p11k-boot-{tag}-{}-{}",
            std::process::id(),
            SOCKET_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        Sandbox {
            socket,
            dir,
            state,
            config,
            announce,
            ack,
            rc,
        }
    }

    fn bin() -> &'static str {
        env!("CARGO_BIN_EXE_p11k")
    }

    /// Load the engine's generated rc for `shell` into the sandbox by starting the engine once and
    /// copying the template out of its run directory. The engine writes the rc there and removes
    /// it on exit, so the copy has to happen while it runs.
    fn copy_engine_rc(&mut self, shell: &str) {
        assert_on_path("tmux");
        assert_on_path(shell);
        let work = self.dir.path().join("engine");
        std::fs::create_dir_all(&work).expect("engine work dir");
        let cmd = format!(
            "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; sleep 0.3; cd '{}' && XDG_STATE_HOME='{}' \
             XDG_CONFIG_HOME='{}' P11K_USER_ZSHRC=/dev/null P11K_USER_PROFILE=/dev/null \
             exec '{}' --shell {shell}",
            work.display(),
            self.state.display(),
            self.config.display(),
            Self::bin()
        );
        new_session(&self.socket, &cmd);
        // Wait for the run directory and the shell's rc file.
        let rel = match shell {
            "zsh" => ".zshrc",
            "bash" => ".bashrc",
            "fish" => "fish/config.fish",
            "pwsh" => "p11k.ps1",
            other => panic!("unknown shell {other}"),
        };
        let runs = self.state.join("p11k");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut found: Option<PathBuf> = None;
        while Instant::now() < deadline {
            if let Ok(entries) = std::fs::read_dir(&runs) {
                for entry in entries.flatten() {
                    let candidate = entry.path().join(rel);
                    if candidate.is_file() {
                        found = Some(candidate);
                        break;
                    }
                }
            }
            if found.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let template = found.unwrap_or_else(|| panic!("the engine wrote no rc for {shell}"));
        std::fs::create_dir_all(self.rc.clone()).expect("rc dir");
        let dest = self.rc.join(rel);
        std::fs::create_dir_all(dest.parent().unwrap()).expect("rc parent");
        std::fs::copy(&template, &dest).expect("copy the rc template");
        let _ = tmux(&self.socket, &["kill-server"]);
    }

    /// Start the shell with the copied rc, a fake announce path and no ack file: the shell has to
    /// fall back to its cached prompt shape after the timeout.
    /// The command line that starts `shell` with the copied rc, a fake announce path and this
    /// sandbox's ack path.
    fn shell_cmd(&self, shell: &str) -> String {
        self.shell_cmd_env(shell, &[])
    }

    /// Like [`Self::shell_cmd`] with extra environment entries, such as a locale.
    fn shell_cmd_env(&self, shell: &str, extra: &[(&str, &str)]) -> String {
        let extra: String = extra
            .iter()
            .map(|(key, value)| format!("{key}='{value}' "))
            .collect();
        match shell {
            "zsh" => format!(
                "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; cd '{}' && env {extra}ZDOTDIR='{}' \
                 P11K_ANNOUNCE='{}' P11K_ACK='{}' P11K_PREFIX_OK='OK> ' P11K_PREFIX_ERR='ERR> ' \
                 P11K_USER_ZSHRC=/dev/null zsh -i",
                self.dir.path().display(),
                self.rc.display(),
                self.announce.display(),
                self.ack.display(),
            ),
            "bash" => format!(
                "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; cd '{}' && env {extra}P11K_ANNOUNCE='{}' \
                 P11K_ACK='{}' P11K_PREFIX_OK='OK> ' P11K_PREFIX_ERR='ERR> ' \
                 P11K_USER_ZSHRC=/dev/null bash --rcfile '{}' -i",
                self.dir.path().display(),
                self.announce.display(),
                self.ack.display(),
                self.rc.join(".bashrc").display(),
            ),
            "fish" => format!(
                "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; cd '{}' && env {extra}XDG_CONFIG_HOME='{}' \
                 P11K_ANNOUNCE='{}' P11K_ACK='{}' P11K_PREFIX_OK='OK> ' P11K_PREFIX_ERR='ERR> ' \
                 fish -i",
                self.dir.path().display(),
                self.rc.display(),
                self.announce.display(),
                self.ack.display(),
            ),
            other => panic!("unsupported shell {other}"),
        }
    }

    /// Start the shell with the copied rc, a fake announce path and no ack file: the shell has to
    /// fall back to its cached prompt shape after the timeout.
    fn start_shell_without_ack(&self, shell: &str) {
        new_session(&self.socket, &self.shell_cmd(shell));
    }

    /// Start the shell with a pre-written ack: the prompt must take its prefix without waiting.
    fn start_shell_with_ack(&self, shell: &str) {
        // Header rows, fold flag, the row the user types on (left and right) and the right indent.
        std::fs::write(&self.ack, "1\n1\nACK> \n\n1\n").expect("write the ack");
        new_session(&self.socket, &self.shell_cmd(shell));
    }
}

/// The ack timeout has to keep the shell usable: with no ack file the prompt still appears, the
/// fallback prefix is used, the warning is printed once, and the shell answers a command.
#[test]
fn an_unanswered_ack_does_not_block_the_shell() {
    for shell in ["zsh", "bash", "fish"] {
        let mut sandbox = Sandbox::new(&format!("timeout-{shell}"));
        sandbox.copy_engine_rc(shell);
        sandbox.start_shell_without_ack(shell);
        // The rendered screen, not the pipe-pane file: the screen is the shell's own output and
        // cannot lag behind the process under test.
        // `capture-pane` trims the trailing spaces of a line, so the prefix is checked without
        // its trailing space.
        let text = screen_until(&sandbox.socket, "OK>", 1, Duration::from_secs(10));
        assert!(
            text.contains("OK>"),
            "{shell}: the prompt did not fall back to the cached prefix after the ack timeout; \
             screen: {text:?}"
        );
        let text = screen_until(
            &sandbox.socket,
            "did not answer in time",
            1,
            Duration::from_secs(5),
        );
        assert!(
            text.contains("did not answer in time"),
            "{shell}: the ack timeout was not reported; screen: {text:?}"
        );
        send_keys(&sandbox.socket, "echo BOOT-ALIVE");
        let text = screen_until(&sandbox.socket, "BOOT-ALIVE", 2, Duration::from_secs(5));
        assert!(
            text.matches("BOOT-ALIVE").count() >= 2,
            "{shell}: the shell stopped answering after the timeout; screen: {text:?}"
        );
        let _ = tmux(&sandbox.socket, &["kill-server"]);
    }
}

/// A written ack is read and used without waiting: the prompt prefix comes from the file and
/// the timeout path does not run. This is the path a live engine takes on every prompt.
#[test]
fn a_written_ack_sets_the_prompt_prefix() {
    for shell in ["zsh", "bash", "fish"] {
        let mut sandbox = Sandbox::new(&format!("ack-{shell}"));
        sandbox.copy_engine_rc(shell);
        sandbox.start_shell_with_ack(shell);
        let text = screen_until(&sandbox.socket, "ACK>", 1, Duration::from_secs(8));
        assert!(
            text.contains("ACK>"),
            "{shell}: the prompt did not use the prefix from the ack; screen: {text:?}"
        );
        assert!(
            !text.contains("did not answer in time"),
            "{shell}: the ack was there but the timeout path ran; screen: {text:?}"
        );
        let _ = tmux(&sandbox.socket, &["kill-server"]);
    }
}

/// zsh expands `$(…)` in PROMPT when `prompt_subst` is on. The engine renders the prompt from
/// theme text (directory and branch names), so the template turns the option off after the user
/// rc; otherwise a crafted name would run on every prompt.
#[test]
fn zsh_does_not_run_prompt_subst_from_the_row() {
    let mut sandbox = Sandbox::new("prompt-subst");
    sandbox.copy_engine_rc("zsh");
    let user_rc = sandbox.dir.path().join("user.zshrc");
    std::fs::write(&user_rc, "setopt prompt_subst\n").expect("user rc");
    let marker = sandbox.dir.path().join("pwned");
    std::fs::write(
        &sandbox.ack,
        format!("1\n1\n$(touch {})> \n\n1\n", marker.display()),
    )
    .expect("write the ack");
    // The ack carries the row, so the test needs no engine; the copied template is enough.
    let cmd = format!(
        "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; cd '{}' && \
         env ZDOTDIR='{}' P11K_ANNOUNCE='{}' P11K_ACK='{}' P11K_PREFIX_OK='OK> ' \
         P11K_PREFIX_ERR='ERR> ' P11K_USER_ZSHRC='{}' zsh -i",
        sandbox.dir.path().display(),
        sandbox.rc.display(),
        sandbox.announce.display(),
        sandbox.ack.display(),
        user_rc.display(),
    );
    new_session(&sandbox.socket, &cmd);
    let text = screen_until(&sandbox.socket, "$(touch", 1, Duration::from_secs(10));
    assert!(
        text.contains("$(touch"),
        "the row text must appear literally; screen: {text:?}"
    );
    assert!(
        !marker.exists(),
        "the prompt text must not run; screen: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// The right part of the row and its indent: fish has no `ZLE_RPROMPT_INDENT`, so the template
/// pads the right part with spaces. Nothing else in the suite reads ack lines 4-5 end to end.
#[test]
fn fish_pads_the_right_part_by_the_indent() {
    let mut sandbox = Sandbox::new("fish-right");
    sandbox.copy_engine_rc("fish");
    std::fs::write(&sandbox.ack, "1\n1\nLEFT> \nRIGHT\n3\n").expect("write the ack");
    new_session(&sandbox.socket, &sandbox.shell_cmd("fish"));
    // The first prompt takes the ack; plant it again for the prompt this case captures.
    screen_until(&sandbox.socket, "LEFT>", 1, Duration::from_secs(10));
    let capture = sandbox.dir.path().join("capture");
    pipe_pane(&sandbox.socket, &capture);
    std::fs::write(&sandbox.ack, "1\n1\nLEFT> \nRIGHT\n3\n").expect("write the ack");
    let out = tmux(&sandbox.socket, &["send-keys", "-t", SESSION, "C-m"]);
    assert!(out.status.success(), "send-keys C-m failed");
    let text = capture_until(&capture, "RIGHT", Duration::from_secs(10));
    assert!(
        text.contains("RIGHT"),
        "the right part should reach the terminal: {text:?}"
    );
    assert!(
        text.contains("RIGHT   "),
        "three columns of indent should follow the right part: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// zsh draws the right part through `RPROMPT`, and the indent goes through
/// `ZLE_RPROMPT_INDENT`, which zle applies itself. This case covers ack lines 4-5 reaching
/// `RPROMPT`; the indent itself is pinned by the template and the manual runs.
#[test]
fn zsh_prints_the_right_part() {
    let mut sandbox = Sandbox::new("zsh-right");
    sandbox.copy_engine_rc("zsh");
    std::fs::write(&sandbox.ack, "1\n1\nLEFT> \nRIGHT\n3\n").expect("write the ack");
    new_session(&sandbox.socket, &sandbox.shell_cmd("zsh"));
    // The first prompt takes the ack; plant it again for the prompt this case captures.
    screen_until(&sandbox.socket, "LEFT>", 1, Duration::from_secs(10));
    let capture = sandbox.dir.path().join("capture");
    pipe_pane(&sandbox.socket, &capture);
    std::fs::write(&sandbox.ack, "1\n1\nLEFT> \nRIGHT\n3\n").expect("write the ack");
    let out = tmux(&sandbox.socket, &["send-keys", "-t", SESSION, "C-m"]);
    assert!(out.status.success(), "send-keys C-m failed");
    let text = capture_until(&capture, "RIGHT", Duration::from_secs(10));
    assert!(
        text.contains("RIGHT"),
        "zsh should print the right part: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// The generated templates must parse with their own shell. A syntax error in a template only
/// shows up as a broken prompt at runtime, where it is much harder to place.
#[test]
fn every_template_parses_with_its_shell() {
    assert_on_path("tmux");
    for shell in ["zsh", "bash", "fish", "pwsh"] {
        assert_on_path(shell);
        let mut sandbox = Sandbox::new(&format!("syntax-{shell}"));
        sandbox.copy_engine_rc(shell);
        let rc = match shell {
            "zsh" => sandbox.rc.join(".zshrc"),
            "bash" => sandbox.rc.join(".bashrc"),
            "fish" => sandbox.rc.join("fish/config.fish"),
            "pwsh" => sandbox.rc.join("p11k.ps1"),
            other => panic!("unsupported shell {other}"),
        };
        let output = match shell {
            "zsh" => Command::new("zsh").arg("-n").arg(&rc).output(),
            "bash" => Command::new("bash").arg("-n").arg(&rc).output(),
            "fish" => Command::new("fish").arg("--no-execute").arg(&rc).output(),
            "pwsh" => Command::new("pwsh")
                .args(["-NoProfile", "-NoLogo", "-Command"])
                .arg(format!(
                    "$null = [ScriptBlock]::Create((Get-Content -Raw -LiteralPath '{}'))",
                    rc.display()
                ))
                .output(),
            other => panic!("unsupported shell {other}"),
        }
        .expect("the shell parser runs");
        assert!(
            output.status.success(),
            "{shell}: the generated template does not parse: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// The engine escapes the cwd it announces; a directory name with a tab and a newline must not
/// break the prompt. The shell stays alive, the next command runs, and the header shows the
/// complete path: with the unescaped cwd the record shifted and the path stopped at the tab.
#[test]
fn a_cwd_with_control_characters_keeps_the_prompt_alive() {
    assert_on_path("tmux");
    assert_on_path("zsh");
    let sandbox = Sandbox::new("escaping");
    let work = sandbox.dir.path().join("engine");
    std::fs::create_dir_all(&work).expect("engine work dir");
    // Children of the engine's cwd, so `cd` passes the characters through the command line. A
    // tab keeps the header on one line, so its rendering can be asserted; the newline case only
    // needs the shell to stay alive.
    let tabbed = work.join("boot\tpath");
    std::fs::create_dir_all(&tabbed).expect("tabbed dir");
    let newlined = work.join("line\nbreak");
    std::fs::create_dir_all(&newlined).expect("newline dir");
    let cmd = format!(
        "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; sleep 0.3; cd '{}' && XDG_STATE_HOME='{}' \
         XDG_CONFIG_HOME='{}' P11K_USER_ZSHRC=/dev/null P11K_USER_PROFILE=/dev/null \
         exec '{}' --shell zsh",
        work.display(),
        sandbox.state.display(),
        sandbox.config.display(),
        Sandbox::bin()
    );
    new_session(&sandbox.socket, &cmd);
    // The first prompt prints the instant header and the prefix.
    let text = screen_until(&sandbox.socket, "\u{276f}", 1, Duration::from_secs(10));
    assert!(
        text.contains('\u{276f}'),
        "the engine printed no prompt before the escaping case; screen: {text:?}"
    );
    // zsh resolves $'...' itself, so the tab and the newline never travel as control bytes.
    send_keys(&sandbox.socket, "cd -- $'boot\\tpath'");
    std::thread::sleep(Duration::from_millis(300));
    send_keys(&sandbox.socket, "echo ESCAPE-ALIVE");
    let text = screen_until(&sandbox.socket, "ESCAPE-ALIVE", 2, Duration::from_secs(10));
    assert!(
        text.matches("ESCAPE-ALIVE").count() >= 2,
        "the shell stopped answering after cd into a path with a tab; screen: {text:?}"
    );
    // The header line is the one that carries the engine directory path; the printed command
    // line spells the tab as `\t` and never contains "engine/boot". With the unescaped cwd the
    // announce record ended at the tab, so that header path stopped at "boot". The dir segment
    // picks its icon by the path (a subdirectory of HOME gets a different glyph than /tmp), so
    // the assertion uses the text, not an icon, and only the path suffix.
    let header = text
        .lines()
        .find(|line| line.contains("engine/boot"))
        .unwrap_or_default();
    assert!(
        header.contains("path"),
        "the header path was cut at the tab: {header:?}\nfull screen: {text:?}"
    );
    // A newline in the name must not break the prompt either.
    send_keys(&sandbox.socket, "cd -- $'line\\nbreak'");
    std::thread::sleep(Duration::from_millis(300));
    send_keys(&sandbox.socket, "echo NEWLINE-ALIVE");
    let text = screen_until(&sandbox.socket, "NEWLINE-ALIVE", 2, Duration::from_secs(10));
    assert!(
        text.matches("NEWLINE-ALIVE").count() >= 2,
        "the shell stopped answering after cd into a path with a newline; screen: {text:?}"
    );
    assert!(
        !text.contains("panicked"),
        "the engine panicked on the escaped cwd: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// A recursive launch must say what happened and fall back to a clean `$SHELL` instead of
/// starting a second engine.
#[test]
fn a_recursive_launch_reports_and_exits() {
    assert_on_path("tmux");
    let sandbox = Sandbox::new("recursive");
    let capture = sandbox.dir.path().join("capture");
    let cmd = format!(
        "env -u P11K_LANG -u P11K_LOCALEDIR -u LANGUAGE LC_ALL=C.UTF-8 P11K_ENGINE=1 SHELL=/bin/true '{}'",
        Sandbox::bin()
    );
    new_session(&sandbox.socket, &cmd);
    pipe_pane(&sandbox.socket, &capture);
    let text = capture_until(
        &capture,
        "recursive launch detected",
        Duration::from_secs(10),
    );
    assert!(
        text.contains("recursive launch detected"),
        "the recursive launch was not reported; captured: {text:?}"
    );
    assert!(
        text.contains("shell rc"),
        "the hint should name the shell rc; captured: {text:?}"
    );
    // The guard falls back to a clean $SHELL, which here is `/bin/true`: the session must end.
    // Exec'ing a hard-coded interactive shell would keep it alive instead.
    assert!(
        wait_for_session_end(&sandbox.socket, Duration::from_secs(5)),
        "the recursive launch did not fall back to the configured $SHELL; captured: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// A shell that cannot be started must fail the engine with a message and a non-zero status.
/// Continuing past the failure leaves an engine that paints a prompt with no shell behind it.
#[test]
fn a_failed_inner_shell_is_reported() {
    assert_on_path("tmux");
    let sandbox = Sandbox::new("failed-spawn");
    let capture = sandbox.dir.path().join("capture");
    // The shell wrapping the engine prints the exit status after the engine has exited.
    let cmd = format!(
        "env -u P11K_LANG -u P11K_LOCALEDIR -u LANGUAGE LC_ALL=C.UTF-8 PATH=/nonexistent XDG_STATE_HOME='{}' \
         '{}' --shell zsh; echo EXIT:$?",
        sandbox.state.display(),
        Sandbox::bin()
    );
    new_session(&sandbox.socket, &cmd);
    pipe_pane(&sandbox.socket, &capture);
    let text = capture_until(
        &capture,
        "the inner shell failed to start",
        Duration::from_secs(10),
    );
    assert!(
        text.contains("the inner shell failed to start"),
        "a failed shell spawn went unreported; captured: {text:?}"
    );
    // Without the fix the engine carried on: it painted the instant header and the prompt, then
    // exited with status 0 when the empty pty hit end of file. The status is the observable
    // difference.
    let text = capture_until(&capture, "EXIT:", Duration::from_secs(10));
    assert!(
        text.contains("EXIT:1"),
        "the engine did not fail with a non-zero status after a broken shell spawn; \
         captured: {text:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}

/// Poll the announce file until it has at least `min` records or the timeout expires.
fn wait_for_announce(path: &Path, min: usize, timeout: Duration) -> Vec<String> {
    let start = Instant::now();
    loop {
        let records: Vec<String> = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        if records.len() >= min || start.elapsed() >= timeout {
            return records;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The command duration must not depend on the locale's fraction separator. Bash and zsh print
/// `EPOCHREALTIME` with the separator from `LC_NUMERIC`: a comma in `it_IT`, another character in
/// other locales. The templates parse that string, so a separator they do not expect either
/// aborts the arithmetic (bash) or produces a wrong number (zsh).
#[test]
fn a_locale_separator_does_not_break_the_command_duration() {
    const SEPARATORS: [&str; 3] = [".", ",", "\u{066b}"];
    let mut failures = Vec::new();
    for shell in ["bash", "zsh"] {
        // Digits printed after the separator: bash uses six (microseconds), zsh ten.
        let fraction = if shell == "bash" {
            "000000"
        } else {
            "0000000000"
        };
        let mut sandbox = Sandbox::new(&format!("duration-{shell}"));
        sandbox.copy_engine_rc(shell);
        sandbox.start_shell_with_ack(shell);
        screen_until(&sandbox.socket, "ACK>", 1, Duration::from_secs(10));
        if shell == "zsh" {
            // The preexec hook sets the start on every command; remove it so the test can plant
            // a start value that the next command would otherwise overwrite.
            send_keys(&sandbox.socket, "add-zsh-hook -d preexec _p11k_preexec");
            screen_until(&sandbox.socket, "ACK>", 2, Duration::from_secs(5));
        }
        for separator in SEPARATORS {
            std::fs::write(&sandbox.announce, "").expect("clear the announce file");
            // The sandbox has no engine, so plant an ack for every prompt: otherwise each round
            // waits out the ack timeout and the screen fills with that warning.
            std::fs::write(&sandbox.ack, "1\n1\nACK> \n\n1\n").expect("write the ack");
            // The start lies five seconds in the past, so a working calculation reports about
            // 5000 ms. The separator is the only difference between the rounds.
            let command =
                format!("_p11k_cmd_start=\"$(( $(date +%s) - 5 )){separator}{fraction}\"; :");
            send_keys(&sandbox.socket, &command);
            let records = wait_for_announce(&sandbox.announce, 1, Duration::from_secs(5));
            let Some(record) = records.iter().rev().find(|line| line.starts_with("h\t")) else {
                failures.push(format!(
                    "{shell} with {separator:?}: no announce record; records: {records:?}; \
                     screen: {:?}",
                    screen(&sandbox.socket)
                ));
                continue;
            };
            let exec = record
                .split('\t')
                .nth(6)
                .and_then(|v| v.parse::<u64>().ok());
            match exec {
                Some(ms) if (4000..=6500).contains(&ms) => {}
                other => failures.push(format!(
                    "{shell} with {separator:?}: exec_ms {other:?} (expected about 5000); \
                     record: {record:?}; screen: {:?}",
                    screen(&sandbox.socket)
                )),
            }
            let text = screen(&sandbox.socket);
            if text.contains("value too great") || text.contains("bad math expression") {
                failures.push(format!(
                    "{shell} with {separator:?}: the shell reported an error: {text:?}"
                ));
            }
        }
        let _ = tmux(&sandbox.socket, &["kill-server"]);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Whether the host can switch to `it_IT.UTF-8`. The flake provides it in the devShell and in
/// the checks through `localesOf`; a host without it skips the test, because the
/// injected-separator test covers the parser on its own.
fn comma_locale_available() -> bool {
    let output = Command::new("bash")
        .args(["-c", "printf %s \"$EPOCHREALTIME\""])
        .env("LC_ALL", "it_IT.UTF-8")
        .env("LANG", "it_IT.UTF-8")
        .output();
    // A working comma locale puts a comma in EPOCHREALTIME. A missing locale keeps the previous
    // locale or reports the failure on stderr, and leaves no comma in the value.
    output.is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(','))
}

/// The runtime path from the bug report: bash takes the EPOCHREALTIME fraction separator from
/// `LC_NUMERIC`, so a comma locale has to produce the same duration as the dot locale. Zsh
/// prints a dot under the same locale in this version, so its case checks the duration under
/// the locale; the comma branch stays covered by the injected-separator test.
#[test]
fn a_real_comma_locale_reports_the_command_duration() {
    if !comma_locale_available() {
        // The devShell and the flake checks carry it_IT.UTF-8 (flake.nix, `localesOf`), so this
        // test runs there. A host without the locale skips it.
        eprintln!("skipping: it_IT.UTF-8 is not available on this host");
        return;
    }
    for shell in ["bash", "zsh"] {
        let mut sandbox = Sandbox::new(&format!("comma-locale-{shell}"));
        sandbox.copy_engine_rc(shell);
        std::fs::write(&sandbox.ack, "1\n1\nOK> \n\n1\n").expect("write the ack");
        new_session(
            &sandbox.socket,
            &sandbox.shell_cmd_env(shell, &[("LC_ALL", "it_IT.UTF-8"), ("LANG", "it_IT.UTF-8")]),
        );
        let text = screen_until(&sandbox.socket, "OK>", 1, Duration::from_secs(10));
        assert!(
            text.contains("OK>"),
            "{shell}: the prompt did not appear; screen: {text:?}"
        );
        std::fs::write(&sandbox.announce, "").expect("clear the announce file");
        std::fs::write(&sandbox.ack, "1\n1\nOK> \n\n1\n").expect("write the ack");
        // A real command: the shell measures it with its own timestamps under the locale.
        send_keys(&sandbox.socket, "sleep 2");
        let records = wait_for_announce(&sandbox.announce, 1, Duration::from_secs(10));
        let Some(record) = records.iter().rev().find(|line| line.starts_with("h\t")) else {
            panic!(
                "{shell}: no announce record; records: {records:?}; screen: {:?}",
                screen(&sandbox.socket)
            );
        };
        let exec = record
            .split('\t')
            .nth(6)
            .and_then(|v| v.parse::<u64>().ok());
        assert!(
            matches!(exec, Some(ms) if (1500..=2600).contains(&ms)),
            "{shell}: exec_ms {exec:?} (expected about 2000); record: {record:?}; screen: {:?}",
            screen(&sandbox.socket)
        );
        let text = screen(&sandbox.socket);
        assert!(
            !text.contains("value too great") && !text.contains("bad math expression"),
            "{shell}: the shell reported an arithmetic error: {text:?}"
        );
        let _ = tmux(&sandbox.socket, &["kill-server"]);
    }
}

/// The instant header has no exit status, execution time or jobs, so its right column can be
/// empty. The frame's suffix must still land on the right edge: the row is filled with gap
/// characters instead of ending right after the left content.
#[test]
fn the_instant_header_fills_the_row() {
    assert_on_path("tmux");
    assert_on_path("zsh");
    let sandbox = Sandbox::new("instant-width");
    let work = sandbox.dir.path().join("engine");
    std::fs::create_dir_all(&work).expect("engine work dir");
    let theme_dir = sandbox.config.join("p11k");
    std::fs::create_dir_all(&theme_dir).expect("theme dir");
    std::fs::write(
        theme_dir.join("p11k.kdl"),
        "layout {\n  left { line { dir } }\n  \
         right { line { status; command_execution_time; background_jobs } }\n}\n\
         frame {\n  first-prefix \"╭─\"\n  first-suffix \"─╮\"\n  \
         newline-prefix \"├─\"\n  newline-suffix \"─┤\"\n  last-prefix \"╰─\"\n}\n\
         separators { gap \"·\" }",
    )
    .expect("write the theme");
    let cmd = format!(
        "unset P11K_LANG P11K_LOCALEDIR LANGUAGE; export LC_ALL=C.UTF-8; sleep 0.3; cd '{}' && \
         XDG_STATE_HOME='{}' XDG_CONFIG_HOME='{}' P11K_USER_ZSHRC=/dev/null \
         P11K_USER_PROFILE=/dev/null exec '{}' --shell zsh",
        work.display(),
        sandbox.state.display(),
        sandbox.config.display(),
        Sandbox::bin()
    );
    new_session(&sandbox.socket, &cmd);
    let text = screen_until(&sandbox.socket, "❯", 1, Duration::from_secs(10));
    let header = text
        .lines()
        .find(|line| line.starts_with('╭'))
        .unwrap_or_else(|| panic!("no instant header; screen: {text:?}"));
    // With an empty right column the gap run is what places the suffix at the right edge. A row
    // that stops after the left content has no gap at all.
    let gap = header.matches('·').count();
    assert!(
        gap >= 20,
        "the instant header did not fill the row to its right edge (gap chars: {gap}): {header:?}"
    );
    assert!(
        header.trim_end().ends_with("─╮"),
        "the instant header lost its frame suffix: {header:?}"
    );
    let _ = tmux(&sandbox.socket, &["kill-server"]);
}
