//! CLI behavior of `p11k --version` / `--help`.
//!
//! Neither path touches terminal state, so no pty is needed: launching the process and
//! reading stdout suffices.

use std::process::Command;

fn p11k(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .args(args)
        // The assertions check English text, while help goes through gettext: a host locale
        // (especially LC_MESSAGES, which outranks LANG) set to zh_CN would print Chinese.
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
