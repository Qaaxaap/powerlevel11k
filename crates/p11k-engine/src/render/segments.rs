//! Segment text: the values each segment renders, plus the helpers that read them from the
//! repository, the environment and the system.
//!
//! The renderer dispatches to these functions from [`super::render_segment`]. The segment text
//! is plain (no colour); the style is applied by the dispatcher.

use std::fmt::Write as _;

use crate::config::{Config, Segment, Style};
use crate::model::HeaderInfo;

use super::env::*;
use super::icons::dir_class_match;
use super::*;

/// p10k `DIR_SHOW_WRITABLE` values: `#true`=1, `"v2"`=2, `"v3"`=3 (as in p10k only these are
/// valid; p11k additionally accepts an integer). Returns 0 for no check.
pub(super) fn dir_show_writable(seg: &crate::config::Segment) -> i64 {
    match seg.prop("show-writable") {
        Some(crate::config::Prop::Bool(true)) => 1,
        Some(crate::config::Prop::Int(n)) => (*n).clamp(0, 3),
        Some(crate::config::Prop::Str(s)) => match s.as_str() {
            "true" => 1,
            "v2" => 2,
            "v3" => 3,
            _ => 0,
        },
        _ => 0,
    }
}

/// Returns the matching state name when the directory is not writable (or missing under v3),
/// otherwise None. p10k tests `[[ -w $PWD ]]`; access(2) matches that here.
pub(super) fn dir_writable_state(seg: &crate::config::Segment, cwd: &str) -> Option<&'static str> {
    let mode = dir_show_writable(seg);
    if mode <= 0 {
        return None;
    }
    let bytes = std::ffi::CString::new(cwd).ok()?;
    let writable = unsafe { libc::access(bytes.as_ptr(), libc::W_OK) } == 0;
    if writable {
        return None;
    }
    if mode > 2 && !std::path::Path::new(cwd).exists() {
        Some("NON_EXISTENT")
    } else {
        Some("NOT_WRITABLE")
    }
}

/// AWS segment: current profile (env `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`).
pub(super) fn aws_text() -> String {
    env_var("AWS_PROFILE")
        .or_else(|| env_var("AWS_DEFAULT_PROFILE"))
        .unwrap_or_default()
}

/// gcloud segment: current configuration name (contents of the active_config file).
pub(super) fn gcloud_text() -> String {
    let dir = env_var("CLOUDSDK_CONFIG").unwrap_or_else(|| format!("{}/.config/gcloud", home()));
    std::fs::read_to_string(format!("{dir}/active_config"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// azure segment: default subscription name (the name with `isDefault: true` in
/// azureProfile.json).
pub(super) fn azure_text() -> String {
    let dir = env_var("AZURE_CONFIG_DIR").unwrap_or_else(|| format!("{}/.azure", home()));
    let Some(s) = std::fs::read_to_string(format!("{dir}/azureProfile.json")).ok() else {
        return String::new();
    };
    let mut rest = s.as_str();
    while let Some(pos) = rest.find("\"isDefault\":") {
        // Whitespace is allowed after `isDefault`, then check for true.
        if rest[pos + "\"isDefault\":".len()..]
            .trim_start()
            .starts_with("true")
        {
            let before = &rest[..pos];
            if let Some(n) = before.rfind("\"name\"") {
                let after = &before[n + "\"name\"".len()..];
                if let Some(v) = after.split('"').nth(1) {
                    return v.to_string();
                }
            }
        }
        rest = &rest[pos + 1..];
    }
    String::new()
}

/// kubecontext segment: current context (from `$KUBECONFIG` or `~/.kube/config`).
pub(super) fn kubecontext_text(_cwd: &str) -> String {
    let path = env_var("KUBECONFIG").unwrap_or_else(|| format!("{}/.kube/config", home()));
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.trim()
                    .strip_prefix("current-context:")
                    .map(|v| v.trim().to_string())
            })
        })
        .unwrap_or_default()
}

/// terraform segment: current workspace (from `$TF_DATA_DIR`/`.terraform/environment`).
pub(super) fn terraform_text(cwd: &str) -> String {
    let dir = env_var("TF_DATA_DIR").unwrap_or_else(|| ".terraform".into());
    std::fs::read_to_string(std::path::Path::new(cwd).join(dir).join("environment"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

pub(super) fn toolbox_text() -> String {
    env_var("P9K_TOOLBOX_NAME")
        .or_else(|| {
            std::fs::read_to_string("/run/.containerenv")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find_map(|l| l.strip_prefix("name=").map(|v| v.trim().to_string()))
                })
        })
        .unwrap_or_default()
}

pub(super) fn dir_writable_text(cwd: &str) -> String {
    use std::os::unix::fs::MetadataExt;
    let writable = std::fs::metadata(cwd)
        .map(|m| m.mode() & 0o222 != 0)
        .unwrap_or(true);
    if writable { String::new() } else { "!".into() }
}

pub(super) fn per_directory_history_text() -> String {
    match env_var("PER_DIRECTORY_HISTORY_TOGGLE").as_deref() {
        Some("global") => "global".into(),
        Some(_) => "local".into(),
        None => String::new(),
    }
}

pub(super) fn haskell_stack_text() -> String {
    run_cmd("stack", &["--version"])
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .map(|v| v.trim_end_matches(',').to_string())
        })
        .unwrap_or_default()
}

/// Finds `filename` in the cwd's ancestors and returns its full contents.
pub(super) fn find_up_content(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            return Some(c);
        }
        dir = dir.parent()?;
    }
}

/// package segment: `name`/`version` from package.json.
pub(super) fn package_text(cwd: &str) -> String {
    let Some(content) = find_up_content(cwd, "package.json") else {
        return String::new();
    };
    let name = json_str_field(&content, "name");
    let version = json_str_field(&content, "version");
    match (name, version) {
        (Some(n), Some(v)) => format!("{n}@{}", v.trim_start_matches('v')),
        (Some(n), None) => n,
        _ => String::new(),
    }
}

/// Extracts the value of a `"key": "value"` field from a JSON string.
/// One string field of a JSON document. A missing key, a non-string value and invalid JSON
/// all yield `None`.
pub(super) fn json_str_field(content: &str, key: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()?
        .get(key)?
        .as_str()
        .map(str::to_string)
}

/// vi_mode segment: current zsh edit mode → name; empty when not zsh (unreported).
pub(super) fn vi_mode_text(seg: &Segment, mode: crate::model::ViMode) -> String {
    // The text is configurable (p10k `VI_INSERT/COMMAND/VISUAL/OVERWRITE_MODE_STRING`).
    let sym = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    match mode {
        crate::model::ViMode::Command => sym("normal", "NORMAL"),
        crate::model::ViMode::Insert => sym("insert", "INSERT"),
        crate::model::ViMode::Visual => sym("visual", "VISUAL"),
        crate::model::ViMode::Overwrite => sym("overwrite", "OVERWRITE"),
        crate::model::ViMode::Unreported => String::new(), // unreported or emacs: hidden
    }
}

/// asdf segment: first plugin version line of .tool-versions (simplified, first line only).
pub(super) fn asdf_text(cwd: &str) -> String {
    find_up_version(cwd, ".tool-versions").unwrap_or_default()
}

/// fvm segment: Flutter version detected from .fvm/flutter_sdk (or its parent dir).
pub(super) fn fvm_text(cwd: &str) -> String {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let sdk = dir.join(".fvm").join("flutter_sdk");
        if sdk.exists() {
            // Derived from the version path (e.g. …/versions/3.22.0).
            let real = std::fs::canonicalize(&sdk).unwrap_or(sdk);
            if let Some(v) = real.to_string_lossy().split("/versions/").nth(1) {
                return v.split('/').next().unwrap_or("").to_string();
            }
            return "fvm".into();
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    String::new()
}

/// Finds the cwd ancestor directory containing `filename`.
pub(super) fn find_up_dir(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        if dir.join(filename).exists() {
            return Some(dir.to_string_lossy().into_owned());
        }
        dir = dir.parent()?;
    }
}

/// project_id of the GCP service account JSON.
pub(super) fn google_app_cred_text() -> String {
    let Some(path) = env_var("GOOGLE_APPLICATION_CREDENTIALS") else {
        return String::new();
    };
    let Some(content) = std::fs::read_to_string(&path).ok() else {
        return String::new();
    };
    json_str_field(&content, "project_id").unwrap_or_default()
}

/// Elastic Beanstalk environment name (the `* `-prefixed current line of eb list).
pub(super) fn aws_eb_env_text() -> String {
    run_cmd("eb", &["list"])
        .unwrap_or_default()
        .lines()
        .find(|l| l.trim_start().starts_with("* "))
        .map(|l| l.trim_start_matches("* ").trim().to_string())
        .unwrap_or_default()
}

/// Laravel version: runs `php artisan --version` when an ancestor contains artisan.
pub(super) fn laravel_version_text(cwd: &str) -> String {
    let Some(dir) = find_up_dir(cwd, "artisan") else {
        return String::new();
    };
    run_cmd("php", &[&format!("{dir}/artisan"), "--version"])
        .and_then(|s| s.split_whitespace().nth(2).map(|v| v.to_string()))
        .unwrap_or_default()
}

pub(super) fn count_rs(dir: &std::path::Path) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                n += count_rs(&p);
            } else if p.extension().map(|x| x == "rb").unwrap_or(false) {
                n += 1;
            }
        }
    }
    n
}

/// RSpec coverage: ratio of .rb files in app to those in spec.
pub(super) fn rspec_stats_text(cwd: &str) -> String {
    let app = count_rs(&std::path::Path::new(cwd).join("app"));
    let spec = count_rs(&std::path::Path::new(cwd).join("spec"));
    let total = app + spec;
    if total == 0 {
        return String::new();
    }
    format!("RSpec: {}%", app * 100 / total)
}

pub(super) fn todo_text() -> String {
    run_cmd("todo.sh", &["-p", "ls"])
        .and_then(|s| s.lines().last().map(|l| l.to_string()))
        .unwrap_or_default()
}

pub(super) fn taskwarrior_text() -> String {
    run_cmd("task", &["+PENDING", "count"]).unwrap_or_default()
}

pub(super) fn dropbox_text() -> String {
    run_cmd("dropbox-cli", &["filestatus", "."])
        .and_then(|s| s.lines().next().map(|l| l.to_string()))
        .unwrap_or_default()
}

/// date segment: formats the current date per `date-format` (strftime), defaulting to p10k's
/// `%d.%m.%y`.
pub(super) fn date_text(seg: &crate::config::Segment) -> String {
    let fmt = match seg.prop("date-format") {
        Some(crate::config::Prop::Str(s)) => s.as_str(),
        _ => "%d.%m.%y",
    };
    strftime_now(fmt)
}

pub(super) fn strftime_now(fmt: &str) -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    let cfmt = match std::ffi::CString::new(fmt) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let mut buf = [0u8; 128];
    let n = unsafe {
        libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            cfmt.as_ptr(),
            &tm,
        )
    };
    if n == 0 {
        String::new()
    } else {
        String::from_utf8_lossy(&buf[..n]).into_owned()
    }
}

// Toolchain version segments: run `cmd --version`, cache the output and parse the version;
// shown only when the command exists.
thread_local! {
    static CMD_CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

pub(super) fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    let key = format!("{cmd} {}", args.join(" "));
    CMD_CACHE.with(|c| {
        if let Some(v) = c.borrow().get(&key) {
            return v.clone();
        }
        let out = std::process::Command::new(cmd)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                // Some commands (e.g. java) write the version to stderr; fall back when stdout
                // is empty.
                let s = String::from_utf8_lossy(&o.stdout);
                if s.trim().is_empty() {
                    String::from_utf8_lossy(&o.stderr).trim().to_string()
                } else {
                    s.trim().to_string()
                }
            });
        c.borrow_mut().insert(key.clone(), out.clone());
        out
    })
}

pub(super) fn go_version() -> String {
    // "go version go1.27.0-X:nodwarf5 linux/amd64" → "go1.27.0"
    run_cmd("go", &["version"])
        .and_then(|s| {
            Some(
                s.split_whitespace()
                    .nth(2)?
                    .split('-')
                    .next()
                    .unwrap_or("")
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

pub(super) fn rust_version() -> String {
    // "rustc 1.97.1 (…)" → "1.97.1"
    run_cmd("rustc", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

pub(super) fn node_version() -> String {
    // "v26.8.1" → "26.8.1"
    run_cmd("node", &["--version"])
        .map(|s| s.trim_start_matches('v').to_string())
        .unwrap_or_default()
}

pub(super) fn php_version() -> String {
    // "PHP 8.2.0 (cli)" → "8.2.0"
    run_cmd("php", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

pub(super) fn java_version() -> String {
    // '"17.0.9" ...' → "17.0.9" (cut at the first `-`)
    run_cmd("java", &["-fullversion"])
        .and_then(|s| {
            let s = s.split('"').nth(1)?;
            Some(s.split('-').next().unwrap_or(s).to_string())
        })
        .unwrap_or_default()
}

pub(super) fn dotnet_version() -> String {
    run_cmd("dotnet", &["--version"]).unwrap_or_default()
}

pub(super) fn swift_version() -> String {
    // "Apple Swift version 5.9 (…)" → take the first word starting with a digit
    run_cmd("swift", &["--version"])
        .and_then(|s| {
            Some(
                s.split_whitespace()
                    .find(|w| {
                        w.chars()
                            .next()
                            .map(|c| c.is_ascii_digit())
                            .unwrap_or(false)
                    })?
                    .trim_matches('.')
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

pub(super) fn terraform_version() -> String {
    // "Terraform v1.15.9\non linux_amd64…" → "1.15.9" (first line only)
    run_cmd("terraform", &["--version"])
        .and_then(|s| {
            Some(
                s.lines()
                    .next()?
                    .trim_start_matches("Terraform v")
                    .to_string(),
            )
        })
        .unwrap_or_default()
}

pub(super) fn cpu_arch() -> String {
    std::fs::read_to_string("/proc/sys/kernel/arch")
        .ok()
        .or_else(|| run_cmd("uname", &["-m"]))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// Environment manager and *env family segments: current version from an env var or an
// ancestor `.X-version` file; shown only when active.

pub(super) fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// Finds `filename` in cwd ancestors and returns its first non-empty line (version file).
pub(super) fn find_up_version(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            let first = c.lines().next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
        dir = dir.parent()?;
    }
}

pub(super) fn env_or_file(env: &str, file: &str, cwd: &str) -> Option<String> {
    env_var(env).or_else(|| find_up_version(cwd, file))
}

pub(super) fn virtualenv_text(_cwd: &str) -> String {
    env_var("VIRTUAL_ENV")
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

pub(super) fn anaconda_text(_cwd: &str) -> String {
    env_var("CONDA_PREFIX")
        .or_else(|| env_var("CONDA_ENV_PATH"))
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

/// p10k `prompt_symfony2_version`: reads the `app/bootstrap.php.cache` line containing
/// ` VERSION ` and keeps digits and dots only.
pub(super) fn symfony2_version_text(cwd: &str) -> String {
    let path = std::path::Path::new(cwd).join("app/bootstrap.php.cache");
    let Ok(src) = std::fs::read_to_string(&path) else {
        return String::new();
    };
    for line in src.lines() {
        if !line.contains(" VERSION ") {
            continue;
        }
        return line
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '.')
            .collect();
    }
    String::new()
}

/// p10k `prompt_symfony2_tests`: when `src`, `app` and `app/AppKernel.php` exist, computes
/// the share of `src/**/*.php` paths containing `Tests` (p10k's SF2 test ratio).
/// Returns the ratio as text with two decimals, shaped like `SF2: 12.34%`.
pub(super) fn symfony2_tests_text(cwd: &str) -> String {
    let root = std::path::Path::new(cwd);
    if !root.join("src").is_dir()
        || !root.join("app").is_dir()
        || !root.join("app/AppKernel.php").is_file()
    {
        return String::new();
    }
    let mut all = 0usize;
    let mut tests = 0usize;
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "php") {
                all += 1;
                if path.to_string_lossy().contains("Tests") {
                    tests += 1;
                }
            }
        }
    }
    if all == 0 {
        return String::new();
    }
    let code = all - tests;
    if code == 0 {
        return String::new();
    }
    let ratio = 100.0 * tests as f64 / code as f64;
    format!("SF2: {ratio:.2}%")
}

pub(super) fn pyenv_text(cwd: &str) -> String {
    env_or_file("PYENV_VERSION", ".python-version", cwd)
        .map(|v| v.trim_start_matches("python-").to_string())
        .unwrap_or_default()
}

pub(super) fn nodeenv_text(_cwd: &str) -> String {
    env_var("NODE_VIRTUAL_ENV")
        .map(|v| format!("[{}]", basename(&v)))
        .unwrap_or_default()
}

pub(super) fn nodenv_text(cwd: &str) -> String {
    env_or_file("NODENV_VERSION", ".node-version", cwd).unwrap_or_default()
}

pub(super) fn nvm_text(_cwd: &str) -> String {
    // Simplified: an existing NVM_DIR is enough to show the current node version.
    env_var("NVM_DIR")
        .map(|_| node_version())
        .unwrap_or_default()
}

pub(super) fn rbenv_text(cwd: &str) -> String {
    env_or_file("RBENV_VERSION", ".ruby-version", cwd)
        .map(|v| v.trim_start_matches("ruby-").to_string())
        .unwrap_or_default()
}

pub(super) fn chruby_text(_cwd: &str) -> String {
    env_var("RUBY_ENGINE").unwrap_or_default()
}

pub(super) fn rvm_text(_cwd: &str) -> String {
    env_var("GEM_HOME")
        .filter(|g| g.contains("rvm"))
        .map(|g| basename(&g))
        .unwrap_or_default()
}

pub(super) fn goenv_text(cwd: &str) -> String {
    env_or_file("GOENV_VERSION", ".go-version", cwd)
        .map(|v| v.trim_start_matches("go-").to_string())
        .unwrap_or_default()
}

pub(super) fn jenv_text(cwd: &str) -> String {
    env_or_file("JENV_VERSION", ".java-version", cwd).unwrap_or_default()
}

pub(super) fn phpenv_text(cwd: &str) -> String {
    env_or_file("PHPENV_VERSION", ".php-version", cwd).unwrap_or_default()
}

pub(super) fn luaenv_text(cwd: &str) -> String {
    env_or_file("LUAENV_VERSION", ".lua-version", cwd).unwrap_or_default()
}

pub(super) fn plenv_text(cwd: &str) -> String {
    env_or_file("PLENV_VERSION", ".perl-version", cwd).unwrap_or_default()
}

pub(super) fn scalaenv_text(cwd: &str) -> String {
    env_or_file("SCALAENV_VERSION", ".scala-version", cwd).unwrap_or_default()
}

pub(super) fn perlbrew_text(_cwd: &str) -> String {
    env_var("PERLBREW_PERL")
        .map(|v| v.trim_start_matches("perl-").to_string())
        .unwrap_or_default()
}

/// `time` segment text: reads the `time-format` prop; `"12h"` → `HH:MM:SS AM/PM`, 24h by
/// default.
pub(super) fn time_text(seg: &crate::config::Segment) -> String {
    match seg.prop("time-format") {
        Some(crate::config::Prop::Str(s)) if s == "12h" => now_hhmmss_12h(),
        _ => now_hhmmss(),
    }
}

/// Current time HH:MM:SS (libc localtime).
pub(super) fn now_hhmmss() -> String {
    let tm = local_time();
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// 12-hour `HH:MM:SS AM/PM` (matching p10k `%I:%M:%S %p`, hours 01–12).
pub(super) fn now_hhmmss_12h() -> String {
    let tm = local_time();
    let hour12 = tm.tm_hour % 12;
    let hour12 = if hour12 == 0 { 12 } else { hour12 };
    let ampm = if tm.tm_hour < 12 { "AM" } else { "PM" };
    format!("{hour12:02}:{:02}:{:02} {ampm}", tm.tm_min, tm.tm_sec)
}

pub(super) fn local_time() -> libc::tm {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    tm
}

/// Command duration formatting (matching p10k `prompt_command_execution_time`):
/// - `<60s`: `precision` 0 → rounded integer `Ns`; >0 → `N.NNs` (p10k default 2).
/// - `>=60s`: `Xs` / `Xm Ys` / `Xh Ym Zs` / `Xd Xh Ym Zs` (**with spaces**, as in p10k);
///   with `format` set to `H:M:S` instead `MM:SS` / `0H:MM:SS` / `H:MM:SS`.
pub(super) fn format_duration(secs: f64, precision: usize, hms: bool) -> String {
    if secs < 60.0 {
        if precision == 0 {
            format!("{}s", (secs + 0.5) as u64)
        } else {
            format!("{:.*}s", precision, secs)
        }
    } else {
        let d = (secs + 0.5) as u64;
        if hms {
            let (h, m, s) = (d / 3600, d / 60 % 60, d % 60);
            if d >= 36000 {
                format!("{h}:{m:02}:{s:02}")
            } else if d >= 3600 {
                format!("0{h}:{m:02}:{s:02}")
            } else {
                format!("{m:02}:{s:02}")
            }
        } else {
            let mut text = format!("{}s", d % 60);
            if d >= 60 {
                text = format!("{}m {text}", d / 60 % 60);
                if d >= 3600 {
                    text = format!("{}h {text}", d / 3600 % 24);
                    if d >= 86400 {
                        text = format!("{}d {text}", d / 86400);
                    }
                }
            }
            text
        }
    }
}

/// `dir` segment text: shortening (truncate_to_unique) plus per-component coloring by class.
/// - Anchor (`~`/current dir/marker ancestor): `ANCHOR` state
/// - Shortened: `SHORTENED` state
/// - Normal: segment default; the `/` separator keeps its own color (not the component's).
pub(super) fn dir_seg_text(
    config: &Config,
    info: &HeaderInfo,
    seg: &crate::config::Segment,
    default: &Style,
    dir_budget: Option<usize>,
) -> String {
    let shorten = shorten_len(seg);
    let cwd = std::path::Path::new(&info.cwd);
    let home = std::env::var("HOME").ok();
    let home = home.as_deref().map(std::path::Path::new);
    let bool_prop = |name: &str| matches!(seg.prop(name), Some(crate::config::Prop::Bool(true)));
    // p10k `DIR_PATH_ABSOLUTE`: ignore $HOME and show the absolute path; accordingly, home
    // cannot be used as a prefix when splitting, or the full path could not be reconstructed.
    let absolute = bool_prop("path-absolute");
    let parts = crate::dir_shorten::shorten(
        cwd,
        if absolute { None } else { home },
        &dir_shorten_opts(seg, shorten, dir_budget),
    );
    let mut s = String::new();
    let is_home = !absolute && home.map(|h| cwd.starts_with(h)).unwrap_or(false);
    // p10k `DIR_OMIT_FIRST_CHARACTER`: drop the leading `/` of an absolute path (still `/`
    // when cwd=`/`).
    let omit_first = bool_prop("omit-first-character");
    // Home prefix abbreviation (p10k `HOME_FOLDER_ABBREVIATION`, default `~`).
    let abbrev = match seg.prop("home-abbreviation") {
        Some(crate::config::Prop::Str(a)) => a.clone(),
        _ => "~".to_string(),
    };
    // The path separator's own color (p10k `DIR_PATH_SEPARATOR_FOREGROUND`), segment style by
    // default.
    let sep_style = prop_style(seg, default, "path-separator-foreground");
    // p10k `DIR_SHOW_WRITABLE`: when not writable / non-existent the whole segment's state
    // becomes NOT_WRITABLE / NON_EXISTENT (users can color both via `state`).
    let writable_state = dir_writable_state(seg, &info.cwd);
    let is_root = info.cwd == "/";
    if is_root {
        // The root directory has no components; p10k shows a single `/` here.
        s.push_str(&paint("/", &sep_style));
    } else if is_home {
        let st = seg.effective_style(Some("ANCHOR"), &config.defaults);
        s.push_str(&paint(&abbrev, &st));
        if !parts.is_empty() {
            s.push_str(&paint("/", &sep_style));
        }
    } else if !parts.is_empty() && (!omit_first || is_root) {
        // Leading `/` of an absolute path.
        s.push_str(&paint("/", &sep_style));
    }
    // p10k `DIR_PATH_HIGHLIGHT_{FOREGROUND,BOLD}`: the last component (the current directory)
    // is colored on its own.
    let highlight_bold = bool_prop("path-highlight-bold");
    let last = parts.len().saturating_sub(1);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str(&paint("/", &sep_style)); // separator
        }
        let state = match part.class {
            crate::dir_shorten::Class::Anchor => Some("ANCHOR"),
            crate::dir_shorten::Class::Shortened => Some("SHORTENED"),
            crate::dir_shorten::Class::Normal => None,
        };
        // Not-writable/non-existent takes precedence over ANCHOR/SHORTENED (p10k also fixes
        // the state before picking a color); with `dir-classes` the matched class state name is
        // applied first and the not-writable suffix appended
        // (p10k: WORK → WORK_NOT_WRITABLE / WORK_NON_EXISTENT).
        let mut st = match writable_state {
            Some(s) => {
                let named = dir_class_match(config, &info.cwd)
                    .map(|c| format!("{}_{}", c.state, s))
                    .filter(|n| seg.states.contains_key(n))
                    .or_else(|| Some(s.to_string()));
                seg.effective_style(named.as_deref(), &config.defaults)
            }
            None => match dir_class_match(config, &info.cwd) {
                Some(c) if !c.state.is_empty() => {
                    seg.effective_style(Some(&c.state), &config.defaults)
                }
                _ => seg.effective_style(state, &config.defaults),
            },
        };
        if i == last {
            st = prop_style(seg, &st, "path-highlight-foreground");
            if highlight_bold {
                st.bold = true;
            }
        }
        s.push_str(&paint(&part.text, &st));
    }
    let s = if seg.content.is_some() {
        value_of(seg.content.as_deref(), s)
    } else {
        s
    };
    // p10k `DIR_HYPERLINK`: wrap the directory in an OSC 8 hyperlink (absolute paths only).
    if bool_prop("hyperlink") && info.cwd.starts_with('/') {
        format!(
            "\u{1b}]8;;file://{}\u{7}{}\u{1b}]8;;\u{7}",
            url_escape(&info.cwd),
            s
        )
    } else {
        s
    }
}

/// p10k `_p9k_url_escape`: everything outside `[a-zA-Z0-9"/:_.-!'()~]` is escaped as `%XX`.
pub(super) fn url_escape(s: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'"')
        .remove(b'/')
        .remove(b':')
        .remove(b'_')
        .remove(b'.')
        .remove(b'-')
        .remove(b'!')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')')
        .remove(b'~');
    utf8_percent_encode(s, KEEP).to_string()
}

/// Reads the dir segment's `shorten-dir-length` (keep the last N levels, default 1 as in p10k).
pub(super) fn shorten_len(seg: &crate::config::Segment) -> usize {
    if let Some(crate::config::Prop::Int(n)) = seg.prop("shorten-dir-length") {
        (*n).clamp(1, 20) as usize
    } else {
        1
    }
}

/// dir segment shortening options: `shorten-strategy` / `shorten-delimiter` /
/// `shorten-folder-marker` (matching p10k's `SHORTEN_*`).
pub(super) fn dir_shorten_opts(
    seg: &crate::config::Segment,
    length: usize,
    budget: Option<usize>,
) -> crate::dir_shorten::Opts {
    let str_prop = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    crate::dir_shorten::Opts {
        strategy: crate::dir_shorten::Strategy::parse(&str_prop("shorten-strategy", "")),
        length,
        // The p10k engine's default ellipsis is `…`; `truncate_to_unique` does not emit it
        // (consistent with `SHORTEN_DELIMITER=` in p10k's default config).
        delimiter: str_prop("shorten-delimiter", "\u{2026}"),
        marker: str_prop("shorten-folder-marker", ""),
        budget,
    }
}

/// Expands env vars: `${VAR}` or `$VAR` → the value; undefined → empty string.
/// Expand `$VAR` and `${VAR}` in static text. An unset variable expands to nothing, as p10k's
/// `print -P` does; the process environment is the source (this is the user's text, not a test
/// fixture).
pub(super) fn expand_env(s: &str) -> String {
    shellexpand::env_with_context(s, |name| {
        Ok::<Option<String>, ()>(Some(std::env::var(name).unwrap_or_default()))
    })
    .map(|expanded| expanded.into_owned())
    .unwrap_or_else(|_| s.to_string())
}

/// A foreground-color style prop on a segment → style (segment style by default).
pub(super) fn prop_style(seg: &Segment, style: &Style, prop: &str) -> Style {
    match seg.prop(prop) {
        Some(crate::config::Prop::Int(n)) => Style {
            fg: Color::Xterm(i64::clamp(*n, 0, 255) as u8),
            ..style.clone()
        },
        _ => style.clone(),
    }
}

/// vcs segment text, colored piecewise as p10k does: branch name/branch icon/ahead/behind/
/// stash use `clean-foreground`, staged/unstaged counts use `modified-foreground`, the
/// conflict count uses `conflicted-foreground` (falling back to modified), untracked uses
/// `untracked-foreground` (falling back to the segment style). The returned string is colored.
pub(super) fn vcs_text(
    vcs: Option<&GitStatus>,
    branch_icon: &str,
    commit_icon: &str,
    seg: &Segment,
    style: &Style,
) -> String {
    let Some(v) = vcs else { return String::new() };
    let clean = prop_style(seg, style, "clean-foreground");
    let modified = prop_style(seg, style, "modified-foreground");
    let conflicted = prop_style(seg, &modified, "conflicted-foreground");
    let untracked = prop_style(seg, style, "untracked-foreground");
    // p10k's `meta` (%246F gray): the `@` of `@hash`, the `#` of `#tag`, and the remote branch
    // name.
    let meta = prop_style(seg, style, "meta-foreground");
    let mut s = String::new();
    // p10k `SHOW_CHANGESET`: when explicitly enabled, or when there is no local branch
    // (detached HEAD), show the first `changeset-hash-length` chars of the commit (default 8).
    // p10k's `SHOW_CHANGESET` (explicitly enabled, on the piecewise path) draws `icon + hash`.
    let show_changeset = matches!(
        seg.prop("show-changeset"),
        Some(crate::config::Prop::Bool(true))
    );
    let hash_len = match seg.prop("changeset-hash-length") {
        Some(crate::config::Prop::Int(n)) if *n > 0 => *n as usize,
        _ => 8,
    };
    let mut has_commit = false;
    if show_changeset && !v.commit.is_empty() {
        let hash: String = v.commit.chars().take(hash_len).collect();
        let mut t = String::new();
        if !commit_icon.is_empty() {
            t.push_str(commit_icon);
            t.push(' ');
        }
        t.push_str(&hash);
        s.push_str(&paint(&t, &clean));
        has_commit = true;
    }
    if !v.branch.is_empty() {
        let mut b = String::new();
        if has_commit {
            b.push(' ');
        }
        if !branch_icon.is_empty() {
            b.push_str(branch_icon);
            b.push(' ');
        }
        b.push_str(&shorten_branch(&v.branch, seg));
        s.push_str(&paint(&b, &clean));
    } else if !v.tag.is_empty() {
        // p10k: `${meta}#${clean}${tag}`; tag names over 32 chars shorten like branch names.
        s.push_str(&paint("#", &meta));
        s.push_str(&paint(&shorten_branch(&v.tag, seg), &clean));
    } else if !v.commit.is_empty() {
        // p10k: `${meta}@${clean}${VCS_STATUS_COMMIT[1,8]}` (detached HEAD).
        let hash: String = v.commit.chars().take(hash_len).collect();
        s.push_str(&paint("@", &meta));
        s.push_str(&paint(&hash, &clean));
    }
    // p10k:`if [[ -n ${VCS_STATUS_REMOTE_BRANCH:#$VCS_STATUS_LOCAL_BRANCH} ]]`
    // → `${meta}:${clean}${remote_branch}`, directly after the branch name (no space).
    if !v.remote_branch.is_empty() && v.remote_branch != v.branch {
        s.push_str(&paint(":", &meta));
        s.push_str(&paint(&v.remote_branch, &clean));
    }
    // Count symbols can be overridden via props (in p10k these chars are hardcoded in the
    // config's formatting functions; exposing them lets users swap them without touching the
    // engine).
    let sym = |name: &str, default: &str| -> String {
        match seg.prop(name) {
            Some(crate::config::Prop::Str(s)) => s.clone(),
            _ => default.to_string(),
        }
    };
    let mut parts: Vec<(String, &Style)> = Vec::new();
    // Order and symbols match the vcs formatting function in p10k's generated config:
    //   wip → ⇣behind⇡ahead → ⇠push_behind⇢push_ahead → *stashes → <action>
    //   → ~conflicted → +staged → !unstaged → ?untracked → ─
    // No space between ahead/behind, nor between the push pair (p10k has `⇣42⇡42`).
    // p10k: a standalone word wip/WIP in the commit summary inserts a `wip` (modified color).
    if has_wip_word(&v.commit_summary) {
        parts.push(("wip".to_string(), &modified));
    }
    if v.ahead > 0 || v.behind > 0 {
        let mut ab = String::new();
        if v.behind > 0 {
            ab.push_str(&format!("{}{}", sym("behind-symbol", "⇣"), v.behind));
        }
        if v.ahead > 0 {
            ab.push_str(&format!("{}{}", sym("ahead-symbol", "⇡"), v.ahead));
        }
        parts.push((ab, &clean));
    }
    // p10k: `${clean}⇠N` / `${clean}⇢N` (behind/ahead of the push remote).
    if v.push_ahead > 0 || v.push_behind > 0 {
        let mut pb = String::new();
        if v.push_behind > 0 {
            pb.push_str(&format!(
                "{}{}",
                sym("push-behind-symbol", "⇠"),
                v.push_behind
            ));
        }
        if v.push_ahead > 0 {
            pb.push_str(&format!(
                "{}{}",
                sym("push-ahead-symbol", "⇢"),
                v.push_ahead
            ));
        }
        parts.push((pb, &clean));
    }
    if v.stashes > 0 {
        parts.push((format!("{}{}", sym("stash-symbol", "*"), v.stashes), &clean));
    }
    // p10k: `[[ -n $VCS_STATUS_ACTION ]] && res+=" ${conflicted}${VCS_STATUS_ACTION}"`
    // — an in-progress operation such as merge/rebase is drawn in the conflicted color before
    // the conflict count.
    if !v.action.is_empty() {
        parts.push((v.action.clone(), &conflicted));
    }
    if v.conflicted > 0 {
        parts.push((
            format!("{}{}", sym("conflicted-symbol", "~"), v.conflicted),
            &conflicted,
        ));
    }
    if v.staged > 0 {
        parts.push((
            format!("{}{}", sym("staged-symbol", "+"), v.staged),
            &modified,
        ));
    }
    if v.unstaged > 0 {
        parts.push((
            format!("{}{}", sym("unstaged-symbol", "!"), v.unstaged),
            &modified,
        ));
    }
    if v.untracked > 0 {
        parts.push((
            format!("{}{}", sym("untracked-symbol", "?"), v.untracked),
            &untracked,
        ));
    }
    // p10k: `(( VCS_STATUS_HAS_UNSTAGED == -1 )) && res+=" ${modified}─"`:
    // with `max-index-size-dirty` set and the index larger than it the dirty scan is skipped
    // and the unstaged count is unknown (this is the rule the gitstatus plugin uses to set
    // HAS_UNSTAGED to -1).
    let dirty_cap = match seg.prop("max-index-size-dirty") {
        Some(crate::config::Prop::Int(n)) => Some(*n),
        _ => None,
    };
    if let Some(cap) = dirty_cap
        && cap >= 0
        && v.index_size > cap as usize
    {
        parts.push(("─".to_string(), &modified));
    }
    if !parts.is_empty() {
        // Spaces are colored with the segment style: a bare space is drawn with the terminal
        // default background and would show a black seam in a backgrounded segment (classic
        // etc.).
        let sep = paint(" ", style);
        s.push_str(&sep);
        let joined: Vec<String> = parts.iter().map(|(t, st)| paint(t, st)).collect();
        s.push_str(&joined.join(&sep));
    }
    s
}

/// p10k's `[[ $VCS_STATUS_COMMIT_SUMMARY == (|*[^[:alnum:]])(wip|WIP)(|[^[:alnum:]]*) ]]`:
/// `wip` / `WIP` in the commit summary delimited by non-alphanumerics (or the string ends).
pub(super) fn has_wip_word(summary: &str) -> bool {
    let bytes: Vec<char> = summary.chars().collect();
    let is_word = |c: char| c.is_alphanumeric();
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let cand: String = bytes[i..i + 3].iter().collect();
        if cand == "wip" || cand == "WIP" {
            let before_ok = i == 0 || !is_word(bytes[i - 1]);
            let after_ok = i + 3 == bytes.len() || !is_word(bytes[i + 3]);
            if before_ok && after_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Branch name shortening (p10k `VCS_SHORTEN_*`): only when both `shorten-length` and
/// `shorten-min-length` are set and the name is longer than both; `shorten-strategy` picks
/// `truncate_middle` (first N + ellipsis + last N) or `truncate_from_right` (first N +
/// ellipsis, default).
pub(super) fn shorten_branch(branch: &str, seg: &Segment) -> String {
    let num = |name: &str| -> Option<usize> {
        match seg.prop(name) {
            Some(crate::config::Prop::Int(n)) if *n > 0 => Some(*n as usize),
            _ => None,
        }
    };
    let (Some(len), Some(min)) = (num("shorten-length"), num("shorten-min-length")) else {
        return branch.to_string();
    };
    let chars: Vec<char> = branch.chars().collect();
    if chars.len() <= min || chars.len() <= len {
        return branch.to_string();
    }
    let delim = match seg.prop("shorten-delimiter") {
        Some(crate::config::Prop::Str(s)) => s.clone(),
        _ => "\u{2026}".to_string(),
    };
    let head: String = chars[..len].iter().collect();
    let middle = matches!(
        seg.prop("shorten-strategy"),
        Some(crate::config::Prop::Str(s)) if s == "truncate_middle"
    );
    if middle {
        let tail: String = chars[chars.len() - len..].iter().collect();
        format!("{head}{delim}{tail}")
    } else {
        format!("{head}{delim}")
    }
}

/// p10k's `status` visual identifier: FAIL_ICON on a failed command, OK_ICON on a successful
/// one when the segment is verbose, and none without an exit code — a silent success draws
/// neither text nor icon. The icon chars come from the `ok`/`error` icon names (users can
/// override them in the top-level `icon{}`).
pub(super) fn status_icon_key(info: &HeaderInfo, seg: &Segment) -> Option<&'static str> {
    match info.exit_code {
        None => None,
        Some(0) => {
            let verbose = matches!(seg.prop("verbose"), Some(crate::config::Prop::Bool(true)));
            verbose.then_some("ok")
        }
        Some(_) => Some("error"),
    }
}

/// Foreground property of the `status` state the last command left the segment in.
pub(super) fn status_foreground(info: &HeaderInfo) -> &'static str {
    match info.exit_code {
        Some(n) if n != 0 => "error-foreground",
        _ => "ok-foreground",
    }
}

/// The `status` text: the exit code on failure. The icon is not part of it (see
/// [`status_icon_key`]), so the right column draws `1 ✘` rather than `✘ 1`.
pub(super) fn status_text(info: &HeaderInfo, seg: &Segment, style: &Style) -> String {
    match info.exit_code {
        Some(n) if n != 0 => paint(
            &n.to_string(),
            &prop_style(seg, style, status_foreground(info)),
        ),
        _ => String::new(),
    }
}

pub(super) fn value_of(content: Option<&str>, default: impl Into<String>) -> String {
    content.map(String::from).unwrap_or(default.into())
}

/// Assembles one line as ANSI: left segments (joined by `sub`/`segment`, closed by `end`) +
/// right-aligned right segments.
/// Which end of the row a column is: the caps and the separators differ per side.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Left,
    Right,
}

/// Emit one column: start cap, the segments with their separators, end cap. The two sides share
/// the loop; the caps, the separator chars and the fallback spacing differ per side.
fn emit_side(
    parts: &[&SegmentText],
    seps: &crate::config::Separators,
    side: Side,
) -> super::PaintedLine {
    use super::ansi::width;
    let (start_cap, end_cap, sep, sub, sub_fg) = match side {
        Side::Left => (
            &seps.left_tail,
            &seps.end,
            &seps.segment,
            &seps.sub,
            &seps.sub_foreground,
        ),
        Side::Right => (
            &seps.right_start,
            &seps.right_tail,
            &seps.right_segment,
            &seps.right_sub,
            &seps.right_sub_foreground,
        ),
    };
    let mut out = String::new();
    let mut line_width = 0usize;
    if !start_cap.is_empty()
        && let Some(first) = parts.first()
        && first.style.bg != Color::Default
    {
        out.push_str(&arrow(start_cap, first.style.bg.clone(), Color::Default));
        line_width += width(start_cap);
    }
    let mut prev: Option<&SegmentText> = None;
    for s in parts {
        match prev {
            None => {
                // A joined first segment has no left neighbor, so its leading space stays.
                if s.joined_left && (side == Side::Right || s.style.bg != Color::Default) {
                    out.push_str(&paint(" ", &s.style));
                    line_width += 1;
                }
            }
            Some(p) if !s.joined_left => {
                let prev_has = p.style.bg != Color::Default;
                if prev_has {
                    let same = s.style.bg != Color::Default && s.style.bg == p.style.bg;
                    if same {
                        if !sub.is_empty() {
                            let st = match sub_fg {
                                Some(fg) => Style {
                                    fg: fg.clone(),
                                    ..s.style.clone()
                                },
                                None => s.style.clone(),
                            };
                            out.push_str(&paint(sub, &st));
                            line_width += width(sub);
                        } else if side == Side::Left && !p.padded {
                            out.push(' ');
                            line_width += 1;
                        }
                    } else if !sep.is_empty() {
                        // Left: the arrow fg is the previous bg (pointing to the current bg).
                        // Right: the arrow fg is the current bg (the triangle points left at
                        // the previous bg).
                        let (fg, bg) = match side {
                            Side::Left => (p.style.bg.clone(), s.style.bg.clone()),
                            Side::Right => (s.style.bg.clone(), p.style.bg.clone()),
                        };
                        out.push_str(&arrow(sep, fg, bg));
                        line_width += width(sep);
                    } else if side == Side::Left && !p.padded {
                        out.push(' ');
                        line_width += 1;
                    }
                } else if side == Side::Left && !p.padded {
                    out.push(' ');
                    line_width += 1;
                }
            }
            Some(_) => {}
        }
        out.push_str(&s.text); // text is already colored, do not recolor
        line_width += s.width;
        prev = Some(s);
    }
    if !end_cap.is_empty()
        && let Some(last) = parts.last()
        && last.style.bg != Color::Default
    {
        out.push_str(&arrow(end_cap, last.style.bg.clone(), Color::Default));
        line_width += width(end_cap);
    }
    super::PaintedLine {
        text: out,
        width: line_width,
    }
}

pub(super) fn assemble_row(
    left: &[SegmentText],
    right: &[SegmentText],
    cols: usize,
    seps: &crate::config::Separators,
    right_indent: usize,
) -> super::PaintedLine {
    use super::ansi::width;
    let left_parts: Vec<&SegmentText> = left.iter().filter(|s| !s.text.is_empty()).collect();
    let right_parts: Vec<&SegmentText> = right.iter().filter(|s| !s.text.is_empty()).collect();
    let left_line = emit_side(&left_parts, seps, Side::Left);
    let mut out = left_line.text;
    let mut line_width = left_line.width;
    if !right_parts.is_empty() {
        let right_line = emit_side(&right_parts, seps, Side::Right);
        // Right alignment: gap chars fill the span from the left segments to the right
        // column's start. The right column keeps `right_indent` columns from the right edge—
        // zsh's `ZLE_RPROMPT_INDENT` defaults to 1, so p10k's right column always has one cell
        // before the window border; drawing flush would make it "fit" one column earlier than
        // p10k and reach the last cell (many terminals wrap there).
        let edge = cols.saturating_sub(right_indent);
        if edge > right_line.width {
            let start = edge - right_line.width; // right column start (0-based)
            if start > line_width {
                let gap_char = if seps.gap.is_empty() { " " } else { &seps.gap };
                let gap_str = gap_char.repeat(start - line_width);
                line_width += width(&gap_str);
                // gap foreground (p10k MULTILINE_FIRST_PROMPT_GAP_FOREGROUND);
                // unset → emitted as-is (terminal default color).
                match &seps.gap_foreground {
                    Some(fg) => out.push_str(&paint(
                        &gap_str,
                        &Style {
                            fg: fg.clone(),
                            ..Style::default()
                        },
                    )),
                    None => out.push_str(&gap_str),
                }
            }
        }
        out.push_str(&right_line.text);
        line_width += right_line.width;
    }
    super::PaintedLine {
        text: out,
        width: line_width,
    }
}

/// Draws a separator arrow: `fg` is its foreground (joining the previous background), `bg` the
/// background.
pub(super) fn arrow(ch: &str, fg: Color, bg: Color) -> String {
    let mut s = String::new();
    match fg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(&n)).unwrap(),
    }
    match bg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(&n)).unwrap(),
    }
    s.push_str(ch);
    s.push_str("\x1b[0m");
    s
}

pub(crate) fn display_width(s: &str) -> usize {
    super::ansi::width(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_fields_are_parsed_as_json() {
        let doc = r#"{
            "name": "p11k",
            "version": "1.2.3",
            "nested": { "name": "inner" },
            "count": 3,
            "escaped": "a\"b\\c"
        }"#;
        assert_eq!(json_str_field(doc, "name").as_deref(), Some("p11k"));
        assert_eq!(json_str_field(doc, "version").as_deref(), Some("1.2.3"));
        // The old hand parser read the first `name` it found, including a nested key; a real
        // parser reads the top level.
        assert_eq!(json_str_field(doc, "nested").as_deref(), None);
        // A non-string value is not a string field.
        assert_eq!(json_str_field(doc, "count").as_deref(), None);
        // Escapes are decoded.
        assert_eq!(json_str_field(doc, "escaped").as_deref(), Some("a\"b\\c"));
        // Invalid JSON and a missing key yield None.
        assert_eq!(json_str_field("not json", "name").as_deref(), None);
        assert_eq!(json_str_field(doc, "missing").as_deref(), None);
    }

    #[test]
    fn env_expansion_keeps_the_p10k_semantics() {
        // A set variable expands; an unset one expands to nothing.
        let path = std::env::var("PATH").expect("the test process has PATH");
        assert_eq!(expand_env("${PATH}"), path);
        assert_eq!(expand_env("$PATH"), path);
        assert_eq!(expand_env("a${PATH}b"), format!("a{path}b"));
        assert_eq!(expand_env("x$P11K_TEST_UNDEFINED_VAR-y"), "x-y");
        // The unset variable expands to empty; a `${VAR:-default}` default is not applied,
        // which keeps the previous behaviour (p10k's `print -P` would apply it, but static
        // text never used that form).
        assert_eq!(expand_env("${P11K_TEST_UNDEFINED_VAR:-fallback}"), "");
    }

    #[test]
    fn url_escape_keeps_the_p10k_set() {
        assert_eq!(url_escape("/home/u/proj"), "/home/u/proj");
        assert_eq!(url_escape("a b"), "a%20b");
        assert_eq!(url_escape("caf\u{e9}"), "caf%C3%A9");
        // Quotes and the other allowed characters stay literal.
        assert_eq!(url_escape("\"x\"/:_.-!'()~"), "\"x\"/:_.-!'()~");
    }
}
