//! Icons: per-mode glyph selection, overrides, the segment icon table and the directory class
//! icons.
//!
//! The rendered glyph depends on the icon mode, the configured overrides, the segment name and
//! the current directory. This module keeps the tables and the resolution order together.

use crate::config::Config;
use crate::theme::GitStatus;
use std::cell::RefCell;

use super::dir_writable_state;
use super::glob::glob_match_path;

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

/// Resolves an icon name: top-level `icon{}` overrides (auto-detected category via all plus
/// exact nf/compat/ascii tiers) → otherwise the built-in default table (per mode).
pub(super) fn icon_str(config: &Config, key: &str) -> Option<String> {
    // An empty icon means "no icon", not "a zero-width icon": the default table is empty for
    // some segments in the compatible and ascii tiers, and an empty override must not fall back
    // to the default. Returning None keeps the padding and the middle text slot out of the row.
    fn non_empty(s: String) -> Option<String> {
        (!s.is_empty()).then_some(s)
    }
    if let Some(ov) = config.icon_overrides.get(key) {
        let exact = match config.mode {
            crate::config::IconMode::NerdfontComplete
            | crate::config::IconMode::NerdfontFontconfig => ov.nf.as_ref(),
            crate::config::IconMode::Compatible => ov.compat.as_ref(),
            crate::config::IconMode::Ascii => ov.ascii.as_ref(),
        };
        if let Some(c) = exact {
            return non_empty(c.clone());
        }
        if let Some(a) = &ov.all
            && all_covers(a, &config.mode)
        {
            return non_empty(a.clone());
        }
    }
    let e = icon_default(key)?;
    non_empty(icon_by_mode(&e, &config.mode))
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
    // The two special segments keep their icon keys here; every other segment reads the key
    // from the registry row, so the icon and the text come from one place.
    if name == "command_execution_time" {
        return icon_str(config, "execution-time");
    }
    if name == "symfony2_tests" {
        return icon_str(config, "test");
    }
    let key = super::registry::find(name)?.icon_key()?;
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

thread_local! {
    static OS_ICON: RefCell<Option<IconEntry>> = const { RefCell::new(None) };
}

fn detect_os_icon() -> IconEntry {
    let uname = std::env::consts::OS;
    if uname != "linux" {
        return match uname {
            "macos" => icon("\u{f179}", "OSX", "mac"),   // 
            "windows" => icon("\u{f17a}", "WIN", "win"), // 
            _ => icon("\u{f17c}", "Lx", "linux"),        // default linux icon
        };
    }
    // Linux: read the ID from /etc/os-release (substring match, matching p10k case *arch*
    // etc.).
    let id = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find(|l| l.starts_with("ID=")).map(|l| {
                l.trim_start_matches("ID=")
                    .trim()
                    .trim_matches('"')
                    .to_string()
            })
        })
        .unwrap_or_default();
    // The three-tier text abbreviations match p10k's compatible/ascii branches.
    if id.contains("arch") {
        icon("\u{f303}", "Arc", "arch")
    } else if id.contains("ubuntu") {
        icon("\u{f31b}", "Ubu", "ubuntu")
    } else if id.contains("debian") {
        icon("\u{f306}", "Deb", "debian")
    } else if id.contains("fedora") {
        icon("\u{f30a}", "Fed", "fedora")
    } else if id.contains("gentoo") {
        icon("\u{f30d}", "Gen", "gentoo")
    } else if id.contains("nixos") {
        icon("\u{f313}", "Nix", "nixos")
    } else if id.contains("manjaro") {
        icon("\u{f312}", "Man", "manjaro")
    } else if id.contains("mint") {
        icon("\u{f30e}", "LMi", "mint")
    } else if id.contains("alpine") {
        icon("\u{f300}", "Alp", "alpine")
    } else if id.contains("void") {
        icon("\u{f32e}", "Vo", "void")
    } else if id.contains("artix") {
        icon("\u{f31f}", "Art", "artix")
    } else if id.contains("opensuse") || id.contains("suse") {
        icon("\u{f314}", "OSu", "suse")
    } else {
        icon("\u{f17c}", "Lx", "linux")
    }
}

/// os icon: uname family + /etc/os-release ID to match the distro (matching p10k
/// `_p9k_set_os`), returned per font tier by mode.
pub(super) fn os_icon(mode: &crate::config::IconMode) -> String {
    OS_ICON.with(|c| {
        let e = *c.borrow_mut().get_or_insert_with(detect_os_icon);
        icon_by_mode(&e, mode)
    })
}
