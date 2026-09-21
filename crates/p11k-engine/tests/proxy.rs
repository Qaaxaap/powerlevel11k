//! End-to-end tests: put the engine binary behind a pty and assert passthrough and
//! the prompt window from the other side.
//!
//! The engine does not parse pty output; the bytes it paints are the same ones the user
//! sees in the terminal — so the assertions land directly on those bytes: the
//! placeholder protocol, header repaint, exit-code coloring.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const COLS: u16 = 100;

struct Engine {
    master: Box<dyn MasterPty + Send>,
    /// The handle is kept and cleaned up by Drop: closing the pty master makes the inner shell exit on its own, no kill needed.
    #[allow(dead_code)]
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

/// Start the engine under isolation: explicit `--shell zsh` (CI's `$SHELL` is bash,
/// while the assertions below are written against zsh's output), user rc pointed at
/// `/dev/null` (a real `~/.zshrc` with oh-my-zsh/p10k is both slow and disruptive to
/// the assertions), cwd `/tmp` (not a git directory, so git status scanning cannot
/// jitter the prompt timing).
fn spawn_engine() -> Engine {
    spawn_engine_with_config_home("/nonexistent-p11k-config")
}

/// `config_home` becomes `$XDG_CONFIG_HOME`, where the engine looks for the theme when
/// `--config` is not given. The default is a directory that does not exist, or the developer's
/// own theme would decide what these assertions see.
fn spawn_engine_with_config_home(config_home: &str) -> Engine {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_p11k"));
    cmd.arg("--shell");
    cmd.arg("zsh");
    cmd.env("P11K_USER_ZSHRC", "/dev/null");
    cmd.env("XDG_CONFIG_HOME", config_home);
    cmd.cwd("/tmp");
    let child = pair.slave.spawn_command(cmd).unwrap();
    drop(pair.slave);
    let reader = pair.master.try_clone_reader().unwrap();
    let writer = pair.master.take_writer().unwrap();
    Engine {
        master: pair.master,
        child,
        reader,
        writer,
    }
}

/// Read until `needle` appears or the timeout expires, returning the accumulated output.
/// Poll with a deadline: a plain blocking read would turn the timeout into an indefinite wait.
fn read_until(
    master: &dyn MasterPty,
    reader: &mut dyn Read,
    needle: &str,
    timeout: Duration,
) -> String {
    let fd = master.as_raw_fd().expect("pty fd");
    let mut acc = String::new();
    let mut buf = [0u8; 4096];
    let start = Instant::now();
    while start.elapsed() < timeout {
        if acc.contains(needle) {
            return acc;
        }
        let mut fds = [libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), 1, 200) };
        if rc > 0
            && fds[0].revents & libc::POLLIN != 0
            && let Ok(n) = reader.read(&mut buf)
        {
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    acc
}

/// Wait for the real first prompt to be ready. The criterion is `✔`: the instant header
/// is painted by the engine from the state known at startup and carries no exit code;
/// only the header repainted after the inner shell finishes loading and runs its first
/// precmd carries it.
fn wait_ready(master: &dyn MasterPty, reader: &mut dyn Read) -> String {
    let out = read_until(master, reader, "\u{f00c}", Duration::from_secs(10));
    if !out.contains('\u{f00c}') {
        // On timeout all we see is "output was truncated", with no way to tell whether spawn
        // failed, the engine is stuck on an ack, or the shell never started — include the tail
        // of the engine log.
        match std::fs::read_to_string("/tmp/p11k-engine.log") {
            Ok(log) => {
                let tail: Vec<&str> = log.lines().rev().take(15).collect();
                eprintln!("engine log tail (newest first): {tail:#?}");
            }
            Err(e) => eprintln!("no engine log at /tmp/p11k-engine.log: {e}"),
        }
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

    let mut eng = spawn_engine_with_config_home(home.to_str().unwrap());
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "FROM-CONFIG-HOME",
        Duration::from_secs(10),
    );
    let _ = std::fs::remove_dir_all(&home);
    assert!(
        out.contains("FROM-CONFIG-HOME"),
        "the theme in the config home should be used, got {out:?}"
    );
}

#[test]
fn initial_prompt_shows_header_and_input_line() {
    let mut eng = spawn_engine();
    let out = wait_ready(&*eng.master, &mut *eng.reader);
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

/// Placeholder protocol: the `__` rendered by the shell (`_` repeated to the prefix's
/// visible width) passes through verbatim, and the engine then overwrites it with `\r` +
/// the prefix. The prefix width equals the placeholder's (2 columns), so zle's repaint
/// column offset lines up.
#[test]
fn placeholder_overwritten_by_prefix() {
    let mut eng = spawn_engine();
    let mut full = wait_ready(&*eng.master, &mut *eng.reader);
    assert!(
        full.contains("__"),
        "placeholder should pass through verbatim, got {full:?}"
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let ph = full.find("__").expect("placeholder present");
        if full[ph..].contains('❯') {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "prefix ❯ should appear after the __ placeholder, got {full:?}"
        );
        full.push_str(&read_until(
            &*eng.master,
            &mut *eng.reader,
            "❯",
            Duration::from_secs(1),
        ));
    }
}

#[test]
fn command_output_passthrough_and_next_prompt() {
    let mut eng = spawn_engine();
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"echo hello-from-shell\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "hello-from-shell",
        Duration::from_secs(5),
    );
    assert!(
        out.contains("hello-from-shell"),
        "command output should pass through"
    );

    // The command finished → precmd → the header is repainted, this time carrying ✔.
    let out2 = read_until(
        &*eng.master,
        &mut *eng.reader,
        "\u{f00c}",
        Duration::from_secs(5),
    );
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
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer
        .write_all(b"printf 'P11K-ROW-PROBE\\n'\n")
        .unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "\x1b]133;A\x07",
        Duration::from_secs(5),
    );
    let _ = std::fs::remove_dir_all(&home);

    // The command output ends with its own CRLF; everything between it and the prompt-start
    // marker is the blank row the engine writes before the header.
    let output = "P11K-ROW-PROBE\r\n";
    let start = out.find(output).expect("command output") + output.len();
    let rest = &out[start..];
    let blank = &rest[..rest.find("\x1b]133;A\x07").expect("prompt start marker")];
    assert_eq!(
        blank.matches("\r\n").count(),
        1,
        "prompt-add-newline #true should open one blank row, got {blank:?}"
    );
}

#[test]
fn exit_code_shows_in_status() {
    let mut eng = spawn_engine();
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"false\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "\u{f00d}",
        Duration::from_secs(5),
    );
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
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"false\n").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "\x1b[38;5;196m",
        Duration::from_secs(5),
    );
    assert!(
        out.contains("\x1b[38;5;196m❯"),
        "after a failure prompt_char should turn red in the ERROR state (196), got {out:?}"
    );
}

#[test]
fn ctrl_c_interrupts_running_command() {
    let mut eng = spawn_engine();
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"sleep 5\n").unwrap();
    eng.writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    eng.writer.write_all(b"\x03").unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(&*eng.master, &mut *eng.reader, "❯", Duration::from_secs(5));
    assert!(
        out.contains('❯'),
        "a new prompt should appear after Ctrl-C, got {out:?}"
    );
}
