//! CLI behavior of `p11k --version` / `--help` / `reload`.
//!
//! None of these paths touches terminal state, so no pty is needed: launching the process
//! and reading stdout suffices.

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
        "p11k reload",
        "--shell",
        "--config",
        "--preset",
        "--version",
        "--help",
        "configure",
        "reload",
    ] {
        assert!(
            stdout.contains(needle),
            "help should mention {needle}: {stdout}"
        );
    }
}

#[test]
fn reload_refuses_to_run_outside_a_session() {
    // `p11k reload` reaches the engine through the environment the engine exports into the
    // inner shell; without it there is nothing to reload and the command must say so instead
    // of signalling an unrelated process.
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .arg("reload")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env_remove("P11K_ENGINE_PID")
        .env_remove("P11K_CONFIG")
        .output()
        .expect("run p11k reload");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not inside a p11k session"),
        "got {stderr:?}"
    );
}

#[test]
fn reload_reports_a_session_without_a_theme_file() {
    // A session started with --preset or the built-in theme has no file to re-read.
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .arg("reload")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env("P11K_ENGINE_PID", "1")
        .env_remove("P11K_CONFIG")
        .output()
        .expect("run p11k reload");
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no theme file"), "got {stderr:?}");
}

#[test]
fn reload_reports_a_broken_theme_file() {
    // The file is parsed by the command first, so a syntax error is reported in the ordinary
    // way rather than leaving the prompt silently unchanged.
    let path = std::env::temp_dir().join(format!("p11k-reload-{}.kdl", std::process::id()));
    std::fs::write(&path, "segments { dir fg= }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .arg("reload")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env("P11K_ENGINE_PID", "1")
        .env("P11K_CONFIG", &path)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_file(&path);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("KDL"), "got {stderr:?}");
}
