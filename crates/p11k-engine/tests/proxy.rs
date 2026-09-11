//! M0 集成测试：把引擎二进制放进一个 pty，从另一侧断言
//! 透传行为与 prompt 窗口的主题输出。
//!
//! 真实终端（测试侧 pty）看到的内容 = 引擎透传的 shell 输出 + 引擎画的
//! header，与用户在 kitty 里看到的一致。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const COLS: u16 = 100;

/// 一个跑起来的引擎：pty 主端、子进程、读端、写端。
struct Engine {
    master: Box<dyn MasterPty + Send>,
    /// 进程句柄：由 Drop 收尾（测试里不需要 kill，pty 关闭时 shell 自会退出）。
    #[allow(dead_code)]
    child: Box<dyn Child + Send + Sync>,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}

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
    // 隔离：不依赖测试机上的真实 ~/.zshrc（有 oh-my-zsh/p10k，慢且干扰断言）。
    // 指向空文件，让内部 shell 只跑引擎协议层。
    cmd.env("P11K_USER_ZSHRC", "/dev/null");
    // 隔离：cwd 用非 git 目录，避免 git 状态同步扫描拖慢/抖动 prompt 时序。
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

/// 读 pty 输出直到 `needle` 出现（或超时），返回累计内容。
/// 用 poll 限时，避免阻塞读把超时变成死等。
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

/// 等真正的第一个 prompt 就绪：instant header 没有退出码状态（无 ✔），只有
/// 内部 shell 加载完、第一次 precmd 后清屏重画的真 header 才带 ✔。
fn wait_ready(master: &dyn MasterPty, reader: &mut dyn Read) -> String {
    read_until(master, reader, "\u{f00c}", Duration::from_secs(10))
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
    // instant header 已含输入行前缀 ❯;真 prompt 的前缀覆盖由
    // placeholder_overwritten_by_prefix 单独验证(❯ 在占位符 __ 之后)。
    assert!(
        out.contains('❯'),
        "input line should contain ❯, got {out:?}"
    );
    assert!(
        out.contains('\x1b'),
        "header should contain ANSI color/positioning sequences"
    );
}

/// 占位协议：占位符 `aa` 原样透传，引擎随后 `\r` + 前缀顶掉（输入行延后
/// 绘制）。前缀的可见宽度与占位符恒等（2 列），zle 重绘列偏移由此对齐。
#[test]
fn placeholder_overwritten_by_prefix() {
    let mut eng = spawn_engine();
    let mut full = wait_ready(&*eng.master, &mut *eng.reader);
    // 占位协议：shell 渲染的多行占位(换行 + 占位符 __)先透传,引擎随后 \r + 前缀
    // (❯)回行首顶掉。instant 的 ❯ 在 __ 之前,不算;真 prompt 的 ❯ 一定在 __ 之后。
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
    // 等真正的 prompt 就绪（instant 不算，内部 shell 还没起）。
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

    // 下一个 prompt 也该出现（命令执行完 → precmd → header 重画带 ✔）。
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

    // 失败命令 → header 右段显示红色 ✘ n。
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
    // 默认 lean:prompt_char 带 state ERROR fg=196。失败命令后输入行前缀
    // 的 ❯ 应变红(38;5;196),正常态是 76。
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

    // sleep 前台运行，Ctrl-C 打断，然后出新 prompt。
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
