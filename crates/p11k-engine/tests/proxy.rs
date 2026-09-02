//! M0 集成测试：把引擎二进制放进一个 pty，从另一侧断言
//! 透传行为与 prompt 窗口的主题输出。
//!
//! 真实终端（测试侧 pty）看到的内容 = 引擎透传的 shell 输出 + 引擎画的
//! header，与用户在 kitty 里看到的一致。

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

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

#[test]
fn initial_prompt_shows_header_and_input_line() {
    let (master, _child, mut reader, _writer) = spawn_engine();
    let out = read_until(&master, &mut reader, "@", Duration::from_secs(10));
    assert!(
        out.contains('@'),
        "header 应含 user@host，实际输出：{out:?}"
    );
    assert!(out.contains("❯"), "输入行应含 ❯，实际输出：{out:?}");
    // 时间 HH:MM 右对齐存在（用 \e[..G 定位过）。
    assert!(out.contains('\x1b'), "header 应含 ANSI 颜色/定位序列");
}

#[test]
fn command_output_passthrough_and_next_prompt() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    // 等初始 prompt 就绪。
    read_until(&master, &mut reader, "❯", Duration::from_secs(10));

    writer.write_all(b"echo hello-from-shell\n").unwrap();
    writer.flush().unwrap();
    let out = read_until(&master, &mut reader, "hello-from-shell", Duration::from_secs(5));
    assert!(out.contains("hello-from-shell"), "命令输出应透传");

    // 下一个 prompt 也该出现（precmd 宣告 + header 重画）。
    let out2 = read_until(&master, &mut reader, "❯", Duration::from_secs(5));
    assert!(out2.contains("hello-from-shell"));
}

#[test]
fn exit_code_shows_in_status() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    read_until(&master, &mut reader, "❯", Duration::from_secs(10));

    // 失败命令 → header 右段显示红色 ✘ n。
    writer.write_all(b"false\n").unwrap();
    writer.flush().unwrap();
    let out = read_until(&master, &mut reader, "✘", Duration::from_secs(5));
    assert!(out.contains('✘'), "退出码状态应显示 ✘，实际：{out:?}");
}

#[test]
fn ctrl_c_interrupts_running_command() {
    let (master, _child, mut reader, mut writer) = spawn_engine();
    read_until(&master, &mut reader, "❯", Duration::from_secs(10));

    // sleep 前台运行，Ctrl-C 打断，然后出新 prompt。
    writer.write_all(b"sleep 5\n").unwrap();
    writer.flush().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    writer.write_all(b"\x03").unwrap();
    writer.flush().unwrap();
    let out = read_until(&master, &mut reader, "❯", Duration::from_secs(5));
    assert!(out.contains('❯'), "Ctrl-C 后应出新 prompt，实际：{out:?}");
}
