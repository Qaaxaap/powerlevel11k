//! 端到端测试：把引擎二进制放进 pty，从另一侧断言透传与 prompt 窗口。
//!
//! 引擎不解析 pty 输出，它画出的字节和用户在终端里看到的是同一份 —— 所以断言
//! 直接落在这些字节上：占位协议、header 回填、退出码着色。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const COLS: u16 = 100;

struct Engine {
    master: Box<dyn MasterPty + Send>,
    /// 留着句柄由 Drop 收尾：pty 主端一关，内部 shell 自己退出，不需要 kill。
    #[allow(dead_code)]
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

/// 按隔离条件起引擎：显式 `--shell zsh`（CI 的 `$SHELL` 是 bash，而下面的断言是
/// 照 zsh 的输出写的）、用户 rc 指向 `/dev/null`（真实 `~/.zshrc` 里的
/// oh-my-zsh/p10k 又慢又会干扰断言）、cwd 用 `/tmp`（非 git 目录，避免 git 状态
/// 扫描抖动 prompt 时序）。
fn spawn_engine() -> Engine {
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

/// 读到 `needle` 出现或超时，返回累计输出。用 poll 限时：直接阻塞读会让超时
/// 变成永久等待。
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

/// 等真正的第一个 prompt 就绪。判据是 `✔`：instant header 是引擎按启动时已知的
/// 状态画的、不含退出码，只有内部 shell 加载完、第一次 precmd 之后重画的 header
/// 才带它。
fn wait_ready(master: &dyn MasterPty, reader: &mut dyn Read) -> String {
    let out = read_until(master, reader, "\u{f00c}", Duration::from_secs(10));
    if !out.contains('\u{f00c}') {
        // 超时时只看到“输出被截断”，无从判断是 spawn 失败、卡在 ack 还是 shell
        // 根本没起来 —— 把引擎日志尾部带上。
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
    // instant header 也含 ❯；前缀覆盖占位符这件事由
    // placeholder_overwritten_by_prefix 单独验证。
    assert!(
        out.contains('❯'),
        "input line should contain ❯, got {out:?}"
    );
    assert!(
        out.contains('\x1b'),
        "header should contain ANSI color/positioning sequences"
    );
}

/// 占位协议：shell 渲染的 `__`（`_` 按前缀可见宽度重复而成）原样透传，引擎随后
/// 用 `\r` + 前缀覆盖它。前缀宽度与占位符恒等（2 列），zle 重绘的列偏移由此对齐。
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

    // 命令执行完 → precmd → 重画 header，此时带 ✔。
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
    // 默认 lean 的 prompt_char 带 state ERROR fg=196、正常态 fg=76：失败命令后
    // 前缀 ❯ 应该变红。
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
