//! `p11k --version` / `--help` 的 CLI 行为。
//!
//! 这两条路径不涉及终端状态，因此不需要 pty：直接启动进程读取 stdout 即可。

use std::process::Command;

fn p11k(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .args(args)
        // 断言写的是英文文案，而 help 是走 gettext 的：宿主 locale（尤其
        // LC_MESSAGES，它比 LANG 优先）一设成 zh_CN 就会输出中文。
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .output()
        .expect("run p11k");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn version_prints_the_crate_version() {
    let (stdout, stderr, code) = p11k(&["--version"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout.trim(), format!("p11k {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn short_version_flag_works_too() {
    let (stdout, _, code) = p11k(&["-V"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("p11k "), "got {stdout:?}");
}

#[test]
fn help_lists_the_flags_and_the_wizard() {
    let (stdout, stderr, code) = p11k(&["--help"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for needle in [
        "usage: p11k",
        "p11k configure",
        "--shell",
        "--config",
        "--preset",
        "--version",
        "--help",
        "configure",
    ] {
        assert!(
            stdout.contains(needle),
            "help should mention {needle}: {stdout}"
        );
    }
}
