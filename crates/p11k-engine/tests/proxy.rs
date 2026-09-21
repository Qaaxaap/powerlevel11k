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
    spawn_engine_shell("zsh", config_home)
}

/// `shell` is passed to `--shell`. The placeholder protocol differs per shell: zsh announces
/// the rendered prompt through zle-line-init, while bash (and fish/pwsh) leave the engine to
/// match the placeholder in the pass-through stream.
fn spawn_engine_shell(shell: &str, config_home: &str) -> Engine {
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
    cmd.arg(shell);
    cmd.env("P11K_USER_ZSHRC", "/dev/null");
    cmd.env("XDG_CONFIG_HOME", config_home);
    // Pin the terminal type: readline's clear-screen (Ctrl-L) comes from terminfo, and an
    // unset or unknown TERM in the test environment silently turns it into a plain repaint.
    cmd.env("TERM", "xterm-256color");
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

/// bash reprints the whole `PS1` whenever readline redraws the prompt over itself, and
/// showing completion candidates is one of those cases. `PS1` is the placeholder itself, so
/// the engine has to paint the prefix over the marker every time it shows up again, not only
/// on the first prompt after precmd. A marker that is left to be followed by the reprinted
/// input line is the artifact this test exists for.
#[test]
fn bash_prompt_reprint_keeps_the_placeholder_covered() {
    let home = std::env::temp_dir().join(format!("p11k-bash-reprint-{}", std::process::id()));
    let dir = home.join("probe");
    let _ = std::fs::remove_dir_all(&home);
    for name in ["probe-alpha", "probe-beta", "probe-gamma"] {
        std::fs::create_dir_all(dir.join(name)).unwrap();
    }

    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&*eng.master, &mut *eng.reader);
    assert_readline_available(&mut eng);

    eng.writer
        .write_all(format!("cd {}/probe-\t\t", dir.display()).as_bytes())
        .unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "probe-gamma",
        Duration::from_secs(5),
    );
    out.push_str(&read_until(
        &*eng.master,
        &mut *eng.reader,
        "❯",
        Duration::from_secs(2),
    ));
    let _ = std::fs::remove_dir_all(&home);

    let mut seen = 0;
    for (i, _) in out.match_indices("__") {
        seen += 1;
        // The engine paints right on top of the marker: the backfill starts with a cursor save
        // and a move up, so the reprinted input line may not slip in before it.
        let after: String = out[i + 2..].chars().take(16).collect();
        assert!(
            after.starts_with("\x1b[s"),
            "placeholder #{seen} was not covered, next bytes: {after:?}"
        );
    }
    assert!(
        seen >= 1,
        "the completion redraw should reprint the placeholder, got {seen}"
    );
}

/// The placeholder carries one newline per header row and the tty turns each of them into
/// `\r\n`, so the marker match has to tolerate the CRs: a two-row header must still be
/// backfilled, in every shell that matches the marker itself.
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
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"echo two-row-probe\n").unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "two-row-probe",
        Duration::from_secs(5),
    );
    out.push_str(&read_until(
        &*eng.master,
        &mut *eng.reader,
        "❯",
        Duration::from_secs(2),
    ));
    let _ = std::fs::remove_dir_all(&home);

    assert!(
        out.contains("two-row-probe"),
        "the command should have run, got {out:?}"
    );
    for (i, _) in out.match_indices("__") {
        let after: String = out[i + 2..].chars().take(16).collect();
        assert!(
            after.starts_with("\x1b[s"),
            "placeholder was not covered, next bytes: {after:?}"
        );
    }
}

/// The marker is as wide as the prefix, so a wide prefix means a long marker and a long
/// placeholder: the match, the backfill and the overwrite all have to follow that length (and
/// the header's row count) rather than any fixed size.
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
    wait_ready(&*eng.master, &mut *eng.reader);

    eng.writer.write_all(b"echo wide-probe\n").unwrap();
    eng.writer.flush().unwrap();
    let mut out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "wide-probe",
        Duration::from_secs(5),
    );
    out.push_str(&read_until(
        &*eng.master,
        &mut *eng.reader,
        "❯",
        Duration::from_secs(2),
    ));
    let _ = std::fs::remove_dir_all(&home);

    // Fourteen columns of prefix → a fourteen byte marker, in front of three header rows.
    let marker = "_".repeat(14);
    let mut seen = 0;
    for (i, _) in out.match_indices(&marker) {
        seen += 1;
        let after: String = out[i + marker.len()..].chars().take(16).collect();
        assert!(
            after.starts_with("\x1b[s"),
            "the long placeholder was not covered, next bytes: {after:?}"
        );
    }
    assert!(
        seen >= 1,
        "the wide placeholder should be printed and matched, got {seen}"
    );
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
    wait_ready(&*eng.master, &mut *eng.reader);

    for ch in ["_", "_", "_", "_", "_"] {
        eng.writer.write_all(ch.as_bytes()).unwrap();
        eng.writer.flush().unwrap();
        let out = read_until(&*eng.master, &mut *eng.reader, ch, Duration::from_secs(2));
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
    wait_ready(&*eng.master, &mut *eng.reader);

    for ch in ["i", "m", "m", "e", "d"] {
        eng.writer.write_all(ch.as_bytes()).unwrap();
        eng.writer.flush().unwrap();
        let out = read_until(&*eng.master, &mut *eng.reader, ch, Duration::from_secs(2));
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
    wait_ready(&*eng.master, &mut *eng.reader);
    assert_readline_available(&mut eng);

    eng.writer
        .write_all(format!("ls {}/c\t\t", dir.display()).as_bytes())
        .unwrap();
    eng.writer.flush().unwrap();
    // bash asks before listing this many candidates.
    read_until(
        &*eng.master,
        &mut *eng.reader,
        "possibilities",
        Duration::from_secs(5),
    );
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
    let mut out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "c899",
        Duration::from_secs(5),
    );
    out.push_str(&read_until(
        &*eng.master,
        &mut *eng.reader,
        "❯",
        Duration::from_secs(2),
    ));
    let _ = std::fs::remove_dir_all(&home);

    let mut seen = 0;
    for (i, _) in out.match_indices("__") {
        seen += 1;
        let after: String = out[i + 2..].chars().take(16).collect();
        assert!(
            after.starts_with("\x1b[s"),
            "placeholder #{seen} printed by the pager was not covered, next bytes: {after:?}"
        );
    }
    assert!(
        seen >= 1,
        "the paged prompt should reprint the placeholder, got {seen}"
    );
}

/// Ctrl-L makes readline clear the screen and reprint the prompt. The header was on screen
/// before the clear, so the engine has to rebuild it from the reprint: the screen must not
/// end up with a bare marker and no header.
#[test]
fn bash_ctrl_l_rebuilds_the_prompt() {
    let mut eng = spawn_engine_shell("bash", "/nonexistent-p11k-config");
    wait_ready(&*eng.master, &mut *eng.reader);
    assert_readline_available(&mut eng);

    // Run an empty command first and let the next prompt settle: bash paints the first prompt
    // while the rcfile is still taking effect, and a Ctrl-L sent in between lands in the line
    // buffer instead of running clear-screen.
    eng.writer.write_all(b"\n").unwrap();
    eng.writer.flush().unwrap();
    read_until(&*eng.master, &mut *eng.reader, "❯", Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(300));

    eng.writer.write_all(b"\x0c").unwrap();
    eng.writer.flush().unwrap();

    let mut cleared = read_until(
        &*eng.master,
        &mut *eng.reader,
        "\x1b[2J",
        Duration::from_secs(5),
    );
    assert!(
        cleared.contains("\x1b[2J"),
        "readline clears the screen on Ctrl-L, got {cleared:?}"
    );
    // The repaint may already be in the same read as the clear.
    if !cleared.contains('❯') {
        cleared.push_str(&read_until(
            &*eng.master,
            &mut *eng.reader,
            "❯",
            Duration::from_secs(2),
        ));
    }
    assert!(
        cleared.contains('❯'),
        "the prompt should be rebuilt after Ctrl-L, got {cleared:?}"
    );
}

/// The tests below drive readline itself — clear-screen, the completion pager. nixpkgs' default
/// `bash` is built without readline (`bind` does not even exist), so on that shell they would
/// fail for a reason that has nothing to do with the engine. Say which it is.
fn assert_readline_available(eng: &mut Engine) {
    eng.writer
        .write_all(b"printf 'RL%sRL\\n' \"$(bind -P | grep -c '^clear-screen')\"\n")
        .unwrap();
    eng.writer.flush().unwrap();
    let out = read_until(
        &*eng.master,
        &mut *eng.reader,
        "RL1RL",
        Duration::from_secs(2),
    );
    assert!(
        out.contains("RL1RL"),
        "the inner bash has no readline (nixpkgs' non-interactive bash build?): {out:?}"
    );
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
