//! Icons: per-mode glyph selection, overrides, the segment icon table and the directory class
//! icons.
//!
//! The rendered glyph depends on the icon mode, the configured overrides, the segment name and
//! the current directory. This module keeps the tables and the resolution order together.

use crate::config::Config;
use crate::theme::GitStatus;
use std::cell::RefCell;

use super::glob::glob_match_path;
use super::{
    aws_eb_env_text, current_dir, detect_os_icon, dir_writable_state, dir_writable_text,
    dropbox_text, env, env_has_proxy, env_var, fvm_text, haskell_stack_text, in_nix_shell,
    laravel_version_text, level_icon, package_text, per_directory_history_text, rspec_stats_text,
    taskwarrior_text, todo_text, toolbox_text,
};

pub(super) fn icon_by_mode(e: &IconEntry, mode: &crate::config::IconMode) -> String {
    match mode {
        crate::config::IconMode::NerdfontComplete | crate::config::IconMode::NerdfontFontconfig => {
            e.nerdfont
        }
        crate::config::IconMode::Compatible => e.compatible,
        crate::config::IconMode::Ascii => e.ascii,
    }
    .to_string()
}

/// The built-in icon chars of a segment across the three font tiers.
/// `nerdfont-complete` and `nerdfont-fontconfig` are the same case branch with identical
/// glyphs in p10k's source (`internal/icons.zsh`), so they merge into one `nerdfont` tier;
/// `compatible` uses standard Unicode + Powerline fonts and `ascii` is plain ASCII.
/// The icon data lives in the single [`icon_default`] table.
#[derive(Clone, Copy)]
pub(super) struct IconEntry {
    nerdfont: &'static str,
    compatible: &'static str,
    ascii: &'static str,
}

/// Icon table construction helper: three-tier chars → [`IconEntry`].
pub(super) const fn icon(n: &'static str, c: &'static str, a: &'static str) -> IconEntry {
    IconEntry {
        nerdfont: n,
        compatible: c,
        ascii: a,
    }
}

/// Icon name → default chars for the three font tiers.
/// Segment rendering resolves icon names via [`segment_icon_key`];
/// users can override them by icon name in the top-level `icon{}`.
pub(super) fn icon_default(key: &str) -> Option<IconEntry> {
    match key {
        // p10k's three directory icons (HOME_ICON / HOME_SUB_ICON / FOLDER_ICON): the dir
        // segment picks one through its built-in classes, see `default_dir_icon_key`.
        "folder" => Some(icon("\u{f115}", "", "")), // p10k FOLDER_ICON
        "home" => Some(icon("\u{f015}", "", "")),   // p10k HOME_ICON
        "home-sub" => Some(icon("\u{f07c}", "", "")), // p10k HOME_SUB_ICON
        "etc" => Some(icon("\u{f013}", "\u{2699}", "")), // p10k ETC_ICON ⚙
        "git" => Some(icon("\u{f1d3}", "", "")),
        "commit" => Some(icon("\u{e729}", "", "@")), // p10k VCS_COMMIT_ICON
        "time" => Some(icon("\u{f017}", "", "")),    // clock
        "date" => Some(icon("\u{f073}", "", "")),    // calendar
        // p10k EXECUTION_TIME_ICON: U+F252 (hourglass) in the nerdfont tier,
        // empty in compatible/ascii since p10k itself has none.
        "execution-time" => Some(icon("\u{f252}", "", "")),
        // p10k RULER_CHAR: `─` in the nerdfont/compatible tiers, `-` in ascii.
        "ruler" => Some(icon("\u{2500}", "\u{2500}", "-")),
        "background-jobs" => Some(icon("\u{f013}", "\u{2699}", "%%")), // gear ⚙ %%
        "go" => Some(icon("\u{e626}", "Go", "go")),
        "rust" => Some(icon("\u{e7a8}", "R", "rust")),
        "node" => Some(icon("\u{e617}", "Node", "node")),
        "php" => Some(icon("\u{e608}", "php", "php")),
        "java" => Some(icon("\u{e738}", "\u{2615}", "java")), // ☕
        "dotnet" => Some(icon("\u{e77f}", ".NET", ".net")),
        "terraform" => Some(icon("\u{f1bb}", "tf", "tf")),
        "cpu-arch" => Some(icon("\u{e266}", "arch", "arch")),
        "python" => Some(icon("\u{e73c}", "Py", "py")),
        "ruby" => Some(icon("\u{f219}", "Ruby", "rb")),
        "lua" => Some(icon("\u{e620}", "lua", "lua")),
        "perl" => Some(icon("\u{e769}", "perl", "perl")),
        "scala" => Some(icon("\u{e737}", "scala", "scala")),
        "load" => Some(icon("\u{f080}", "L", "cpu")),
        "ram" => Some(icon("\u{f0e4}", "RAM", "ram")),
        "swap" => Some(icon("\u{f464}", "SWP", "swap")),
        "battery" => Some(icon("\u{f240}", "\u{1F50B}", "battery")), // 🔋
        "disk" => Some(icon("\u{f0a0}", "hdd", "disk")),
        "aws" => Some(icon("\u{f270}", "AWS", "aws")),
        "azure" => Some(icon("\u{fd03}", "\u{2601}", "az")), // ☁
        "gcloud" => Some(icon("\u{f7b7}", "G", "gcloud")),
        "kube" => Some(icon("\u{2388}", "\u{2388}", "kube")), // ⎈
        "network" => Some(icon("\u{f50d}", "IP", "ip")),
        "vpn" => Some(icon("\u{f023}", "vpn", "vpn")),
        "wifi" => Some(icon("\u{f1eb}", "WiFi", "wifi")),
        "public-ip" => Some(icon("\u{f0ac}", "IP", "ip")),
        "toolbox" => Some(icon("\u{e20f}", "\u{2b22}", "toolbox")), // ⬢
        "lock" => Some(icon("\u{f023}", "\u{e0a2}", "!w")),
        "history" => Some(icon("\u{f1da}", "hist", "hist")),
        "haskell" => Some(icon("\u{e61f}", "hs", "hs")),
        "package" => Some(icon("\u{f8d6}", "pkg", "pkg")),
        "flutter" => Some(icon("F", "F", "flutter")),
        "aws-eb" => Some(icon("\u{f1bd}", "\u{1F331}", "eb")), // 🌱
        "laravel" => Some(icon("\u{e73f}", "", "")),
        "test" => Some(icon("\u{f188}", "", "")),
        // p10k SYMFONY_ICON: U+E757 in the nerdfont tier, `SF` in the others.
        "symfony" => Some(icon("\u{e757}", "SF", "SF")),
        "todo" => Some(icon("\u{2611}", "\u{2206}", "todo")), // ☑ ∆
        "taskwarrior" => Some(icon("\u{f4a0}", "task", "task")),
        "dropbox" => Some(icon("\u{f16b}", "Dropbox", "dropbox")),
        "ssh" => Some(icon("\u{f489}", "ssh", "ssh")),
        "proxy" => Some(icon("\u{2194}", "\u{2194}", "proxy")), // ↔
        "server" => Some(icon("\u{f0ae}", "", "")),
        "ranger" => Some(icon("\u{f00b}", "\u{2b50}", "ranger")), // ⭐
        "yazi" => Some(icon("\u{f00b}", "\u{2b50}", "yazi")),
        "nnn" => Some(icon("nnn", "nnn", "nnn")),
        "lf" => Some(icon("lf", "lf", "lf")),
        "nix" => Some(icon("\u{f313}", "nix", "nix")),
        "xplr" => Some(icon("xplr", "xplr", "xplr")),
        "mc" => Some(icon("mc", "mc", "mc")),
        "vim" => Some(icon("\u{e62b}", "vim", "vim")),
        "direnv" => Some(icon("\u{25bc}", "\u{25bc}", "direnv")), // ▼
        "chezmoi" => Some(icon("\u{f015}", "Chez", "chezmoi")),
        // status OK/ERROR and the vcs branch (p10k OK_ICON/FAIL_ICON/VCS_BRANCH_ICON).
        "ok" => Some(icon("\u{f00c}", "\u{2714}", "ok")),
        "error" => Some(icon("\u{f00d}", "\u{2718}", "err")),
        "branch" => Some(icon("\u{f126}", "@", "")),
        _ => None,
    }
}

/// Segment → the icon name it references by default. Conditional segments (shown only when
/// the environment allows) evaluate their condition here; unmet → None (no icon, which hides
/// the segment together with its empty text).
pub(super) fn segment_icon_key(name: &str) -> Option<&'static str> {
    match name {
        "dir" => Some("folder"),
        "vcs" => Some("git"),
        "time" => Some("time"),
        "date" => Some("date"),
        "command_execution_time" => Some("execution-time"),
        "symfony2_version" => Some("symfony"),
        "symfony2_tests" => Some("test"),
        "background_jobs" => Some("background-jobs"),
        "go_version" | "goenv" => Some("go"),
        "rust_version" => Some("rust"),
        "node_version" | "nodeenv" | "nodenv" | "nvm" => Some("node"),
        "php_version" | "phpenv" => Some("php"),
        "java_version" | "jenv" => Some("java"),
        "dotnet_version" => Some("dotnet"),
        "terraform_version" | "terraform" => Some("terraform"),
        "cpu_arch" => Some("cpu-arch"),
        "virtualenv" | "anaconda" | "pyenv" => Some("python"),
        "rbenv" | "chruby" | "rvm" => Some("ruby"),
        "luaenv" => Some("lua"),
        "plenv" | "perlbrew" => Some("perl"),
        "scalaenv" => Some("scala"),
        "load" => Some("load"),
        "ram" => Some("ram"),
        "swap" => Some("swap"),
        "battery" => Some("battery"),
        "disk_usage" => Some("disk"),
        "aws" => Some("aws"),
        "azure" => Some("azure"),
        "gcloud" | "google_app_cred" => Some("gcloud"),
        "kubecontext" => Some("kube"),
        "ip" => Some("network"),
        "vpn_ip" => Some("vpn"),
        "wifi" => Some("wifi"),
        "public_ip" => Some("public-ip"),
        "toolbox" => (!toolbox_text().is_empty()).then_some("toolbox"),
        "dir_writable" => (!dir_writable_text(&current_dir()).is_empty()).then_some("lock"),
        "per_directory_history" => (!per_directory_history_text().is_empty()).then_some("history"),
        "haskell_stack" => (!haskell_stack_text().is_empty()).then_some("haskell"),
        "package" => (!package_text(&current_dir()).is_empty()).then_some("package"),
        "fvm" => (!fvm_text(&current_dir()).is_empty()).then_some("flutter"),
        "aws_eb_env" => (!aws_eb_env_text().is_empty()).then_some("aws-eb"),
        "laravel_version" => {
            (!laravel_version_text(&current_dir()).is_empty()).then_some("laravel")
        }
        "rspec_stats" => (!rspec_stats_text(&current_dir()).is_empty()).then_some("test"),
        "todo" => (!todo_text().is_empty()).then_some("todo"),
        "taskwarrior" => (!taskwarrior_text().is_empty()).then_some("taskwarrior"),
        "dropbox" => (!dropbox_text().is_empty()).then_some("dropbox"),
        // Environment indicator segments: the icon is conditional too; unmet → None.
        "ssh" => env().ssh.then_some("ssh"),
        "proxy" => env_has_proxy().then_some("proxy"),
        "docker_machine" => env_var("DOCKER_MACHINE_NAME").is_some().then_some("server"),
        "ranger" => level_icon("RANGER_LEVEL", "\u{f00b}")
            .is_some()
            .then_some("ranger"),
        "yazi" => level_icon("YAZI_LEVEL", "\u{f00b}")
            .is_some()
            .then_some("yazi"),
        "nnn" => level_icon("NNNLVL", "nnn").is_some().then_some("nnn"),
        "lf" => level_icon("LF_LEVEL", "lf").is_some().then_some("lf"),
        "nix_shell" => in_nix_shell().then_some("nix"),
        "xplr" => env_var("XPLR_PID").is_some().then_some("xplr"),
        "midnight_commander" => env_var("MC_TMPDIR").is_some().then_some("mc"),
        "vim_shell" => env_var("VIMRUNTIME").is_some().then_some("vim"),
        "direnv" => env_var("DIRENV_DIR").is_some().then_some("direnv"),
        "chezmoi_shell" => env_var("CHEZMOI").is_some().then_some("chezmoi"),
        _ => None,
    }
}

/// Resolves an icon name: top-level `icon{}` overrides (auto-detected category via all plus
/// exact nf/compat/ascii tiers) → otherwise the built-in default table (per mode).
pub(super) fn icon_str(config: &Config, key: &str) -> Option<String> {
    if let Some(ov) = config.icon_overrides.get(key) {
        let exact = match config.mode {
            crate::config::IconMode::NerdfontComplete
            | crate::config::IconMode::NerdfontFontconfig => ov.nf.as_ref(),
            crate::config::IconMode::Compatible => ov.compat.as_ref(),
            crate::config::IconMode::Ascii => ov.ascii.as_ref(),
        };
        if let Some(c) = exact {
            return Some(c.clone());
        }
        if let Some(a) = &ov.all
            && all_covers(a, &config.mode)
        {
            return Some(a.clone());
        }
    }
    let e = icon_default(key)?;
    Some(icon_by_mode(&e, &config.mode))
}

/// Auto-detection for the `all` field: NF Private Use chars land in the nf tier only,
/// standard ones in nf+compat, plain ASCII in all three.
pub(super) fn all_covers(s: &str, mode: &crate::config::IconMode) -> bool {
    match mode {
        crate::config::IconMode::NerdfontComplete | crate::config::IconMode::NerdfontFontconfig => {
            true
        }
        crate::config::IconMode::Compatible => {
            s.chars().all(|c| !(0xE000..=0xF8FF).contains(&(c as u32)))
        }
        crate::config::IconMode::Ascii => s.is_ascii(),
    }
}

/// Default icon resolution for segment rendering: os is dynamic, vcs remote takes priority,
/// the rest go segment → icon name → table lookup.
pub(super) fn resolve_icon(
    config: &Config,
    name: &str,
    vcs: Option<&GitStatus>,
    cwd: &str,
) -> Option<String> {
    if name == "os" || name == "os_icon" {
        return Some(os_icon(&config.mode));
    }
    if name == "vcs" {
        if let Some(v) = vcs {
            for (domain, ic) in &config.vcs_remote_icons {
                if !domain.is_empty() && v.remote_url.contains(domain) {
                    return Some(ic.clone());
                }
            }
        }
        return icon_str(config, "git");
    }
    // p10k `DIR_SHOW_WRITABLE`: when not writable (under v3 also non-existent) the dir icon
    // becomes a lock.
    if name == "dir" {
        if dir_writable_state(config.segment("dir"), cwd).is_some() {
            return icon_str(config, "lock");
        }
        // p10k ships four `DIR_CLASSES` of its own and uses them whenever the config declares
        // none (`if (( $+_POWERLEVEL9K_DIR_CLASSES ))` ... else). Only the icon is resolved
        // here: p10k's default class names would otherwise shadow ANCHOR/SHORTENED, which
        // p11k derives from the shortened path instead.
        if config.dir_classes.is_empty() {
            return icon_str(config, default_dir_icon_key(cwd));
        }
        // p10k `DIR_CLASSES`: the matched rule's icon; an empty icon in p10k means no icon at
        // all, not a fallback to the default folder icon (measured: with HOME's icon='', p10k
        // draws nothing).
        if let Some(c) = dir_class_match(config, cwd) {
            return if c.icon.is_empty() {
                None
            } else {
                Some(c.icon.clone())
            };
        }
    }
    let key = segment_icon_key(name)?;
    icon_str(config, key)
}

/// p10k's built-in `DIR_CLASSES`, in their order: `/etc|/etc/*`, `~`, `~/*`, and the
/// catch-all. Returns the icon name of the first match.
///
/// `*` is written as `**` because zsh matches a pattern's `*` against `/` too, while p11k's
/// glob stops at a separator: p10k gives every directory below $HOME the HOME_SUB_ICON, not
/// just the direct children (measured on `~/Projects/powerlevel11k`).
pub(super) fn default_dir_icon_key(cwd: &str) -> &'static str {
    let home = std::env::var("HOME").unwrap_or_default();
    let hit = |pattern: &str| glob_match_path(pattern, cwd, &home);
    if hit("/etc") || hit("/etc/**") {
        "etc"
    } else if !home.is_empty() && hit("~") {
        "home"
    } else if !home.is_empty() && hit("~/**") {
        "home-sub"
    } else {
        "folder"
    }
}

/// Matches cwd against `dir-classes` in order and returns the first hit (p10k `DIR_CLASSES`).
pub(super) fn dir_class_match<'a>(
    config: &'a Config,
    cwd: &str,
) -> Option<&'a crate::config::DirClass> {
    let home = std::env::var("HOME").unwrap_or_default();
    config
        .dir_classes
        .iter()
        .find(|c| glob_match_path(&c.pattern, cwd, &home))
}

/// Segments whose icon is the content: the icon is drawn even with empty text (env
/// indicators, os badges). For every other segment the icon is decoration, and empty text
/// hides the whole segment.
pub(super) fn icon_is_content(name: &str) -> bool {
    matches!(
        name,
        "ssh"
            | "xplr"
            | "midnight_commander"
            | "vim_shell"
            | "direnv"
            | "chezmoi_shell"
            | "os"
            | "os_icon"
            // A verbose, successful status is its icon alone (p10k draws `✔` with no text).
            | "status"
    )
}

thread_local! {
    static OS_ICON: RefCell<Option<IconEntry>> = const { RefCell::new(None) };
}

/// os icon: uname family + /etc/os-release ID to match the distro (matching p10k
/// `_p9k_set_os`), returned per font tier by mode.
pub(super) fn os_icon(mode: &crate::config::IconMode) -> String {
    OS_ICON.with(|c| {
        let e = *c.borrow_mut().get_or_insert_with(detect_os_icon);
        icon_by_mode(&e, mode)
    })
}
