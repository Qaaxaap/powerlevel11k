//! Segment registry: one table for every segment that only delegates to a text function.
//!
//! Each entry holds the text source and the icon source together, so adding a segment means one
//! table row instead of a match arm plus an icon-table arm. The special segments that need the
//! renderer state (`dir`, `vcs`, `status`, `prompt_char`, `command_execution_time`,
//! `symfony2_tests`) keep their branch in the dispatcher.
//!
//! The icon is rendered from the same row: the visible identifier is the icon key for the mode,
//! and the renderer asks for it once per segment.

use crate::config::Segment;
use crate::theme::HeaderInfo;

use super::env::*;
use super::segments::*;

/// Where a segment's text comes from.
pub(super) enum TextSource {
    Plain(fn() -> String),
    Cwd(fn(&str) -> String),
    Segment(fn(&Segment) -> String),
    Arg(fn(&str) -> String, &'static str),
    Env(&'static str),
    /// The number of background jobs.
    Jobs,
    History,
    Empty,
}

/// How a segment's icon is chosen.
pub(super) enum IconSource {
    None,
    Key(&'static str),
    Cond(fn() -> Option<&'static str>),
    /// The icon is the content: it draws even with empty text.
    ContentCond(fn() -> Option<&'static str>),
}

/// One segment row.
pub(super) struct SegmentSpec {
    pub(super) name: &'static str,
    pub(super) text: TextSource,
    pub(super) icon: IconSource,
}

const fn spec(name: &'static str, text: TextSource, icon: IconSource) -> SegmentSpec {
    SegmentSpec { name, text, icon }
}

impl SegmentSpec {
    /// The plain text of the segment (no colour, no padding).
    pub(super) fn plain_text(&self, info: &HeaderInfo, seg: &Segment) -> String {
        match self.text {
            TextSource::Plain(f) => f(),
            TextSource::Cwd(f) => f(&info.cwd),
            TextSource::Segment(f) => f(seg),
            TextSource::Arg(f, a) => f(a),
            TextSource::Env(k) => env_var(k).unwrap_or_default(),
            TextSource::Jobs => {
                if info.jobs > 0 {
                    info.jobs.to_string()
                } else {
                    String::new()
                }
            }
            TextSource::History => info.history.to_string(),
            TextSource::Empty => String::new(),
        }
    }

    /// The icon key for the current state, if the segment has one.
    pub(super) fn icon_key(&self) -> Option<&'static str> {
        match self.icon {
            IconSource::None => None,
            IconSource::Key(k) => Some(k),
            IconSource::Cond(f) | IconSource::ContentCond(f) => f(),
        }
    }

    /// Whether the icon is the content, so an empty text still draws.
    pub(super) fn icon_is_content(&self) -> bool {
        matches!(self.icon, IconSource::ContentCond(_))
    }
}

/// The registry, ordered by segment name.
pub(super) static SEGMENTS: &[SegmentSpec] = &[
    spec(
        "time",
        TextSource::Segment(time_text),
        IconSource::Key("time"),
    ),
    spec("context", TextSource::Plain(context_text), IconSource::None),
    spec("user", TextSource::Plain(user_name), IconSource::None),
    spec("host", TextSource::Plain(host_text), IconSource::None),
    spec(
        "root_indicator",
        TextSource::Plain(root_indicator_text),
        IconSource::None,
    ),
    spec(
        "date",
        TextSource::Segment(date_text),
        IconSource::Key("date"),
    ),
    spec(
        "symfony2_version",
        TextSource::Cwd(symfony2_version_text),
        IconSource::Key("symfony"),
    ),
    spec(
        "go_version",
        TextSource::Plain(go_version),
        IconSource::Key("go"),
    ),
    spec(
        "rust_version",
        TextSource::Plain(rust_version),
        IconSource::Key("rust"),
    ),
    spec(
        "node_version",
        TextSource::Plain(node_version),
        IconSource::Key("node"),
    ),
    spec(
        "php_version",
        TextSource::Plain(php_version),
        IconSource::Key("php"),
    ),
    spec(
        "java_version",
        TextSource::Plain(java_version),
        IconSource::Key("java"),
    ),
    spec(
        "dotnet_version",
        TextSource::Plain(dotnet_version),
        IconSource::Key("dotnet"),
    ),
    spec(
        "swift_version",
        TextSource::Plain(swift_version),
        IconSource::None,
    ),
    spec(
        "terraform_version",
        TextSource::Plain(terraform_version),
        IconSource::Key("terraform"),
    ),
    spec(
        "cpu_arch",
        TextSource::Plain(cpu_arch),
        IconSource::Key("cpu-arch"),
    ),
    spec(
        "virtualenv",
        TextSource::Cwd(virtualenv_text),
        IconSource::Key("python"),
    ),
    spec(
        "anaconda",
        TextSource::Cwd(anaconda_text),
        IconSource::Key("python"),
    ),
    spec(
        "pyenv",
        TextSource::Cwd(pyenv_text),
        IconSource::Key("python"),
    ),
    spec(
        "nodeenv",
        TextSource::Cwd(nodeenv_text),
        IconSource::Key("node"),
    ),
    spec(
        "nodenv",
        TextSource::Cwd(nodenv_text),
        IconSource::Key("node"),
    ),
    spec("nvm", TextSource::Cwd(nvm_text), IconSource::Key("node")),
    spec(
        "rbenv",
        TextSource::Cwd(rbenv_text),
        IconSource::Key("ruby"),
    ),
    spec(
        "chruby",
        TextSource::Cwd(chruby_text),
        IconSource::Key("ruby"),
    ),
    spec("rvm", TextSource::Cwd(rvm_text), IconSource::Key("ruby")),
    spec("goenv", TextSource::Cwd(goenv_text), IconSource::Key("go")),
    spec("jenv", TextSource::Cwd(jenv_text), IconSource::Key("java")),
    spec(
        "phpenv",
        TextSource::Cwd(phpenv_text),
        IconSource::Key("php"),
    ),
    spec(
        "luaenv",
        TextSource::Cwd(luaenv_text),
        IconSource::Key("lua"),
    ),
    spec(
        "plenv",
        TextSource::Cwd(plenv_text),
        IconSource::Key("perl"),
    ),
    spec(
        "scalaenv",
        TextSource::Cwd(scalaenv_text),
        IconSource::Key("scala"),
    ),
    spec(
        "perlbrew",
        TextSource::Cwd(perlbrew_text),
        IconSource::Key("perl"),
    ),
    spec("load", TextSource::Plain(load), IconSource::Key("load")),
    spec("ram", TextSource::Plain(ram), IconSource::Key("ram")),
    spec("swap", TextSource::Plain(swap), IconSource::Key("swap")),
    spec(
        "disk_usage",
        TextSource::Cwd(disk_usage),
        IconSource::Key("disk"),
    ),
    spec(
        "battery",
        TextSource::Plain(battery),
        IconSource::Key("battery"),
    ),
    spec("aws", TextSource::Plain(aws_text), IconSource::Key("aws")),
    spec(
        "azure",
        TextSource::Plain(azure_text),
        IconSource::Key("azure"),
    ),
    spec(
        "gcloud",
        TextSource::Plain(gcloud_text),
        IconSource::Key("gcloud"),
    ),
    spec(
        "kubecontext",
        TextSource::Cwd(kubecontext_text),
        IconSource::Key("kube"),
    ),
    spec(
        "terraform",
        TextSource::Cwd(terraform_text),
        IconSource::Key("terraform"),
    ),
    spec("ip", TextSource::Plain(ip_text), IconSource::Key("network")),
    spec(
        "vpn_ip",
        TextSource::Plain(vpn_ip_text),
        IconSource::Key("vpn"),
    ),
    spec(
        "wifi",
        TextSource::Plain(wifi_text),
        IconSource::Key("wifi"),
    ),
    spec(
        "public_ip",
        TextSource::Plain(public_ip_text),
        IconSource::Key("public-ip"),
    ),
    spec(
        "detect_virt",
        TextSource::Plain(detect_virt),
        IconSource::None,
    ),
    spec(
        "toolbox",
        TextSource::Plain(toolbox_text),
        IconSource::Cond(icon_toolbox),
    ),
    spec(
        "dir_writable",
        TextSource::Cwd(dir_writable_text),
        IconSource::Cond(icon_dir_writable),
    ),
    spec(
        "per_directory_history",
        TextSource::Plain(per_directory_history_text),
        IconSource::Cond(icon_per_directory_history),
    ),
    spec(
        "haskell_stack",
        TextSource::Plain(haskell_stack_text),
        IconSource::Cond(icon_haskell_stack),
    ),
    spec(
        "package",
        TextSource::Cwd(package_text),
        IconSource::Cond(icon_package),
    ),
    spec("asdf", TextSource::Cwd(asdf_text), IconSource::None),
    spec("fvm", TextSource::Cwd(fvm_text), IconSource::Cond(icon_fvm)),
    spec(
        "google_app_cred",
        TextSource::Plain(google_app_cred_text),
        IconSource::Key("gcloud"),
    ),
    spec(
        "aws_eb_env",
        TextSource::Plain(aws_eb_env_text),
        IconSource::Cond(icon_aws_eb_env),
    ),
    spec(
        "laravel_version",
        TextSource::Cwd(laravel_version_text),
        IconSource::Cond(icon_laravel_version),
    ),
    spec(
        "rspec_stats",
        TextSource::Cwd(rspec_stats_text),
        IconSource::Cond(icon_rspec_stats),
    ),
    spec(
        "todo",
        TextSource::Plain(todo_text),
        IconSource::Cond(icon_todo),
    ),
    spec(
        "taskwarrior",
        TextSource::Plain(taskwarrior_text),
        IconSource::Cond(icon_taskwarrior),
    ),
    spec(
        "dropbox",
        TextSource::Plain(dropbox_text),
        IconSource::Cond(icon_dropbox),
    ),
    spec(
        "proxy",
        TextSource::Plain(proxy_text),
        IconSource::Cond(icon_proxy),
    ),
    spec(
        "openfoam",
        TextSource::Plain(openfoam_text),
        IconSource::None,
    ),
    spec(
        "ranger",
        TextSource::Arg(level_text, "RANGER_LEVEL"),
        IconSource::ContentCond(icon_ranger),
    ),
    spec(
        "yazi",
        TextSource::Arg(level_text, "YAZI_LEVEL"),
        IconSource::ContentCond(icon_yazi),
    ),
    spec(
        "nnn",
        TextSource::Arg(level_text, "NNNLVL"),
        IconSource::ContentCond(icon_nnn),
    ),
    spec(
        "lf",
        TextSource::Arg(level_text, "LF_LEVEL"),
        IconSource::ContentCond(icon_lf),
    ),
    spec(
        "nix_shell",
        TextSource::Plain(nix_shell_text),
        IconSource::Cond(icon_nix_shell),
    ),
    spec(
        "vi_mode",
        TextSource::Segment(vi_mode_text),
        IconSource::None,
    ),
    spec(
        "docker_machine",
        TextSource::Env("DOCKER_MACHINE_NAME"),
        IconSource::Cond(icon_docker_machine),
    ),
    spec("history", TextSource::History, IconSource::None),
    spec(
        "background_jobs",
        TextSource::Jobs,
        IconSource::Key("background-jobs"),
    ),
    spec("ssh", TextSource::Empty, IconSource::ContentCond(icon_ssh)),
    spec(
        "xplr",
        TextSource::Empty,
        IconSource::ContentCond(icon_xplr),
    ),
    spec(
        "midnight_commander",
        TextSource::Empty,
        IconSource::ContentCond(icon_midnight_commander),
    ),
    spec(
        "vim_shell",
        TextSource::Empty,
        IconSource::ContentCond(icon_vim_shell),
    ),
    spec(
        "direnv",
        TextSource::Empty,
        IconSource::ContentCond(icon_direnv),
    ),
    spec(
        "chezmoi_shell",
        TextSource::Empty,
        IconSource::ContentCond(icon_chezmoi_shell),
    ),
];

/// Look up one segment row.
pub(super) fn find(name: &str) -> Option<&'static SegmentSpec> {
    SEGMENTS.iter().find(|s| s.name == name)
}

fn icon_toolbox() -> Option<&'static str> {
    (!toolbox_text().is_empty()).then_some("toolbox")
}

fn icon_dir_writable() -> Option<&'static str> {
    (!dir_writable_text(&current_dir()).is_empty()).then_some("lock")
}

fn icon_per_directory_history() -> Option<&'static str> {
    (!per_directory_history_text().is_empty()).then_some("history")
}

fn icon_haskell_stack() -> Option<&'static str> {
    (!haskell_stack_text().is_empty()).then_some("haskell")
}

fn icon_package() -> Option<&'static str> {
    (!package_text(&current_dir()).is_empty()).then_some("package")
}

fn icon_fvm() -> Option<&'static str> {
    (!fvm_text(&current_dir()).is_empty()).then_some("flutter")
}

fn icon_aws_eb_env() -> Option<&'static str> {
    (!aws_eb_env_text().is_empty()).then_some("aws-eb")
}

fn icon_laravel_version() -> Option<&'static str> {
    (!laravel_version_text(&current_dir()).is_empty()).then_some("laravel")
}

fn icon_rspec_stats() -> Option<&'static str> {
    (!rspec_stats_text(&current_dir()).is_empty()).then_some("test")
}

fn icon_todo() -> Option<&'static str> {
    (!todo_text().is_empty()).then_some("todo")
}

fn icon_taskwarrior() -> Option<&'static str> {
    (!taskwarrior_text().is_empty()).then_some("taskwarrior")
}

fn icon_dropbox() -> Option<&'static str> {
    (!dropbox_text().is_empty()).then_some("dropbox")
}

fn icon_ssh() -> Option<&'static str> {
    env().ssh.then_some("ssh")
}

fn icon_proxy() -> Option<&'static str> {
    env_has_proxy().then_some("proxy")
}

fn icon_docker_machine() -> Option<&'static str> {
    env_var("DOCKER_MACHINE_NAME").is_some().then_some("server")
}

fn icon_ranger() -> Option<&'static str> {
    level_icon("RANGER_LEVEL", "\u{f00b}")
        .is_some()
        .then_some("ranger")
}

fn icon_yazi() -> Option<&'static str> {
    level_icon("YAZI_LEVEL", "\u{f00b}")
        .is_some()
        .then_some("yazi")
}

fn icon_nnn() -> Option<&'static str> {
    level_icon("NNNLVL", "nnn").is_some().then_some("nnn")
}

fn icon_lf() -> Option<&'static str> {
    level_icon("LF_LEVEL", "lf").is_some().then_some("lf")
}

fn icon_nix_shell() -> Option<&'static str> {
    in_nix_shell().then_some("nix")
}

fn icon_xplr() -> Option<&'static str> {
    env_var("XPLR_PID").is_some().then_some("xplr")
}

fn icon_midnight_commander() -> Option<&'static str> {
    env_var("MC_TMPDIR").is_some().then_some("mc")
}

fn icon_vim_shell() -> Option<&'static str> {
    env_var("VIMRUNTIME").is_some().then_some("vim")
}

fn icon_direnv() -> Option<&'static str> {
    env_var("DIRENV_DIR").is_some().then_some("direnv")
}

fn icon_chezmoi_shell() -> Option<&'static str> {
    env_var("CHEZMOI").is_some().then_some("chezmoi")
}
