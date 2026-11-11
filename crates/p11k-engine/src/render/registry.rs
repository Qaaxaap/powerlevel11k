//! Segment registry: one table for every built-in segment.
//!
//! Each row is a name and a [`SegmentRender`] implementation. Delegating segments share
//! [`Delegated`]; the segments that need the renderer state (`dir`, `vcs`, `status`,
//! `prompt_char`, `command_execution_time`, `symfony2_tests`) implement the trait directly.
//! The renderer asks the row for its [`SegmentValue`]: text and icon decision come from one
//! evaluation, and no dispatcher branches on the segment name.

use super::env::*;
use super::segment::{Delegated, IconSource, SegmentRender, TextSource};
use super::segments::*;
use super::special::*;

/// One entry in the registry: the name, the implementation and whether the icon is the content
/// (a delegating segment with `IconSource::ContentCond`).
pub(super) struct SegmentMeta {
    pub name: &'static str,
    pub render: &'static dyn SegmentRender,
}

/// The registry: one row per segment. `find` scans it.
pub(super) static SEGMENTS: &[SegmentMeta] = &[
    SegmentMeta {
        name: "time",
        render: &Delegated {
            text: TextSource::Segment(time_text),
            icon: IconSource::Key("time"),
        },
    },
    SegmentMeta {
        name: "context",
        render: &Delegated {
            text: TextSource::Plain(context_text),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "user",
        render: &Delegated {
            text: TextSource::Plain(user_name),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "host",
        render: &Delegated {
            text: TextSource::Plain(host_text),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "root_indicator",
        render: &Delegated {
            text: TextSource::Plain(root_indicator_text),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "date",
        render: &Delegated {
            text: TextSource::Segment(date_text),
            icon: IconSource::Key("date"),
        },
    },
    SegmentMeta {
        name: "symfony2_version",
        render: &Delegated {
            text: TextSource::Cwd(symfony2_version_text),
            icon: IconSource::Key("symfony"),
        },
    },
    SegmentMeta {
        name: "go_version",
        render: &Delegated {
            text: TextSource::Plain(go_version),
            icon: IconSource::Key("go"),
        },
    },
    SegmentMeta {
        name: "rust_version",
        render: &Delegated {
            text: TextSource::Plain(rust_version),
            icon: IconSource::Key("rust"),
        },
    },
    SegmentMeta {
        name: "node_version",
        render: &Delegated {
            text: TextSource::Plain(node_version),
            icon: IconSource::Key("node"),
        },
    },
    SegmentMeta {
        name: "php_version",
        render: &Delegated {
            text: TextSource::Plain(php_version),
            icon: IconSource::Key("php"),
        },
    },
    SegmentMeta {
        name: "java_version",
        render: &Delegated {
            text: TextSource::Plain(java_version),
            icon: IconSource::Key("java"),
        },
    },
    SegmentMeta {
        name: "dotnet_version",
        render: &Delegated {
            text: TextSource::Plain(dotnet_version),
            icon: IconSource::Key("dotnet"),
        },
    },
    SegmentMeta {
        name: "swift_version",
        render: &Delegated {
            text: TextSource::Plain(swift_version),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "terraform_version",
        render: &Delegated {
            text: TextSource::Plain(terraform_version),
            icon: IconSource::Key("terraform"),
        },
    },
    SegmentMeta {
        name: "cpu_arch",
        render: &Delegated {
            text: TextSource::Plain(cpu_arch),
            icon: IconSource::Key("cpu-arch"),
        },
    },
    SegmentMeta {
        name: "virtualenv",
        render: &Delegated {
            text: TextSource::Cwd(virtualenv_text),
            icon: IconSource::Key("python"),
        },
    },
    SegmentMeta {
        name: "anaconda",
        render: &Delegated {
            text: TextSource::Cwd(anaconda_text),
            icon: IconSource::Key("python"),
        },
    },
    SegmentMeta {
        name: "pyenv",
        render: &Delegated {
            text: TextSource::Cwd(pyenv_text),
            icon: IconSource::Key("python"),
        },
    },
    SegmentMeta {
        name: "nodeenv",
        render: &Delegated {
            text: TextSource::Cwd(nodeenv_text),
            icon: IconSource::Key("node"),
        },
    },
    SegmentMeta {
        name: "nodenv",
        render: &Delegated {
            text: TextSource::Cwd(nodenv_text),
            icon: IconSource::Key("node"),
        },
    },
    SegmentMeta {
        name: "nvm",
        render: &Delegated {
            text: TextSource::Cwd(nvm_text),
            icon: IconSource::Key("node"),
        },
    },
    SegmentMeta {
        name: "rbenv",
        render: &Delegated {
            text: TextSource::Cwd(rbenv_text),
            icon: IconSource::Key("ruby"),
        },
    },
    SegmentMeta {
        name: "chruby",
        render: &Delegated {
            text: TextSource::Cwd(chruby_text),
            icon: IconSource::Key("ruby"),
        },
    },
    SegmentMeta {
        name: "rvm",
        render: &Delegated {
            text: TextSource::Cwd(rvm_text),
            icon: IconSource::Key("ruby"),
        },
    },
    SegmentMeta {
        name: "goenv",
        render: &Delegated {
            text: TextSource::Cwd(goenv_text),
            icon: IconSource::Key("go"),
        },
    },
    SegmentMeta {
        name: "jenv",
        render: &Delegated {
            text: TextSource::Cwd(jenv_text),
            icon: IconSource::Key("java"),
        },
    },
    SegmentMeta {
        name: "phpenv",
        render: &Delegated {
            text: TextSource::Cwd(phpenv_text),
            icon: IconSource::Key("php"),
        },
    },
    SegmentMeta {
        name: "luaenv",
        render: &Delegated {
            text: TextSource::Cwd(luaenv_text),
            icon: IconSource::Key("lua"),
        },
    },
    SegmentMeta {
        name: "plenv",
        render: &Delegated {
            text: TextSource::Cwd(plenv_text),
            icon: IconSource::Key("perl"),
        },
    },
    SegmentMeta {
        name: "scalaenv",
        render: &Delegated {
            text: TextSource::Cwd(scalaenv_text),
            icon: IconSource::Key("scala"),
        },
    },
    SegmentMeta {
        name: "perlbrew",
        render: &Delegated {
            text: TextSource::Cwd(perlbrew_text),
            icon: IconSource::Key("perl"),
        },
    },
    SegmentMeta {
        name: "load",
        render: &Delegated {
            text: TextSource::Plain(load),
            icon: IconSource::Key("load"),
        },
    },
    SegmentMeta {
        name: "ram",
        render: &Delegated {
            text: TextSource::Plain(ram),
            icon: IconSource::Key("ram"),
        },
    },
    SegmentMeta {
        name: "swap",
        render: &Delegated {
            text: TextSource::Plain(swap),
            icon: IconSource::Key("swap"),
        },
    },
    SegmentMeta {
        name: "disk_usage",
        render: &Delegated {
            text: TextSource::Cwd(disk_usage),
            icon: IconSource::Key("disk"),
        },
    },
    SegmentMeta {
        name: "battery",
        render: &Delegated {
            text: TextSource::Plain(battery),
            icon: IconSource::Key("battery"),
        },
    },
    SegmentMeta {
        name: "aws",
        render: &Delegated {
            text: TextSource::Plain(aws_text),
            icon: IconSource::Key("aws"),
        },
    },
    SegmentMeta {
        name: "azure",
        render: &Delegated {
            text: TextSource::Plain(azure_text),
            icon: IconSource::Key("azure"),
        },
    },
    SegmentMeta {
        name: "gcloud",
        render: &Delegated {
            text: TextSource::Plain(gcloud_text),
            icon: IconSource::Key("gcloud"),
        },
    },
    SegmentMeta {
        name: "kubecontext",
        render: &Delegated {
            text: TextSource::Cwd(kubecontext_text),
            icon: IconSource::Key("kube"),
        },
    },
    SegmentMeta {
        name: "terraform",
        render: &Delegated {
            text: TextSource::Cwd(terraform_text),
            icon: IconSource::Key("terraform"),
        },
    },
    SegmentMeta {
        name: "ip",
        render: &Delegated {
            text: TextSource::Plain(ip_text),
            icon: IconSource::Key("network"),
        },
    },
    SegmentMeta {
        name: "vpn_ip",
        render: &Delegated {
            text: TextSource::Plain(vpn_ip_text),
            icon: IconSource::Key("vpn"),
        },
    },
    SegmentMeta {
        name: "wifi",
        render: &Delegated {
            text: TextSource::Plain(wifi_text),
            icon: IconSource::Key("wifi"),
        },
    },
    SegmentMeta {
        name: "public_ip",
        render: &Delegated {
            text: TextSource::Plain(public_ip_text),
            icon: IconSource::Key("public-ip"),
        },
    },
    SegmentMeta {
        name: "detect_virt",
        render: &Delegated {
            text: TextSource::Plain(detect_virt),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "toolbox",
        render: &Delegated {
            text: TextSource::Plain(toolbox_text),
            icon: IconSource::Key("toolbox"),
        },
    },
    SegmentMeta {
        name: "dir_writable",
        render: &Delegated {
            text: TextSource::Cwd(dir_writable_text),
            icon: IconSource::Key("lock"),
        },
    },
    SegmentMeta {
        name: "per_directory_history",
        render: &Delegated {
            text: TextSource::Plain(per_directory_history_text),
            icon: IconSource::Key("history"),
        },
    },
    SegmentMeta {
        name: "haskell_stack",
        render: &Delegated {
            text: TextSource::Plain(haskell_stack_text),
            icon: IconSource::Key("haskell"),
        },
    },
    SegmentMeta {
        name: "package",
        render: &Delegated {
            text: TextSource::Cwd(package_text),
            icon: IconSource::Key("package"),
        },
    },
    SegmentMeta {
        name: "asdf",
        render: &Delegated {
            text: TextSource::Cwd(asdf_text),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "fvm",
        render: &Delegated {
            text: TextSource::Cwd(fvm_text),
            icon: IconSource::Key("flutter"),
        },
    },
    SegmentMeta {
        name: "google_app_cred",
        render: &Delegated {
            text: TextSource::Plain(google_app_cred_text),
            icon: IconSource::Key("gcloud"),
        },
    },
    SegmentMeta {
        name: "aws_eb_env",
        render: &Delegated {
            text: TextSource::Plain(aws_eb_env_text),
            icon: IconSource::Key("aws-eb"),
        },
    },
    SegmentMeta {
        name: "laravel_version",
        render: &Delegated {
            text: TextSource::Cwd(laravel_version_text),
            icon: IconSource::Key("laravel"),
        },
    },
    SegmentMeta {
        name: "rspec_stats",
        render: &Delegated {
            text: TextSource::Cwd(rspec_stats_text),
            icon: IconSource::Key("test"),
        },
    },
    SegmentMeta {
        name: "todo",
        render: &Delegated {
            text: TextSource::Plain(todo_text),
            icon: IconSource::Key("todo"),
        },
    },
    SegmentMeta {
        name: "taskwarrior",
        render: &Delegated {
            text: TextSource::Plain(taskwarrior_text),
            icon: IconSource::Key("taskwarrior"),
        },
    },
    SegmentMeta {
        name: "dropbox",
        render: &Delegated {
            text: TextSource::Plain(dropbox_text),
            icon: IconSource::Key("dropbox"),
        },
    },
    SegmentMeta {
        name: "proxy",
        render: &Delegated {
            text: TextSource::Plain(proxy_text),
            icon: IconSource::Key("proxy"),
        },
    },
    SegmentMeta {
        name: "openfoam",
        render: &Delegated {
            text: TextSource::Plain(openfoam_text),
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "ranger",
        render: &Delegated {
            text: TextSource::Arg(level_text, "RANGER_LEVEL"),
            icon: IconSource::ContentCond(icon_ranger),
        },
    },
    SegmentMeta {
        name: "yazi",
        render: &Delegated {
            text: TextSource::Arg(level_text, "YAZI_LEVEL"),
            icon: IconSource::ContentCond(icon_yazi),
        },
    },
    SegmentMeta {
        name: "nnn",
        render: &Delegated {
            text: TextSource::Arg(level_text, "NNNLVL"),
            icon: IconSource::ContentCond(icon_nnn),
        },
    },
    SegmentMeta {
        name: "lf",
        render: &Delegated {
            text: TextSource::Arg(level_text, "LF_LEVEL"),
            icon: IconSource::ContentCond(icon_lf),
        },
    },
    SegmentMeta {
        name: "nix_shell",
        render: &Delegated {
            text: TextSource::Plain(nix_shell_text),
            icon: IconSource::Key("nix"),
        },
    },
    SegmentMeta {
        name: "vi_mode",
        render: &Delegated {
            text: TextSource::ViMode,
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "docker_machine",
        render: &Delegated {
            text: TextSource::Env("DOCKER_MACHINE_NAME"),
            icon: IconSource::Key("server"),
        },
    },
    SegmentMeta {
        name: "history",
        render: &Delegated {
            text: TextSource::History,
            icon: IconSource::None,
        },
    },
    SegmentMeta {
        name: "background_jobs",
        render: &Delegated {
            text: TextSource::Jobs,
            icon: IconSource::Key("background-jobs"),
        },
    },
    SegmentMeta {
        name: "ssh",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_ssh),
        },
    },
    SegmentMeta {
        name: "xplr",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_xplr),
        },
    },
    SegmentMeta {
        name: "midnight_commander",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_midnight_commander),
        },
    },
    SegmentMeta {
        name: "vim_shell",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_vim_shell),
        },
    },
    SegmentMeta {
        name: "direnv",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_direnv),
        },
    },
    SegmentMeta {
        name: "chezmoi_shell",
        render: &Delegated {
            text: TextSource::Empty,
            icon: IconSource::ContentCond(icon_chezmoi_shell),
        },
    },
    SegmentMeta {
        name: "dir",
        render: &DirSegment,
    },
    SegmentMeta {
        name: "vcs",
        render: &VcsSegment,
    },
    SegmentMeta {
        name: "status",
        render: &StatusSegment,
    },
    SegmentMeta {
        name: "prompt_char",
        render: &PromptCharSegment,
    },
    SegmentMeta {
        name: "command_execution_time",
        render: &ExecTimeSegment,
    },
    SegmentMeta {
        name: "symfony2_tests",
        render: &Symfony2TestsSegment,
    },
    SegmentMeta {
        name: "os",
        render: &OsSegment,
    },
    SegmentMeta {
        name: "os_icon",
        render: &OsSegment,
    },
];

/// Look up one segment row.
pub(super) fn find(name: &str) -> Option<&'static SegmentMeta> {
    SEGMENTS.iter().find(|s| s.name == name)
}

fn icon_ssh() -> Option<&'static str> {
    env().ssh.then_some("ssh")
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
