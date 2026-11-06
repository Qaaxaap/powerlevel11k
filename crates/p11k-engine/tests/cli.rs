//! CLI behavior of `p11k --version` / `--help` / `reload`.
//!
//! None of these paths touches terminal state, so no pty is needed: launching the process
//! and reading stdout suffices.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

static SESSION_SEQ: AtomicU32 = AtomicU32::new(0);

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

/// A session directory holding one token, plus an isolated config home with no default theme.
fn temp_session(token: &str) -> (PathBuf, PathBuf) {
    let seq = SESSION_SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("p11k-reload-{}-{seq}", std::process::id()));
    let session = base.join("session");
    let config_home = base.join("config");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&session).unwrap();
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::write(session.join("session"), token).unwrap();
    (session, config_home)
}

fn reload_command(session: &std::path::Path, token: &str, pid: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_p11k"));
    cmd.arg("reload")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env("P11K_ENGINE_PID", pid)
        .env("P11K_ENGINE_SESSION", session)
        .env("P11K_ENGINE_TOKEN", token);
    cmd
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
fn help_lists_the_flags_and_the_commands() {
    let (stdout, stderr, code) = p11k(&["--help"]);
    assert_eq!(code, 0, "stderr: {stderr}");
    for needle in [
        "Usage: p11k",
        "Commands:",
        "Options:",
        "configure",
        "reload",
        "--shell",
        "--config",
        "--preset",
        "--log-level",
        "--version",
        "--help",
        "Environment:",
        "P11K_ENGINE",
        "P11K_USER_ZSHRC",
        "P11K_USER_PROFILE",
    ] {
        assert!(
            stdout.contains(needle),
            "help should mention {needle}: {stdout}"
        );
    }
    // Internal variables and the removed zsh-only debug override stay out of the user help.
    for needle in [
        "P11K_ZSHRC",
        "P11K_LOCALEDIR",
        "P11K_ENGINE_PID",
        "P11K_ENGINE_TOKEN",
        "P11K_ENGINE_SESSION",
        "P11K_CONFIG",
    ] {
        assert!(
            !stdout.contains(needle),
            "help should not mention {needle}: {stdout}"
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
        .env_remove("P11K_ENGINE_SESSION")
        .env_remove("P11K_ENGINE_TOKEN")
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
fn reload_rejects_a_non_positive_pid() {
    let (session, config_home) = temp_session("token");
    let out = reload_command(&session, "token", "0")
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("invalid P11K_ENGINE_PID"), "got {stderr:?}");
}

#[test]
fn reload_rejects_a_missing_session_token() {
    let (session, config_home) = temp_session("token");
    let out = Command::new(env!("CARGO_BIN_EXE_p11k"))
        .arg("reload")
        .env("LC_ALL", "C")
        .env_remove("LANGUAGE")
        .env("P11K_ENGINE_PID", "1")
        .env("P11K_ENGINE_SESSION", &session)
        .env_remove("P11K_ENGINE_TOKEN")
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("session data is missing"), "got {stderr:?}");
}

#[test]
fn reload_rejects_a_token_that_does_not_match() {
    let (session, config_home) = temp_session("the-real-token");
    let out = reload_command(&session, "a-stale-token", "1")
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("token does not match"), "got {stderr:?}");
}

#[test]
fn reload_reports_a_session_without_a_theme_file() {
    // A session started with --preset or the built-in theme has no file to re-read.
    let (session, config_home) = temp_session("token");
    let out = reload_command(&session, "token", "1")
        .env_remove("P11K_CONFIG")
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no theme file"), "got {stderr:?}");
}

#[test]
fn reload_reports_a_broken_theme_file() {
    // The file is parsed by the command first, so a syntax error is reported in the ordinary
    // way rather than leaving the prompt silently unchanged.
    let (session, config_home) = temp_session("token");
    let path = session.parent().unwrap().join("broken.kdl");
    std::fs::write(&path, "segments { dir fg= }\n").unwrap();
    let out = reload_command(&session, "token", "1")
        .env("P11K_CONFIG", &path)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("KDL"), "got {stderr:?}");
}

#[test]
fn reload_with_config_reports_a_broken_file() {
    let (session, config_home) = temp_session("token");
    let path = session.parent().unwrap().join("bad-config.kdl");
    std::fs::write(&path, "segments { dir fg= }\n").unwrap();
    let out = reload_command(&session, "token", "1")
        .arg("--config")
        .arg(&path)
        .env("XDG_CONFIG_HOME", &config_home)
        .output()
        .expect("run p11k reload");
    let _ = std::fs::remove_dir_all(session.parent().unwrap());
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("KDL"), "got {stderr:?}");
}
