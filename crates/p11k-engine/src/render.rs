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
                .map(|seg| render_row(config, seg, info, vcs))
                .unwrap_or_default();
            let r = right
                .get(i)
                .map(|seg| render_row(config, seg, info, vcs))
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
) -> Vec<SegmentText> {
    elements
        .iter()
        .map(|el| match el {
            Element::Seg(name) => render_segment(config, name, info, vcs),
            Element::Joined(name) => render_segment(config, name, info, vcs),
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
    // 仅拼接显示、不独立成块;fg 缺省跟段走。
    let mut out = String::new();
    if let Some(l) = &seg.text_left {
        out.push_str(&paint_attach(&style, l));
    }
    out.push_str(&icon_text);
    if let Some(m) = &seg.text_middle {
        if !icon_text.is_empty() && !text.is_empty() {
            out.push_str(&paint_attach(&style, m));
        }
    }
    out.push_str(&text);
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
        "dir" => Some("\u{f07c}".into()),             // 
        "vcs" => Some("\u{f1d3}".into()),             // 
        "time" => Some("\u{f017}".into()),            // 
        "background_jobs" => Some("\u{f013}".into()), // 齿轮 
        "date" => Some("\u{f073}".into()),            // 日历 
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
}
