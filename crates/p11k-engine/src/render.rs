//! 渲染器：把 KDL 配置驱动成多行 header 文本。
//!
//! - **几何**：按 [`Config::layout`] 的行结构，逐行「左段串 + 右段右对齐」。
//! - **段**：内置段（dir/vcs/status/time/command_execution_time/background_jobs/
//!   prompt_char/os/text）按配置渲染，可叠加左/中/右附加文字。
//! - **色**：配置的 `fg`/`bg`（三段回退）+ 段间 powerline 分隔符与背景块。
//!
//! 输入：配置 + 当前状态。输出：header 的 ANSI 字符串。

use std::fmt::Write as _;

use crate::config::{AttachText, Color, Config, Element, Style};
use crate::theme::{GitStatus, HeaderInfo};
use std::cell::RefCell;
use std::collections::HashMap;
use unicode_width::UnicodeWidthChar;

/// 输入行前缀:`frame.last_prefix` + `prompt_char` 内容;`width` 是它的显示宽度,
/// 引擎据此生成等宽占位符。
pub struct InputPrefix {
    pub text: String,
    pub width: usize,
}

/// prompt_char 的 state:退出码非 0 → ERROR;0 或无 → 正常态(None)。
fn prompt_state(exit_code: Option<i32>) -> Option<&'static str> {
    match exit_code {
        Some(0) | None => None,
        Some(_) => Some("ERROR"),
    }
}

/// 某 state 下的输入行前缀文本(帧 last_prefix + prompt_char 字符 + 空格)。
fn prefix_text(config: &Config, state: Option<&str>) -> String {
    let mut text = String::new();
    if !config.frame.last_prefix.text.is_empty() {
        let piece = &config.frame.last_prefix;
        text.push_str(&paint(&piece.text, &config.frame_piece_style(piece)));
    }
    let pc = config.segment("prompt_char");
    let st = pc.effective_style(state, &config.defaults);
    text.push_str(&paint(pc.char_for(state, "❯"), &st));
    text.push(' '); // 前缀后空格(无色),对齐原 `❯ ` 几何
    text
}

/// 某 state 下前缀的显示宽度(去 ANSI)。
fn prefix_width(config: &Config, state: Option<&str>) -> usize {
    display_width(&prefix_text(config, state))
}

/// 计算输入行前缀(如 `╰─❯`)。`last_prefix`(帧样式)/提示符字符(prompt_char 样式,
/// 可按 `exit_code` 进 ERROR state)已上色;`width` 为去 ANSI 显示宽。
pub fn input_prefix(config: &Config, exit_code: Option<i32>) -> InputPrefix {
    let state = prompt_state(exit_code);
    let text = prefix_text(config, state);
    let width = display_width(&text);
    InputPrefix { text, width }
}

/// 校验 prompt_char:凡配了 char 的 state(如 ERROR)必须与正常态等宽。
/// 提示符宽度在启动期就得确定(占位符协议按它生成),不等宽会破坏几何,
/// 由引擎启动期报错处理。
pub fn check_prompt_char_widths(config: &Config) -> Result<(), String> {
    let pc = config.segment("prompt_char");
    let base = prefix_width(config, None);
    for (name, spec) in &pc.states {
        if spec.char.is_some() {
            let w = prefix_width(config, Some(name));
            if w != base {
                return Err(format!(
                    "prompt_char state `{name}` 的提示符宽度({w})与正常态({base})不一致,须等宽"
                ));
            }
        }
    }
    Ok(())
}

/// 一段渲染结果:文本 + 它的样式(渲染期才装配 ANSI)。
struct SegmentText {
    text: String,
    style: Style,
}

/// 渲染 header 为**逐行内容**(每行=左段串+右段右对齐,不含光标/清屏/换行)。
/// 供 theme 层逐行 `\r\e[K` + 内容 + `\r\n` 画到终端。
pub fn render_header_lines(
    config: &Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    cols: usize,
) -> Vec<String> {
    let layout = &config.layout;
    let left = &layout.left;
    let right = &layout.right;
    let lines = left.len().max(right.len());
    let frame = &config.frame;
    (0..lines)
        .map(|i| {
            let l = left
                .get(i)
                .map(|seg| render_row(config, seg, info, vcs, false))
                .unwrap_or_default();
            let r = right
                .get(i)
                .map(|seg| render_row(config, seg, info, vcs, true))
                .unwrap_or_default();
            // 每行帧:首行 first,其余 header 行 newline。
            let (prefix, suffix) = if i == 0 {
                (&frame.first_prefix, &frame.first_suffix)
            } else {
                (&frame.newline_prefix, &frame.newline_suffix)
            };
            let pre_w = display_width(&prefix.text);
            let suf_w = display_width(&suffix.text);
            let mut row = String::new();
            if !prefix.text.is_empty() {
                row.push_str(&paint(&prefix.text, &config.frame_piece_style(prefix)));
            }
            // 右对齐预算 = cols - 前缀宽 - 后缀宽(否则帧把行撑宽、后缀挤到下一行)。
            let body = assemble_row(
                &l,
                &r,
                cols.saturating_sub(pre_w + suf_w),
                &config.separators,
            );
            row.push_str(&body);
            if !suffix.text.is_empty() {
                row.push_str(&paint(&suffix.text, &config.frame_piece_style(suffix)));
            }
            row
        })
        .collect()
}

/// 渲染一行:左段串、右段右对齐。
fn render_row(
    config: &Config,
    elements: &[Element],
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    right: bool,
) -> Vec<SegmentText> {
    elements
        .iter()
        .map(|el| match el {
            Element::Seg(name) => render_segment(config, name, info, vcs, right),
            Element::Joined(name) => render_segment(config, name, info, vcs, right),
            Element::Text(t) => {
                let mut style = config
                    .segment("text")
                    .effective_style(None, &config.defaults);
                if style.bg == Color::Default {
                    style.bg = if config.defaults.bg != Color::Default {
                        config.defaults.bg.clone()
                    } else {
                        Color::Xterm(0)
                    };
                }
                SegmentText {
                    text: paint(&expand_env(t), &style),
                    style,
                }
            }
        })
        .collect()
}

/// 渲染单个段。返回的 `text` 已是**上色后的 ANSI**;`style` 供段间分隔符/块背景。
fn render_segment(
    config: &Config,
    name: &str,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
    right: bool,
) -> SegmentText {
    let seg = config.segment(name);
    let mut style = seg.effective_style(None, &config.defaults);
    // 保证段都有背景(哪怕默认色):无显式 bg → defaults.bg → 内置默认背景。
    if style.bg == Color::Default {
        style.bg = if config.defaults.bg != Color::Default {
            config.defaults.bg.clone()
        } else {
            Color::Xterm(0) // 内置默认背景(黑)
        };
    }
    let text = match name {
        "dir" => dir_seg_text(config, info, seg, &style),
        "vcs" => {
            if seg.content.is_some() {
                paint(&value_of(seg.content.as_deref(), vcs_text(vcs)), &style)
            } else {
                paint(&vcs_text(vcs), &style)
            }
        }
        "status" => paint(&status_text(info), &style),
        "prompt_char" => {
            // 提示符字符随退出码进 ERROR state(char/样式都可按态配)。
            let state = prompt_state(info.exit_code);
            style = seg.effective_style(state, &config.defaults);
            paint(seg.char_for(state, "❯"), &style)
        }
        "time" => paint(&now_hhmmss(), &style),
        "command_execution_time" => {
            let threshold = match seg.prop("threshold-seconds") {
                Some(crate::config::Prop::Int(n)) => (*n as f64).max(0.0),
                _ => 3.0,
            };
            if info.exec_seconds >= threshold {
                paint(&format_duration(info.exec_seconds), &style)
            } else {
                paint("", &style)
            }
        }
        "background_jobs" => {
            if info.jobs > 0 {
                paint(&info.jobs.to_string(), &style)
            } else {
                paint("", &style)
            }
        }
        "context" => paint(&context_text(), &style),
        "user" => paint(&user_name(), &style),
        "host" => paint(&host_text(), &style),
        "root_indicator" => paint(&root_indicator_text(), &style),
        "date" => paint(&date_text(seg), &style),
        // 工具链版本段:跑 `cmd --version` 解析版本,有命令才显示。
        "go_version" => paint(&go_version(), &style),
        "rust_version" => paint(&rust_version(), &style),
        "node_version" => paint(&node_version(), &style),
        "php_version" => paint(&php_version(), &style),
        "java_version" => paint(&java_version(), &style),
        "dotnet_version" => paint(&dotnet_version(), &style),
        "swift_version" => paint(&swift_version(), &style),
        "terraform_version" => paint(&terraform_version(), &style),
        "cpu_arch" => paint(&cpu_arch(), &style),
        // 环境管理器/*env 家族:读环境变量或祖先版本文件,激活才显示。
        "virtualenv" => paint(&virtualenv_text(&info.cwd), &style),
        "anaconda" => paint(&anaconda_text(&info.cwd), &style),
        "pyenv" => paint(&pyenv_text(&info.cwd), &style),
        "nodeenv" => paint(&nodeenv_text(&info.cwd), &style),
        "nodenv" => paint(&nodenv_text(&info.cwd), &style),
        "nvm" => paint(&nvm_text(&info.cwd), &style),
        "rbenv" => paint(&rbenv_text(&info.cwd), &style),
        "chruby" => paint(&chruby_text(&info.cwd), &style),
        "rvm" => paint(&rvm_text(&info.cwd), &style),
        "goenv" => paint(&goenv_text(&info.cwd), &style),
        "jenv" => paint(&jenv_text(&info.cwd), &style),
        "phpenv" => paint(&phpenv_text(&info.cwd), &style),
        "luaenv" => paint(&luaenv_text(&info.cwd), &style),
        "plenv" => paint(&plenv_text(&info.cwd), &style),
        "scalaenv" => paint(&scalaenv_text(&info.cwd), &style),
        "perlbrew" => paint(&perlbrew_text(&info.cwd), &style),
        // 系统资源段:读 /proc /sys 或 df,数据源缺失则隐藏。
        "load" => paint(&load(), &style),
        "ram" => paint(&ram(), &style),
        "swap" => paint(&swap(), &style),
        "disk_usage" => paint(&disk_usage(&info.cwd), &style),
        "battery" => paint(&battery(), &style),
        // 云/k8s 段:读云配置或环境变量,配置缺失则隐藏。
        "aws" => paint(&aws_text(), &style),
        "azure" => paint(&azure_text(), &style),
        "gcloud" => paint(&gcloud_text(), &style),
        "kubecontext" => paint(&kubecontext_text(&info.cwd), &style),
        "terraform" => paint(&terraform_text(&info.cwd), &style),
        // 网络段:本机 IP/VPN/WiFi(同步),公网 IP(异步查询缓存)。
        "ip" => paint(&ip_text(), &style),
        "vpn_ip" => paint(&vpn_ip_text(), &style),
        "wifi" => paint(&wifi_text(), &style),
        "public_ip" => paint(&public_ip_text(), &style),
        // 补齐:虚拟化/容器/目录权限/history 作用域/语言栈/包管理器。
        "detect_virt" => paint(&detect_virt(), &style),
        "toolbox" => paint(&toolbox_text(), &style),
        "dir_writable" => paint(&dir_writable_text(&info.cwd), &style),
        "per_directory_history" => paint(&per_directory_history_text(), &style),
        "haskell_stack" => paint(&haskell_stack_text(), &style),
        "package" => paint(&package_text(&info.cwd), &style),
        "asdf" => paint(&asdf_text(&info.cwd), &style),
        "fvm" => paint(&fvm_text(&info.cwd), &style),
        // 云子类/框架/待办:命令存在或文件命中才显示。
        "google_app_cred" => paint(&google_app_cred_text(), &style),
        "aws_eb_env" => paint(&aws_eb_env_text(), &style),
        "laravel_version" => paint(&laravel_version_text(&info.cwd), &style),
        "rspec_stats" => paint(&rspec_stats_text(&info.cwd), &style),
        "todo" => paint(&todo_text(), &style),
        "taskwarrior" => paint(&taskwarrior_text(), &style),
        "dropbox" => paint(&dropbox_text(), &style),
        // 环境指示段:内容来自环境变量,条件不满足 → 空文本 + 空图标(default_icon 按条件给),
        // 整段隐藏。
        "ssh" | "xplr" | "midnight_commander" | "vim_shell" | "direnv" | "chezmoi_shell" => {
            paint("", &style)
        }
        "proxy" => paint(&proxy_text(), &style),
        "docker_machine" => paint(&env_var("DOCKER_MACHINE_NAME").unwrap_or_default(), &style),
        "openfoam" => paint(&openfoam_text(), &style),
        "ranger" => paint(&level_text("RANGER_LEVEL"), &style),
        "yazi" => paint(&level_text("YAZI_LEVEL"), &style),
        "nnn" => paint(&level_text("NNNLVL"), &style),
        "lf" => paint(&level_text("LF_LEVEL"), &style),
        "nix_shell" => paint(&nix_shell_text(), &style),
        _ => paint(&value_of(seg.content.as_deref(), String::new()), &style),
    };
    // 段图标:配置 `icon` 优先,否则按段名内置默认;vcs 段按
    // 远端域名选图标(如 github 、aur )。图标 + 空格前缀,用段样式上色。
    let icon = seg.icon.clone().filter(|i| !i.is_empty()).or_else(|| {
        if name == "vcs" {
            vcs.as_ref().map(|v| vcs_remote_icon(config, &v.remote_url))
        } else {
            default_icon(name)
        }
    });
    let icon_text = match icon {
        Some(ic) => paint(&format!("{ic} "), &style),
        None => String::new(),
    };
    // 附加文字槽:左(icon 前)/中(icon 与内容之间,须二者都有)/右(内容后)。
    // 仅拼接显示、不独立成块;fg 缺省跟段走。text-middle 始终紧随 icon 与内容
    // 之间:左列 icon 在前 → middle 在 icon 后;右列 icon 后置 → middle 在内容与 icon 之间。
    let mut out = String::new();
    if let Some(l) = &seg.text_left {
        out.push_str(&paint_attach(&style, l));
    }
    let middle = if !icon_text.is_empty() && !text.is_empty() {
        seg.text_middle
            .as_ref()
            .map(|m| paint_attach(&style, m))
            .unwrap_or_default()
    } else {
        String::new()
    };
    if right {
        out.push_str(&text);
        out.push_str(&middle);
        out.push_str(&icon_text);
    } else {
        out.push_str(&icon_text);
        out.push_str(&middle);
        out.push_str(&text);
    }
    if let Some(r) = &seg.text_right {
        out.push_str(&paint_attach(&style, r));
    }
    SegmentText { text: out, style }
}

/// 附加文字上色:fg 缺省沿用段样式,配了则覆盖前景(bg/bold 仍跟段)。
fn paint_attach(style: &Style, a: &AttachText) -> String {
    let mut st = style.clone();
    if let Some(fg) = &a.fg {
        st.fg = fg.clone();
    }
    paint(&a.text, &st)
}

/// vcs 图标按远端域名选择(配置 `vcs-remote-icons` 按序子串匹配;未命中默认 git )。
fn vcs_remote_icon(config: &Config, remote_url: &str) -> String {
    for (domain, icon) in &config.vcs_remote_icons {
        if !domain.is_empty() && remote_url.contains(domain) {
            return icon.clone();
        }
    }
    "\u{f1d3}".to_string() //  默认 git
}

/// 内置段图标(无配置 `icon` 时的默认;nerd font)。os 段按发行版动态。
fn default_icon(name: &str) -> Option<String> {
    match name {
        "os" | "os_icon" => Some(os_icon()),
        "dir" => Some("\u{f07c}".into()),               // 
        "vcs" => Some("\u{f1d3}".into()),               // 
        "time" => Some("\u{f017}".into()),              // 
        "background_jobs" => Some("\u{f013}".into()),   // 齿轮 
        "date" => Some("\u{f073}".into()),              // 日历 
        "go_version" => Some("\u{e626}".into()),        // Go
        "rust_version" => Some("\u{e7a8}".into()),      // Rust
        "node_version" => Some("\u{e617}".into()),      // Node
        "php_version" => Some("\u{e608}".into()),       // PHP
        "java_version" => Some("\u{e738}".into()),      // Java
        "dotnet_version" => Some("\u{e77f}".into()),    // .NET
        "terraform_version" => Some("\u{f1bb}".into()), // Terraform
        "cpu_arch" => Some("\u{e266}".into()),          // 芯片
        "virtualenv" | "anaconda" | "pyenv" => Some("\u{e73c}".into()), // Python
        "nodeenv" | "nodenv" | "nvm" => Some("\u{e617}".into()), // Node
        "rbenv" | "chruby" | "rvm" => Some("\u{f219}".into()), // Ruby
        "goenv" => Some("\u{e626}".into()),             // Go
        "jenv" => Some("\u{e738}".into()),              // Java
        "phpenv" => Some("\u{e608}".into()),            // PHP
        "luaenv" => Some("\u{e620}".into()),            // Lua
        "plenv" | "perlbrew" => Some("\u{e769}".into()), // Perl
        "scalaenv" => Some("\u{e737}".into()),          // Scala
        "load" => Some("\u{f080}".into()),              // 负载
        "ram" => Some("\u{f0e4}".into()),               // 内存
        "swap" => Some("\u{f464}".into()),              // swap
        "battery" => Some("\u{f240}".into()),           // 电池
        "disk_usage" => Some("\u{f0a0}".into()),        // 磁盘
        "aws" => Some("\u{f270}".into()),               // AWS
        "azure" => Some("\u{fd03}".into()),             // Azure
        "gcloud" => Some("\u{f7b7}".into()),            // GCloud
        "kubecontext" => Some("\u{2388}".into()),       // ⎈ k8s
        "terraform" => Some("\u{f1bb}".into()),         // Terraform
        "ip" => Some("\u{f50d}".into()),                // 网络
        "vpn_ip" => Some("\u{f023}".into()),            // VPN
        "wifi" => Some("\u{f1eb}".into()),              // WiFi
        "public_ip" => Some("\u{f0ac}".into()),         // 公网
        "toolbox" => (!toolbox_text().is_empty()).then(|| "\u{e20f}".into()), // 容器
        "dir_writable" => {
            (!dir_writable_text(&current_dir()).is_empty()).then(|| "\u{f023}".into())
        }
        "per_directory_history" => {
            (!per_directory_history_text().is_empty()).then(|| "\u{f1da}".into())
        }
        "haskell_stack" => (!haskell_stack_text().is_empty()).then(|| "\u{e61f}".into()),
        "package" => (!package_text(&current_dir()).is_empty()).then(|| "\u{f8d6}".into()),
        "fvm" => (!fvm_text(&current_dir()).is_empty()).then(|| "F".into()), // Flutter
        "google_app_cred" => (!google_app_cred_text().is_empty()).then(|| "\u{f7b7}".into()),
        "aws_eb_env" => (!aws_eb_env_text().is_empty()).then(|| "\u{f1bd}".into()),
        "laravel_version" => {
            (!laravel_version_text(&current_dir()).is_empty()).then(|| "\u{e73f}".into())
        }
        "rspec_stats" => (!rspec_stats_text(&current_dir()).is_empty()).then(|| "\u{f188}".into()),
        "todo" => (!todo_text().is_empty()).then(|| "\u{2611}".into()),
        "taskwarrior" => (!taskwarrior_text().is_empty()).then(|| "\u{f4a0}".into()),
        "dropbox" => (!dropbox_text().is_empty()).then(|| "\u{f16b}".into()),
        // 环境指示段:图标同样按条件给,条件不满足 → None,配合空文本整段隐藏。
        "ssh" => env().ssh.then(|| "\u{f489}".into()), // SSH 会话
        "proxy" => env_has_proxy().then(|| "\u{2194}".into()), // ↔
        "docker_machine" => env_var("DOCKER_MACHINE_NAME").map(|_| "\u{f0ae}".into()), // 服务器
        "ranger" => level_icon("RANGER_LEVEL", "\u{f00b}"), // 文件列表
        "yazi" => level_icon("YAZI_LEVEL", "\u{f00b}"),
        "nnn" => level_icon("NNNLVL", "nnn"),
        "lf" => level_icon("LF_LEVEL", "lf"),
        "nix_shell" => in_nix_shell().then(|| "\u{f313}".into()), // 雪花
        "xplr" => env_var("XPLR_PID").map(|_| "xplr".into()),
        "midnight_commander" => env_var("MC_TMPDIR").map(|_| "mc".into()),
        "vim_shell" => env_var("VIMRUNTIME").map(|_| "\u{e62b}".into()), // vim
        "direnv" => env_var("DIRENV_DIR").map(|_| "\u{25bc}".into()),    // ▼
        "chezmoi_shell" => env_var("CHEZMOI").map(|_| "\u{f015}".into()), // 家
        _ => None,
    }
}

thread_local! {
    static OS_ICON: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// os 图标:uname 大类 + /etc/os-release ID 匹配发行版(对齐 p10k `_p9k_set_os`)。
fn os_icon() -> String {
    OS_ICON.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(detect_os_icon());
        }
        c.borrow().as_ref().unwrap().clone()
    })
}

#[derive(Clone)]
struct EnvInfo {
    user: String,
    host: String,
    ssh: bool,
    root: bool,
}

thread_local! {
    static ENV: RefCell<Option<EnvInfo>> = const { RefCell::new(None) };
}

fn env() -> EnvInfo {
    ENV.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(detect_env());
        }
        c.borrow().as_ref().unwrap().clone()
    })
}

fn detect_env() -> EnvInfo {
    let user = std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("LOGNAME").ok())
        .unwrap_or_default();
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("HOST").ok())
        .unwrap_or_default();
    let ssh = ["SSH_CONNECTION", "SSH_TTY", "SSH_CLIENT"]
        .iter()
        .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()));
    let root = unsafe { libc::geteuid() } == 0;
    EnvInfo {
        user,
        host,
        ssh,
        root,
    }
}

fn user_name() -> String {
    env().user
}

/// context：SSH → `user@host`；本地 root → `user`；本地普通用户 → 空（隐藏）。
fn context_text() -> String {
    let e = env();
    if e.ssh {
        format!("{}@{}", e.user, e.host)
    } else if e.root {
        e.user
    } else {
        String::new()
    }
}

fn host_text() -> String {
    let e = env();
    if e.ssh || e.root {
        e.host
    } else {
        String::new()
    }
}

fn root_indicator_text() -> String {
    if env().root {
        "#".into()
    } else {
        String::new()
    }
}

/// 读环境变量,缺失或空 → None。
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// 层级段(ranger/nnf/lf/yazi):值为 0 或空 → 空(不在该程序里)。
fn level_text(name: &str) -> String {
    env_var(name).filter(|v| v != "0").unwrap_or_default()
}

fn level_icon(name: &str, icon: &str) -> Option<String> {
    env_var(name).filter(|v| v != "0").map(|_| icon.into())
}

fn env_has_proxy() -> bool {
    ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
        .any(|k| env_var(k).is_some())
}

/// proxy 段:取第一个非空代理的 host:port(去 scheme/user@/路径)。
fn proxy_text() -> String {
    for key in ["all_proxy", "http_proxy", "https_proxy", "ftp_proxy"]
        .iter()
        .chain(["ALL_PROXY", "HTTP_PROXY", "HTTPS_PROXY", "FTP_PROXY"].iter())
    {
        let Some(v) = env_var(key) else { continue };
        let v = v.split_once("://").map(|(_, rest)| rest).unwrap_or(&v);
        let v = v.rsplit('@').next().unwrap_or(v);
        let v = v.split(['/', '?']).next().unwrap_or(v);
        if !v.is_empty() {
            return v.to_string();
        }
    }
    String::new()
}

/// openfoam 段:p10k 显示 `OF: <版本>`。
fn openfoam_text() -> String {
    env_var("WM_PROJECT_VERSION")
        .map(|v| format!("OF: {v}"))
        .unwrap_or_default()
}

// nix_shell:对齐 p10k,只认 `IN_NIX_SHELL` 的 pure/impure,其它值视为未激活。
fn nix_shell_text() -> String {
    env_var("IN_NIX_SHELL")
        .filter(|v| v == "pure" || v == "impure")
        .unwrap_or_default()
}

fn in_nix_shell() -> bool {
    env_var("IN_NIX_SHELL").is_some_and(|v| v == "pure" || v == "impure")
}

// 系统资源段:读 /proc 与 /sys(battery),或跑 `df`。数据源缺失则隐藏。
fn human_bytes(bytes: u64) -> String {
    let mut val = bytes as f64;
    let mut i = 0;
    const UNITS: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    while val >= 1024.0 && i < UNITS.len() - 1 {
        val /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{val}B")
    } else {
        format!("{val:.1}{}", UNITS[i])
    }
}

/// 读 /proc/meminfo 某字段(KiB)。
fn meminfo_kb(name: &str) -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()?
        .lines()
        .find_map(|l| {
            l.strip_prefix(&format!("{name}:"))
                .and_then(|rest| rest.split_whitespace().next()?.parse().ok())
        })
}

/// 负载:当前系统负载(/proc/loadavg 第一个值)。
fn load() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()))
        .unwrap_or_default()
}

/// 空闲内存(MemAvailable,人类可读)。
fn ram() -> String {
    meminfo_kb("MemAvailable")
        .map(|kb| human_bytes(kb * 1024))
        .unwrap_or_default()
}

/// 已用 swap。
fn swap() -> String {
    let total = meminfo_kb("SwapTotal").unwrap_or(0);
    let free = meminfo_kb("SwapFree").unwrap_or(0);
    let used = total.saturating_sub(free);
    if used == 0 {
        String::new()
    } else {
        human_bytes(used * 1024)
    }
}

/// 当前目录所在分区的已用百分比。
fn disk_usage(cwd: &str) -> String {
    run_cmd("df", &["-P", cwd])
        .and_then(|s| {
            s.lines()
                .nth(1)
                .and_then(|l| l.split_whitespace().nth(4))
                .map(|v| v.trim_end_matches('%').to_string())
        })
        .unwrap_or_default()
}

/// 电池:电量百分比 + 状态(如 `Charging 87%`),无电池文件则隐藏。
fn battery() -> String {
    let bat = std::fs::read_dir("/sys/class/power_supply")
        .ok()
        .and_then(|rd| {
            rd.flatten()
                .find(|e| e.file_name().to_string_lossy().starts_with("BAT"))
        });
    let Some(bat) = bat else { return String::new() };
    let bat = bat.path();
    let cap = std::fs::read_to_string(bat.join("capacity"))
        .ok()
        .map(|s| s.trim().to_string());
    let status = std::fs::read_to_string(bat.join("status"))
        .ok()
        .map(|s| s.trim().to_string());
    match (cap, status) {
        (Some(c), Some(s)) => format!("{s} {c}%"),
        (Some(c), None) => format!("{c}%"),
        _ => String::new(),
    }
}

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

fn current_dir() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// AWS 段:当前 profile(env `AWS_PROFILE`/`AWS_DEFAULT_PROFILE`)。
fn aws_text() -> String {
    env_var("AWS_PROFILE")
        .or_else(|| env_var("AWS_DEFAULT_PROFILE"))
        .unwrap_or_default()
}

/// gcloud 段:当前配置名(active_config 文件内容)。
fn gcloud_text() -> String {
    let dir = env_var("CLOUDSDK_CONFIG").unwrap_or_else(|| format!("{}/.config/gcloud", home()));
    std::fs::read_to_string(format!("{dir}/active_config"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// azure 段:默认订阅名(azureProfile.json 中 `isDefault: true` 的 name)。
fn azure_text() -> String {
    let dir = env_var("AZURE_CONFIG_DIR").unwrap_or_else(|| format!("{}/.azure", home()));
    let Some(s) = std::fs::read_to_string(format!("{dir}/azureProfile.json")).ok() else {
        return String::new();
    };
    let mut rest = s.as_str();
    while let Some(pos) = rest.find("\"isDefault\":") {
        // `isDefault` 后允许空白,再判断是否为 true。
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

/// kubecontext 段:当前 context(读 `$KUBECONFIG` 或 `~/.kube/config`)。
fn kubecontext_text(_cwd: &str) -> String {
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

/// terraform 段:当前 workspace(读 `$TF_DATA_DIR`/`.terraform/environment`)。
fn terraform_text(cwd: &str) -> String {
    let dir = env_var("TF_DATA_DIR").unwrap_or_else(|| ".terraform".into());
    std::fs::read_to_string(std::path::Path::new(cwd).join(dir).join("environment"))
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// 本机第一个非回环 IPv4(跑 `ip -4 addr show`,跳过 127.0.0.1)。
fn ip_text() -> String {
    run_cmd("ip", &["-4", "addr", "show"])
        .unwrap_or_default()
        .lines()
        .filter(|l| l.contains("inet ") && !l.contains("127.0.0.1"))
        .find_map(|l| l.split("inet ").nth(1)?.split('/').next())
        .unwrap_or_default()
        .to_string()
}

/// VPN 接口 IP(匹配 tailscale/wg/tun/zt 的 IPv4)。
fn vpn_ip_text() -> String {
    let out = run_cmd("ip", &["addr", "show"]).unwrap_or_default();
    let mut ifname = String::new();
    for line in out.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("inet ") {
            if is_vpn_if(&ifname) {
                let ip = rest.split('/').next().unwrap_or("");
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        } else if let Some(name) = line.split(": ").nth(1).and_then(|s| s.split(':').next()) {
            ifname = name.to_string();
        }
    }
    String::new()
}

fn is_vpn_if(name: &str) -> bool {
    ["tailscale", "wg", "tun", "zt"]
        .iter()
        .any(|p| name.starts_with(p))
}

/// WiFi:读 /proc/net/wireless 的接口名 + 信号质量。
fn wifi_text() -> String {
    std::fs::read_to_string("/proc/net/wireless")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.contains(':')) // 数据行(`接口名: ...`)
                .map(|l| l.to_string())
        })
        .and_then(|l| {
            let mut p = l.split_whitespace();
            let ifn = p.next()?.trim_end_matches(':').to_string();
            let _ = p.next(); // flags
            let q = p.next()?; // link quality
            Some(format!("{ifn} {}", q.trim_end_matches('.')))
        })
        .unwrap_or_default()
}

/// 公网 IP:curl 查询(经 run_cmd 缓存;首次同步,后续直接用缓存)。
fn public_ip_text() -> String {
    run_cmd("curl", &["-s", "--max-time", "4", "https://v4.ident.me/"]).unwrap_or_default()
}

// 补齐一批 p10k 常见段:虚拟化/容器/目录权限/history 作用域/语言栈工具/包管理器。

fn detect_virt() -> String {
    run_cmd("systemd-detect-virt", &[])
        .filter(|v| v != "none")
        .unwrap_or_default()
}

fn toolbox_text() -> String {
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

fn dir_writable_text(cwd: &str) -> String {
    use std::os::unix::fs::MetadataExt;
    let writable = std::fs::metadata(cwd)
        .map(|m| m.mode() & 0o222 != 0)
        .unwrap_or(true);
    if writable { String::new() } else { "!".into() }
}

fn per_directory_history_text() -> String {
    match env_var("PER_DIRECTORY_HISTORY_TOGGLE").as_deref() {
        Some("global") => "global".into(),
        Some(_) => "local".into(),
        None => String::new(),
    }
}

fn haskell_stack_text() -> String {
    run_cmd("stack", &["--version"])
        .and_then(|s| {
            s.split_whitespace()
                .nth(1)
                .map(|v| v.trim_end_matches(',').to_string())
        })
        .unwrap_or_default()
}

/// 从 cwd 祖先找 `filename`,返回其完整内容。
fn find_up_content(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            return Some(c);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

/// package 段:package.json 的 `name`/`version`。
fn package_text(cwd: &str) -> String {
    let Some(content) = find_up_content(cwd, "package.json") else {
        return String::new();
    };
    let name = json_str_field(&content, "\"name\":");
    let version = json_str_field(&content, "\"version\":");
    match (name, version) {
        (Some(n), Some(v)) => format!("{n}@{}", v.trim_start_matches('v')),
        (Some(n), None) => n,
        _ => String::new(),
    }
}

/// 从 JSON 字符串里取某字段 `"key": "value"` 的 value。
fn json_str_field(content: &str, key: &str) -> Option<String> {
    let pos = content.find(key)?;
    let rest = &content[pos + key.len()..];
    let rest = rest
        .trim_start()
        .strip_prefix(':')
        .unwrap_or(rest)
        .trim_start();
    Some(rest.strip_prefix('"')?.split('"').next()?.to_string())
}

/// asdf 段:.tool-versions 第一个插件版本行(简化,显示首行)。
fn asdf_text(cwd: &str) -> String {
    find_up_version(cwd, ".tool-versions").unwrap_or_default()
}

/// fvm 段:.fvm/flutter_sdk(或其父目录)检测到 Flutter 版本。
fn fvm_text(cwd: &str) -> String {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let sdk = dir.join(".fvm").join("flutter_sdk");
        if sdk.exists() {
            // 从版本路径推导(如 …/versions/3.22.0)。
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

// 批次8:云子类 / 框架 / 待办命令段。

/// 找 cwd 祖先含 `filename` 的目录。
fn find_up_dir(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        if dir.join(filename).exists() {
            return Some(dir.to_string_lossy().into_owned());
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

/// GCP 服务账号 JSON 的 project_id。
fn google_app_cred_text() -> String {
    let Some(path) = env_var("GOOGLE_APPLICATION_CREDENTIALS") else {
        return String::new();
    };
    let Some(content) = std::fs::read_to_string(&path).ok() else {
        return String::new();
    };
    json_str_field(&content, "\"project_id\"").unwrap_or_default()
}

/// Elastic Beanstalk 环境名(eb list 中带 `* ` 的当前环境行)。
fn aws_eb_env_text() -> String {
    run_cmd("eb", &["list"])
        .unwrap_or_default()
        .lines()
        .find(|l| l.trim_start().starts_with("* "))
        .map(|l| l.trim_start_matches("* ").trim().to_string())
        .unwrap_or_default()
}

/// Laravel 版本:祖先含 artisan 时跑 `php artisan --version`。
fn laravel_version_text(cwd: &str) -> String {
    let Some(dir) = find_up_dir(cwd, "artisan") else {
        return String::new();
    };
    run_cmd("php", &[&format!("{dir}/artisan"), "--version"])
        .and_then(|s| s.split_whitespace().nth(2).map(|v| v.to_string()))
        .unwrap_or_default()
}

fn count_rs(dir: &std::path::Path) -> usize {
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

/// RSpec 完成度:app 与 spec 里 .rb 的比例。
fn rspec_stats_text(cwd: &str) -> String {
    let app = count_rs(&std::path::Path::new(cwd).join("app"));
    let spec = count_rs(&std::path::Path::new(cwd).join("spec"));
    let total = app + spec;
    if total == 0 {
        return String::new();
    }
    format!("RSpec: {}%", app * 100 / total)
}

fn todo_text() -> String {
    run_cmd("todo.sh", &["-p", "ls"])
        .and_then(|s| s.lines().last().map(|l| l.to_string()))
        .unwrap_or_default()
}

fn taskwarrior_text() -> String {
    run_cmd("task", &["+PENDING", "count"]).unwrap_or_default()
}

fn dropbox_text() -> String {
    run_cmd("dropbox-cli", &["filestatus", "."])
        .and_then(|s| s.lines().next().map(|l| l.to_string()))
        .unwrap_or_default()
}

/// date 段：按 `date-format`(strftime)格式化当前日期，默认对齐 p10k `%d.%m.%y`。
fn date_text(seg: &crate::config::Segment) -> String {
    let fmt = match seg.prop("date-format") {
        Some(crate::config::Prop::Str(s)) => s.as_str(),
        _ => "%d.%m.%y",
    };
    strftime_now(fmt)
}

fn strftime_now(fmt: &str) -> String {
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

// 工具链版本段:跑 `cmd --version` 缓存输出并解析版本号,有命令才显示。
thread_local! {
    static CMD_CACHE: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

fn run_cmd(cmd: &str, args: &[&str]) -> Option<String> {
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
                // 部分命令(如 java)把版本信息写到 stderr,stdout 为空时回退。
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

fn go_version() -> String {
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

fn rust_version() -> String {
    // "rustc 1.97.1 (…)" → "1.97.1"
    run_cmd("rustc", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

fn node_version() -> String {
    // "v26.8.1" → "26.8.1"
    run_cmd("node", &["--version"])
        .map(|s| s.trim_start_matches('v').to_string())
        .unwrap_or_default()
}

fn php_version() -> String {
    // "PHP 8.2.0 (cli)" → "8.2.0"
    run_cmd("php", &["--version"])
        .and_then(|s| Some(s.split_whitespace().nth(1)?.to_string()))
        .unwrap_or_default()
}

fn java_version() -> String {
    // '"17.0.9" ...' → "17.0.9"(截到首个 `-`)
    run_cmd("java", &["-fullversion"])
        .and_then(|s| {
            let s = s.split('"').nth(1)?;
            Some(s.split('-').next().unwrap_or(s).to_string())
        })
        .unwrap_or_default()
}

fn dotnet_version() -> String {
    run_cmd("dotnet", &["--version"]).unwrap_or_default()
}

fn swift_version() -> String {
    // "Apple Swift version 5.9 (…)" → 取首个数字开头词
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

fn terraform_version() -> String {
    // "Terraform v1.15.9\non linux_amd64…" → "1.15.9"(只取首行)
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

fn cpu_arch() -> String {
    std::fs::read_to_string("/proc/sys/kernel/arch")
        .ok()
        .or_else(|| run_cmd("uname", &["-m"]))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

// 环境管理器/*env 家族段:从环境变量或 cwd 祖先的 `.X-version` 文件取当前版本,
// 沿用 p10k 的"环境变量 > 本地文件 > 全局"优先级,激活才显示。

fn basename(p: &str) -> String {
    p.rsplit('/').next().unwrap_or(p).to_string()
}

/// 从 cwd 祖先目录找 `filename`,返回其首个非空行(版本文件)。
fn find_up_version(cwd: &str, filename: &str) -> Option<String> {
    let mut dir = std::path::Path::new(cwd);
    loop {
        let p = dir.join(filename);
        if let Ok(c) = std::fs::read_to_string(&p) {
            let first = c.lines().next().unwrap_or("").trim();
            if !first.is_empty() {
                return Some(first.to_string());
            }
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
}

fn env_or_file(env: &str, file: &str, cwd: &str) -> Option<String> {
    env_var(env).or_else(|| find_up_version(cwd, file))
}

fn virtualenv_text(_cwd: &str) -> String {
    env_var("VIRTUAL_ENV")
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

fn anaconda_text(_cwd: &str) -> String {
    env_var("CONDA_PREFIX")
        .or_else(|| env_var("CONDA_ENV_PATH"))
        .map(|v| format!("({})", basename(&v)))
        .unwrap_or_default()
}

fn pyenv_text(cwd: &str) -> String {
    env_or_file("PYENV_VERSION", ".python-version", cwd)
        .map(|v| v.trim_start_matches("python-").to_string())
        .unwrap_or_default()
}

fn nodeenv_text(_cwd: &str) -> String {
    env_var("NODE_VIRTUAL_ENV")
        .map(|v| format!("[{}]", basename(&v)))
        .unwrap_or_default()
}

fn nodenv_text(cwd: &str) -> String {
    env_or_file("NODENV_VERSION", ".node-version", cwd).unwrap_or_default()
}

fn nvm_text(_cwd: &str) -> String {
    // 简化:NVM_DIR 存在即显示当前 node 版本。
    env_var("NVM_DIR")
        .map(|_| node_version())
        .unwrap_or_default()
}

fn rbenv_text(cwd: &str) -> String {
    env_or_file("RBENV_VERSION", ".ruby-version", cwd)
        .map(|v| v.trim_start_matches("ruby-").to_string())
        .unwrap_or_default()
}

fn chruby_text(_cwd: &str) -> String {
    env_var("RUBY_ENGINE").unwrap_or_default()
}

fn rvm_text(_cwd: &str) -> String {
    env_var("GEM_HOME")
        .filter(|g| g.contains("rvm"))
        .map(|g| basename(&g))
        .unwrap_or_default()
}

fn goenv_text(cwd: &str) -> String {
    env_or_file("GOENV_VERSION", ".go-version", cwd)
        .map(|v| v.trim_start_matches("go-").to_string())
        .unwrap_or_default()
}

fn jenv_text(cwd: &str) -> String {
    env_or_file("JENV_VERSION", ".java-version", cwd).unwrap_or_default()
}

fn phpenv_text(cwd: &str) -> String {
    env_or_file("PHPENV_VERSION", ".php-version", cwd).unwrap_or_default()
}

fn luaenv_text(cwd: &str) -> String {
    env_or_file("LUAENV_VERSION", ".lua-version", cwd).unwrap_or_default()
}

fn plenv_text(cwd: &str) -> String {
    env_or_file("PLENV_VERSION", ".perl-version", cwd).unwrap_or_default()
}

fn scalaenv_text(cwd: &str) -> String {
    env_or_file("SCALAENV_VERSION", ".scala-version", cwd).unwrap_or_default()
}

fn perlbrew_text(_cwd: &str) -> String {
    env_var("PERLBREW_PERL")
        .map(|v| v.trim_start_matches("perl-").to_string())
        .unwrap_or_default()
}

fn detect_os_icon() -> String {
    let uname = std::env::consts::OS;
    if uname != "linux" {
        return match uname {
            "macos" => "\u{f179}".into(),   // 
            "windows" => "\u{f17a}".into(), // 
            _ => "\u{f17c}".into(),         // 默认 linux 图标
        };
    }
    // Linux:读 /etc/os-release 的 ID(子串匹配,对齐 p10k case *arch* 等)。
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
    let icon = if id.contains("arch") {
        "\u{f303}" // 
    } else if id.contains("ubuntu") {
        "\u{f31b}" // 
    } else if id.contains("debian") {
        "\u{f306}" // 
    } else if id.contains("fedora") {
        "\u{f30a}" // 
    } else if id.contains("gentoo") {
        "\u{f30d}" // 
    } else if id.contains("nixos") {
        "\u{f313}" // 
    } else if id.contains("manjaro") {
        "\u{f312}" // 
    } else if id.contains("mint") {
        "\u{f30e}" // 
    } else if id.contains("alpine") {
        "\u{f300}" // 
    } else if id.contains("void") {
        "\u{f32e}" // 
    } else if id.contains("artix") {
        "\u{f31f}" // 
    } else if id.contains("opensuse") || id.contains("suse") {
        "\u{f314}" // 
    } else {
        "\u{f17c}" // 默认 
    };
    icon.to_string()
}

/// 当前时间 HH:MM:SS(libc localtime)。
fn now_hhmmss() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

/// 命令时长格式(对齐 p10k lean PRECISION=0):<60s → `Ns`;否则 `Xm Ys`/`Xh Ym Zs`。
fn format_duration(secs: f64) -> String {
    let s = secs.round() as u64;
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m{}s", s / 60, s % 60)
    } else {
        format!("{}h{}m{}s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

/// `dir` 段文本:折叠(truncate_to_unique)+ 逐部件按类别上色。
/// - 锚(`~`/当前目录/marker 祖先):`ANCHOR` state
/// - 缩短: `SHORTENED` state
/// - 普通:段默认;`/` 分隔符本色(不随部件)。
fn dir_seg_text(
    config: &Config,
    info: &HeaderInfo,
    seg: &crate::config::Segment,
    default: &Style,
) -> String {
    let shorten = shorten_len(seg);
    let cwd = std::path::Path::new(&info.cwd);
    let home = std::env::var("HOME").ok();
    let home = home.as_deref().map(std::path::Path::new);
    let parts = crate::dir_shorten::truncate_to_unique(cwd, shorten, home);
    let mut s = String::new();
    let is_home = home.map(|h| cwd.starts_with(h)).unwrap_or(false);
    if is_home {
        // home 前缀 `~`。
        let st = seg.effective_style(Some("ANCHOR"), &config.defaults);
        s.push_str(&paint("~", &st));
        if !parts.is_empty() {
            s.push_str(&paint("/", default));
        }
    } else if !parts.is_empty() {
        // 绝对路径起始 `/`。
        s.push_str(&paint("/", default));
    }
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str(&paint("/", default)); // 分隔符本色
        }
        let state = match part.class {
            crate::dir_shorten::Class::Anchor => Some("ANCHOR"),
            crate::dir_shorten::Class::Shortened => Some("SHORTENED"),
            crate::dir_shorten::Class::Normal => None,
        };
        let st = seg.effective_style(state, &config.defaults);
        s.push_str(&paint(&part.text, &st));
    }
    if seg.content.is_some() {
        value_of(seg.content.as_deref(), s)
    } else {
        s
    }
}

/// 读 dir 段的 `shorten-dir-length`(保留末 N 级,默认 1=p10k)。
fn shorten_len(seg: &crate::config::Segment) -> usize {
    if let Some(crate::config::Prop::Int(n)) = seg.prop("shorten-dir-length") {
        let n = (*n).clamp(1, 20) as usize;
        n
    } else {
        1
    }
}

/// 展开环境变量:`${VAR}` 或 `$VAR` → 环境变量值;未定义 → 空串。
fn expand_env(s: &str) -> String {
    let b: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == '$' && i + 1 < b.len() {
            if b[i + 1] == '{' {
                if let Some(rel) = b[i + 2..].iter().position(|&c| c == '}') {
                    let name: String = b[i + 2..i + 2 + rel].iter().collect();
                    out.push_str(&std::env::var(&name).unwrap_or_default());
                    i = i + 2 + rel + 1;
                    continue;
                }
            } else if b[i + 1].is_ascii_alphanumeric() || b[i + 1] == '_' {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == '_') {
                    j += 1;
                }
                let name: String = b[i + 1..j].iter().collect();
                out.push_str(&std::env::var(&name).unwrap_or_default());
                i = j;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

/// git 文本:分支 + 计数。
fn vcs_text(vcs: Option<&GitStatus>) -> String {
    let Some(v) = vcs else { return String::new() };
    let mut s = String::new();
    if !v.branch.is_empty() {
        s.push_str("\u{f126} "); // 分支图标 
        s.push_str(&v.branch);
    }
    let mut parts: Vec<String> = Vec::new();
    if v.staged > 0 {
        parts.push(format!("+{}", v.staged));
    }
    if v.unstaged > 0 {
        parts.push(format!("~{}", v.unstaged));
    }
    if v.conflicted > 0 {
        parts.push(format!("!{}", v.conflicted));
    }
    if v.untracked > 0 {
        parts.push(format!("?{}", v.untracked));
    }
    if v.ahead > 0 {
        parts.push(format!("↑{}", v.ahead));
    }
    if v.behind > 0 {
        parts.push(format!("↓{}", v.behind));
    }
    if v.stashes > 0 {
        parts.push(format!("≡{}", v.stashes));
    }
    if !parts.is_empty() {
        s.push(' ');
        s.push_str(&parts.join(" "));
    }
    s
}

/// 退出码状态:✓ / ✘ N。
fn status_text(info: &HeaderInfo) -> String {
    match info.exit_code {
        None => String::new(),
        Some(0) => "✓".to_string(),
        Some(n) => format!("✘ {n}"),
    }
}

/// 有 content 配置则用,否则用默认文本。
fn value_of<'a>(content: Option<&'a str>, default: impl Into<String>) -> String {
    content.map(String::from).unwrap_or(default.into())
}

/// 把一行拼成 ANSI:左段串(段间 `sub`/`segment` 分隔、末段 `end` 端符)+ 右段右对齐。
fn assemble_row(
    left: &[SegmentText],
    right: &[SegmentText],
    cols: usize,
    seps: &crate::config::Separators,
) -> String {
    let mut out = String::new();
    let mut prev_bg = Color::Default;
    let mut has_left = false;
    for s in left {
        if s.text.is_empty() {
            continue;
        }
        // 段间分隔(p10k 决策):前段有背景 → 画分隔符(同底 → sub,异色/当前无底 → segment);
        // 前段无背景 → 纯空格。
        if has_left {
            let prev_has = prev_bg != Color::Default;
            if prev_has {
                let same = s.style.bg != Color::Default && s.style.bg == prev_bg;
                let ch = if same { &seps.sub } else { &seps.segment };
                if !ch.is_empty() {
                    if same {
                        out.push_str(&paint(ch, &s.style));
                    } else {
                        out.push_str(&arrow(ch, prev_bg.clone(), s.style.bg.clone()));
                    }
                } else {
                    out.push(' ');
                }
            } else {
                out.push(' ');
            }
        }
        out.push_str(&s.text); // text 已上色,不再二次上色
        prev_bg = s.style.bg.clone();
        has_left = true;
    }
    // 左栏末尾端符(最后一段后,用它自己的背景指向行尾)。
    if has_left && !seps.end.is_empty() && prev_bg != Color::Default {
        out.push_str(&arrow(&seps.end, prev_bg.clone(), Color::Default));
    }
    let mut right_str = String::new();
    let parts: Vec<&SegmentText> = right.iter().filter(|s| !s.text.is_empty()).collect();
    if !parts.is_empty() {
        // 右段行首端符(左三角 ):前景=右段**背景色**(指向右段);画在 gap 上。
        if !seps.right_start.is_empty() {
            right_str.push_str(&arrow(
                &seps.right_start,
                parts[0].style.bg.clone(),
                Color::Default,
            ));
        }
        for (i, s) in parts.iter().enumerate() {
            if i > 0 {
                // 右段间分隔:同色 → right_sub(段前景画在段背景),异色 → right_segment
                // (左三角,前景(三角块)=当前段(右)背景色,背景=前段(左)背景色;
                // 与左段 ``(fg=前段bg、bg=当前段bg)镜像)。
                let prev_bg = parts[i - 1].style.bg.clone();
                if prev_bg != Color::Default {
                    let same = s.style.bg != Color::Default && s.style.bg == prev_bg;
                    if same {
                        if !seps.right_sub.is_empty() {
                            right_str.push_str(&paint(&seps.right_sub, &s.style));
                        }
                    } else if !seps.right_segment.is_empty() {
                        right_str.push_str(&arrow(
                            &seps.right_segment,
                            s.style.bg.clone(),
                            prev_bg,
                        ));
                    }
                }
            }
            right_str.push_str(&s.text);
        }
        let lw = display_width(&out);
        let rw = display_width(&right_str);
        // 右对齐:gap 字符填满左段到右段起点之间。
        if cols > rw {
            let start = cols - rw; // 右段起点(0-based)
            if start > lw {
                let gap_char = if seps.gap.is_empty() {
                    " ".to_string()
                } else {
                    seps.gap.clone()
                };
                out.push_str(&gap_char.repeat(start - lw));
            }
        }
        out.push_str(&right_str);
    }
    out
}

/// 画一个分隔符箭头:`fg` 为它的前景色(连接前一片背景),`bg` 为背景色。
fn arrow(ch: &str, fg: Color, bg: Color) -> String {
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

/// 用样式上色(前景 + 背景块 + 粗体;纯文本,无分隔符）。
fn paint(text: &str, style: &Style) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut s = String::new();
    match &style.fg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[38;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[38;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[38;5;{}m", named_256(n)).unwrap(),
    }
    match &style.bg {
        Color::Default => {}
        Color::Xterm(n) => write!(s, "\x1b[48;5;{n}m").unwrap(),
        Color::Rgb(r, g, b) => write!(s, "\x1b[48;2;{r};{g};{b}m").unwrap(),
        Color::Named(n) => write!(s, "\x1b[48;5;{}m", named_256(n)).unwrap(),
    }
    if style.bold {
        s.push_str("\x1b[1m");
    }
    s.push_str(text);
    s.push_str("\x1b[0m");
    s
}

/// 标准 16 色名映射到 256 色索引。
fn named_256(name: &str) -> u8 {
    match name {
        "black" => 0,
        "red" => 1,
        "green" => 2,
        "yellow" => 3,
        "blue" => 4,
        "magenta" => 5,
        "cyan" => 6,
        "white" => 7,
        "brightblack" => 8,
        "brightred" => 9,
        "brightgreen" => 10,
        "brightyellow" => 11,
        "brightblue" => 12,
        "brightmagenta" => 13,
        "brightcyan" => 14,
        "brightwhite" => 15,
        _ => 7,
    }
}

/// 显示宽度(去 ANSI,按 Unicode 显示宽度:emoji/CJK 宽字符算 2,组合/零宽算 0)。
fn display_width(s: &str) -> usize {
    let mut w = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if in_esc {
            if c == 'm' {
                in_esc = false;
            }
            continue;
        }
        if c == '\x1b' {
            in_esc = true;
            continue;
        }
        w += UnicodeWidthChar::width(c).unwrap_or(0);
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(cwd: &str, code: Option<i32>) -> HeaderInfo {
        HeaderInfo {
            exit_code: code,
            cwd: cwd.to_string(),
            exec_seconds: 0.0,
            jobs: 0,
        }
    }

    #[test]
    fn renders_pure_text_header_with_right_align() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("\r\n");
        // header 只一行行:含目录 + ✓;prompt_char(❯)不在 header(由 render_prompt 画)。
        assert!(h.contains("tmp"), "header 应含目录,实际：{h:?}");
        assert!(h.contains("✓"));
        assert!(
            !h.contains('❯'),
            "输入行前缀 ❯ 由 render_prompt 画，不应在 header"
        );
        assert_eq!(h.split("\r\n").count(), 1, "lean header 一行");
    }

    #[test]
    fn right_aligns_to_cols() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", Some(0)), None, 80).join("\r\n");
        // 右段 ✓ 右对齐:gap 填充使整行显示宽度 = cols。
        assert!(h.contains('✓'), "右段应存在,实际:{h:?}");
        assert_eq!(display_width(&h), 80, "右对齐后行宽应为 80");
    }

    #[test]
    fn line_text_literal_renders() {
        // line 里可穿插静态文本 text "some text"。
        let cfg = Config::parse(
            "layout {\n  left {\n    line { dir #true; text \"some text\"; vcs #true }\n  }\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            h.contains("some text"),
            "line 里的 text 静态文本应渲染，实际：{h:?}"
        );
    }

    #[test]
    fn line_text_expands_env() {
        // text 里的 ${VAR}/$VAR 展开为环境变量。
        let home = std::env::var("HOME").unwrap_or_default();
        let cfg =
            Config::parse("layout {\n  left {\n    line { text \"home=${HOME} $USER\" }\n  }\n}")
                .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains(&home), "text 应展开环境变量，实际：{h:?}");
    }

    #[test]
    fn first_header_line_gets_frame() {
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  first-prefix \"╭─\"\n  first-suffix \"─╮\"\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains("╭─"), "首行应有 first-prefix 帧，实际：{h:?}");
        assert!(h.contains("─╮"), "首行应有 first-suffix 帧");
    }

    #[test]
    fn input_prefix_width_matches_visual() {
        // 有帧 last-prefix ╰─ 时,前缀 = "╰─❯ " (宽4);占位符要按这个宽度生成。
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  last-prefix \"╰─\"\n}",
        )
        .unwrap();
        let p = input_prefix(&cfg, None);
        assert_eq!(
            p.width, 4,
            "╰─❯ 空格 应宽4, 实际 width={} text={:?}",
            p.width, p.text
        );
        assert!(p.text.contains('╰'), "前缀应含帧 last-prefix");
        assert!(p.text.contains('❯'), "前缀应含 prompt_char");
        assert!(p.text.ends_with(' '), "前缀应以空格收尾");
    }

    #[test]
    fn vcs_counts_appear() {
        let cfg = Config::default_lean().unwrap();
        let v = GitStatus {
            branch: "master".into(),
            staged: 1,
            unstaged: 2,
            conflicted: 0,
            untracked: 3,
            ahead: 1,
            behind: 0,
            stashes: 0,
            remote_url: String::new(),
        };
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\r\n");
        assert!(h.contains("master"));
        assert!(h.contains("+1"));
        assert!(h.contains("~2"));
        assert!(h.contains("?3"));
    }

    #[test]
    fn background_blocks_and_powerline_arrow() {
        // dir/vcs 都有背景,且不同 → 段间画 ``(fg=前段bg, bg=后段bg)。
        let cfg = Config::parse(
            "layout { left { line { dir #true; vcs #true } } }\n\
             segments { dir { bg 39 } }\n",
        )
        .unwrap();
        // 手动给 dir 加 bg、并加一个 vcs 段有 bg(两者异色)
        let mut cfg = cfg;
        cfg.segments.get_mut("dir").unwrap().style.bg = Color::Xterm(39);
        cfg.segments.get_mut("dir").unwrap().style.fg = Color::Xterm(0);
        cfg.segments.insert(
            "vcs".into(),
            crate::config::Segment {
                style: crate::config::Style {
                    fg: Color::Xterm(0),
                    bg: Color::Xterm(76),
                    bold: false,
                },
                ..Default::default()
            },
        );
        let v = GitStatus {
            branch: "master".into(),
            staged: 0,
            unstaged: 0,
            conflicted: 0,
            untracked: 0,
            ahead: 0,
            behind: 0,
            stashes: 0,
            remote_url: String::new(),
        };
        // 设置异底段间用 powerline 箭头,末尾端符。
        cfg.separators.segment = "\u{e0b0}".into();
        let h = render_header_lines(&cfg, &info("/tmp", None), Some(&v), 80).join("\r\n");
        assert!(h.contains("\x1b[48;5;39m"), "dir 应有背景块 39");
        assert!(h.contains("\x1b[48;5;76m"), "vcs 应有背景块 76");
        assert!(
            h.contains('\u{e0b0}'),
            "异底段间应画可配置的 segment 分隔符()"
        );
    }

    #[test]
    fn frame_colors_pieces_with_override() {
        // 帧级 fg 作用于所有块;子节点自己的 fg 覆盖帧级。
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             frame fg=76 {\n  first-prefix \"╭─\"\n  last-prefix \"╰─\" fg=196\n}",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            h.contains("\x1b[38;5;76m╭─"),
            "首行前缀用帧级 fg,实际:{h:?}"
        );
        // 输入行前缀:last-prefix 用子节点色(196),prompt_char ❯ 缺省无色。
        let p = input_prefix(&cfg, None);
        assert!(
            p.text.contains("\x1b[38;5;196m╰─"),
            "last-prefix 子节点 fg 应覆盖帧级,实际:{:?}",
            p.text
        );
        assert!(p.text.contains('❯'), "输入行前缀仍含 prompt_char ❯");
    }

    #[test]
    fn prompt_char_configurable_with_error_state() {
        // char 属性配提示符字符;state ERROR 覆盖错误态(退出码非 0)的字符与颜色。
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\">\" fg=76 {\n  state ERROR char=\"✘\" fg=196\n} }",
        )
        .unwrap();
        let ok = input_prefix(&cfg, Some(0));
        assert!(
            ok.text.contains("\x1b[38;5;76m>"),
            "正常态应显示 char(>) 且用 fg=76,实际:{:?}",
            ok.text
        );
        assert!(!ok.text.contains('❯'), "配了 char 就不该用默认 ❯");
        let err = input_prefix(&cfg, Some(1));
        assert!(
            err.text.contains("\x1b[38;5;196m✘"),
            "错误态应显示 state ERROR 的 char/颜色,实际:{:?}",
            err.text
        );
        let none = input_prefix(&cfg, None);
        assert!(none.text.contains('>'), "无退出码(首 prompt)按正常态");
    }

    #[test]
    fn prompt_char_states_must_match_width() {
        // 正常态与 ERROR 态 char 等宽(都单字符)→ 校验通过。
        let ok_cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\"❯\" {\n  state ERROR char=\"✘\"\n} }",
        )
        .unwrap();
        assert!(check_prompt_char_widths(&ok_cfg).is_ok(), "等宽应通过校验");
        // 宽度不等(✘✘ 双宽)→ 校验报错。
        let bad_cfg = Config::parse(
            "layout { left { line { dir #true } } }\n\
             segments { prompt_char char=\"❯\" {\n  state ERROR char=\"✘✘\"\n} }",
        )
        .unwrap();
        assert!(check_prompt_char_widths(&bad_cfg).is_err(), "不等宽应报错");
    }

    #[test]
    fn segment_attach_text_slots() {
        // 附加文字槽:左(icon 前)/中(icon 与内容间)/右(内容后),仅拼接、不独立成块。
        let cfg = Config::parse(
            "layout { left { line { foo #true } } }\n\
             segments { foo fg=15 bg=236 icon=\"🕐\" content=\"02:49:19\" {\n\
               text-left \"L\"\n\
               text-middle \"M\"\n\
               text-right \"R\" fg=196\n\
             } }",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        let li = h.find('L').expect("text-left 应渲染");
        let ii = h.find("🕐").expect("icon 应渲染");
        let mi = h.find('M').expect("text-middle 应渲染");
        let ci = h.find("02:49:19").expect("内容应渲染");
        let ri = h.find('R').expect("text-right 应渲染");
        assert!(
            li < ii && ii < mi && mi < ci && ci < ri,
            "顺序应为 左<icon<中<内容<右,实际:{h:?}"
        );
        assert!(
            h.contains("\x1b[38;5;196m\x1b[48;5;236mR"),
            "text-right 配 fg=196 应覆盖前景,实际:{h:?}"
        );
    }

    #[test]
    fn segment_attach_middle_requires_icon_and_text() {
        // text-middle 仅当段既有 icon 又有文字时才渲染。
        let only_icon = Config::parse(
            "layout { left { line { foo #true } } }\n\
             segments { foo icon=\"🕐\" { text-middle \"M\" } }",
        )
        .unwrap();
        let h = render_header_lines(&only_icon, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains('M'),
            "仅 icon 无文字时 text-middle 应不渲染,实际:{h:?}"
        );

        let only_text = Config::parse(
            "layout { left { line { foo #true } } }\n\
             segments { foo content=\"X\" { text-middle \"M\" } }",
        )
        .unwrap();
        let h = render_header_lines(&only_text, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            !h.contains('M'),
            "仅文字无 icon 时 text-middle 应不渲染,实际:{h:?}"
        );
    }

    #[test]
    fn display_width_counts_wide_chars() {
        assert_eq!(display_width("🎂"), 2, "emoji 应宽 2 列");
        assert_eq!(display_width("a🎂b"), 4);
        assert_eq!(display_width("2026"), 4);
    }

    #[test]
    fn attach_emoji_keeps_row_width_aligned() {
        // 附加 emoji 占 2 列,右对齐预算必须按 Unicode 宽度算,否则末尾被折行。
        let src = "layout {\n  left { line { dir #true } }\n  right { line { foo #true } }\n}\n\
                   segments { foo icon=\"🕐\" content=\"12:34:56\" { text-right \"🎂\" } }";
        let cfg = Config::parse(src).unwrap_or_else(|e| panic!("parse: {e}\nsrc={src:?}"));
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 40).join("\r\n");
        assert_eq!(display_width(&h), 40, "行宽应仍对齐 cols=40,实际:{h:?}");
    }

    #[test]
    fn user_and_date_segments_render() {
        let cfg = Config::parse(
            "layout { left { line { user #true; date #true } } }\n\
             segments { date date-format=\"%Y\" }",
        )
        .unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        let user = std::env::var("USER").unwrap_or_default();
        assert!(h.contains(&user), "user 段应显示 $USER,实际:{h:?}");
        let digits: Vec<char> = h.chars().filter(|c| c.is_ascii_digit()).collect();
        assert!(
            digits.len() >= 4,
            "date 段 date-format=%Y 应输出四位年份,实际:{h:?}"
        );
    }

    #[test]
    fn context_hidden_for_local_non_ssh() {
        // 非 SSH 时 context 不出现 user@host 形式(本地 root 只显示 user)。
        let cfg = Config::parse("layout { left { line { context #true } } }").unwrap();
        let h = render_header_lines(&cfg, &info("/tmp", None), None, 80).join("\r\n");
        assert!(!h.contains('@'), "非 SSH 不应显示 user@host,实际:{h:?}");
    }

    #[test]
    fn env_indicator_segments() {
        // ranger:设 RANGER_LEVEL 显示层级,清除后隐藏。
        let r = Config::parse("layout { left { line { ranger #true } } }").unwrap();
        unsafe {
            std::env::set_var("RANGER_LEVEL", "2");
        }
        let h = render_header_lines(&r, &info("/tmp", None), None, 80).join("\r\n");
        assert!(h.contains('2'), "ranger 应显示层级,实际:{h:?}");
        unsafe {
            std::env::remove_var("RANGER_LEVEL");
        }
        let h2 = render_header_lines(&r, &info("/tmp", None), None, 80).join("\r\n");
        assert_eq!(h2.trim(), "", "清除后 ranger 应隐藏,实际:{h2:?}");
        // proxy:设 http_proxy 后显示其 host:port(清除不深究,机器可能自带代理 env)。
        let p = Config::parse("layout { left { line { proxy #true } } }").unwrap();
        unsafe {
            std::env::set_var("http_proxy", "http://proxy.example:8080");
        }
        let hp = render_header_lines(&p, &info("/tmp", None), None, 80).join("\r\n");
        assert!(
            hp.contains("proxy.example:8080"),
            "proxy 应显示 http_proxy 的 host:port,实际:{hp:?}"
        );
        unsafe {
            std::env::remove_var("http_proxy");
        }
    }

    #[test]
    fn version_segments_resolve() {
        // cpu_arch 从 /proc 或 uname 恒有;不存在的命令 → 段隐藏(None)。
        assert!(
            !cpu_arch().is_empty(),
            "cpu_arch 应有值,实际:{:?}",
            cpu_arch()
        );
        assert_eq!(run_cmd("no-such-cmd-p11k-test", &["--version"]), None);
    }

    #[test]
    fn system_resource_segments_resolve() {
        // /proc 数据源恒有(开发机为 Linux);battery 依赖硬件,不测。
        assert!(!load().is_empty(), "load 应从 /proc/loadavg 读出");
        assert!(!ram().is_empty(), "ram 应是 MemAvailable");
        assert!(!disk_usage("/tmp").is_empty(), "disk_usage 应有 df 结果");
    }
}
