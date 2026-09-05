//! M0 集成测试：把引擎二进制放进一个 pty，从另一侧断言
//! 透传行为与 prompt 窗口的主题输出。
//!
//! 真实终端（测试侧 pty）看到的内容 = 引擎透传的 shell 输出 + 引擎画的
//! header，与用户在 kitty 里看到的一致。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

const COLS: u16 = 100;

fn spawn_engine() -> (
    Box<dyn MasterPty + Send>,
    Box<dyn Child + Send + Sync>,
    Box<dyn Read + Send>,
    Box<dyn Write + Send>,
) {
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
    (pair.master, child, reader, writer)
}

/// 读 pty 输出直到 `needle` 出现（或超时），返回累计内容。
/// 用 poll 限时，避免阻塞读把超时变成死等。
fn read_until(
    master: &Box<dyn MasterPty + Send>,
    reader: &mut Box<dyn Read + Send>,
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
        if rc > 0 && fds[0].revents & libc::POLLIN != 0 {
            if let Ok(n) = reader.read(&mut buf) {
                acc.push_str(&String::from_utf8_lossy(&buf[..n]));
            }
        }
    }
    acc
}

/// 等真正的第一个 prompt 就绪：instant header 没有退出码状态（无 ✓），只有
/// 内部 shell 加载完、第一次 precmd 后清屏重画的真 header 才带 ✓。
fn wait_ready(master: &Box<dyn MasterPty + Send>, reader: &mut Box<dyn Read + Send>) -> String {
    read_until(master, reader, "✓", Duration::from_secs(10))
}

#[test]
fn initial_prompt_shows_header_and_input_line() {
    let (master, _child, mut reader, _writer) = spawn_engine();
    let out = wait_ready(&master, &mut reader);
    assert!(
        out.contains('/') || out.contains('~'),
        "header 应含目录(~ 缩写或路径)，实际输出：{out:?}"
    );
    assert!(out.contains('✓'), "真正 header 应含 ✓ 退出码状态");
    // 等输入行前缀 ❯（占位 aa 之后、zle-line-init 宣告 p 后画）。
    let out2 = read_until(&master, &mut reader, "❯", Duration::from_secs(5));
    assert!(out2.contains('❯'), "输入行应含 ❯，实际输出：{out2:?}");
    // 时间 HH:MM 右对齐存在（用 \e[..G 定位过）。
    assert!(out.contains('\x1b'), "header 应含 ANSI 颜色/定位序列");
}

/// 占位协议：占位符 `aa` 原样透传，引擎随后 `\r` + 前缀顶掉（输入行延后
/// 绘制）。前缀的可见宽度与占位符恒等（2 列），zle 重绘列偏移由此对齐。
#[test]
fn placeholder_overwritten_by_prefix() {
    let (master, _child, mut reader, _writer) = spawn_engine();
    wait_ready(&master, &mut reader);
    // 真正 prompt：占位符先透传，随后 \r + 输入前缀(❯)顶掉。
    let out = read_until(&master, &mut reader, "❯", Duration::from_secs(5));
    assert!(out.contains("__"), "占位符应原样透传，实际输出：{out:?}");
    assert!(
        out.contains('\r') && out.contains('❯'),
        "透传后应回行首画前缀顶掉占位符，实际输出：{out:?}"
    );
    // 占位符出现在前缀之前：先透传后顶掉。
    let ph = out.find("__").expect("占位符存在");
    let px = out.find('❯').expect("前缀存在");
    assert!(ph < px, "占位符应先透传再被顶掉，实际输出：{out:?}");
}

#[test]
fn command_output_passthrough_and_next_prompt() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    // 等真正的 prompt 就绪（instant 不算，内部 shell 还没起）。
    wait_ready(&master, &mut reader);

    writer.write_all(b"echo hello-from-shell\n").unwrap();
    writer.flush().unwrap();
    let out = read_until(
        &master,
        &mut reader,
        "hello-from-shell",
        Duration::from_secs(5),
    );
    assert!(out.contains("hello-from-shell"), "命令输出应透传");

    // 下一个 prompt 也该出现（命令执行完 → precmd → header 重画带 ✓）。
    let out2 = read_until(&master, &mut reader, "✓", Duration::from_secs(5));
    assert!(out2.contains('✓'), "命令后应出新 prompt，实际：{out2:?}");
}

#[test]
fn exit_code_shows_in_status() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    wait_ready(&master, &mut reader);

    // 失败命令 → header 右段显示红色 ✘ n。
    writer.write_all(b"false\n").unwrap();
    writer.flush().unwrap();
    let out = read_until(&master, &mut reader, "✘", Duration::from_secs(5));
    assert!(out.contains('✘'), "退出码状态应显示 ✘，实际：{out:?}");
}

#[test]
fn prompt_char_turns_error_color_on_failure() {
    // 默认 lean:prompt_char 带 state ERROR fg=196。失败命令后输入行前缀
    // 的 ❯ 应变红(38;5;196),正常态是 76。
    let (master, _child, mut reader, mut writer) = spawn_engine();
    wait_ready(&master, &mut reader);

    writer.write_all(b"false\n").unwrap();
    writer.flush().unwrap();
    let out = read_until(
        &master,
        &mut reader,
        "\x1b[38;5;196m",
        Duration::from_secs(5),
    );
    assert!(
        out.contains("\x1b[38;5;196m❯"),
        "失败后 prompt_char 应进 ERROR state 变红(196)，实际：{out:?}"
    );
}

#[test]
fn ctrl_c_interrupts_running_command() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    wait_ready(&master, &mut reader);

    // sleep 前台运行，Ctrl-C 打断，然后出新 prompt。
    writer.write_all(b"sleep 5\n").unwrap();
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(b"\x03").unwrap();
    writer.flush().unwrap();
    let out = read_until(&master, &mut reader, "❯", Duration::from_secs(5));
    assert!(out.contains('❯'), "Ctrl-C 后应出新 prompt，实际：{out:?}");
}
