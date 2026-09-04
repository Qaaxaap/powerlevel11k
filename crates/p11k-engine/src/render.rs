//! 渲染器(第一单元):把 KDL 配置驱动成多行 header 文本。
//!
//! 本单元只做最小闭环,不接分隔符/背景块/帧/图标(那些是后续单元):
//! - **几何**:按 [`Config::layout`] 的行结构,逐行"左段串 + 右段右对齐"。
//! - **段**:只有 `dir`/`vcs`/`status`/`prompt_char` 四个的最简内容(纯文本)。
//! - **色**:用配置的 `fg`/`bg`(三段回退),产出 256/真彩/粗体 ANSI。
//!
//! 输入:配置 + 当前状态(cwd/git/exit)。输出:header 的 ANSI 字符串。

use std::fmt::Write as _;

use std::cell::RefCell;
use crate::config::{Color, Config, Element, Style};
use crate::theme::{GitStatus, HeaderInfo};

/// 输入行前缀(配置驱动):`frame.last_prefix` + `prompt_char` 内容;`width` 是它的
/// 显示宽度,engine 用它生成等宽占位符(占位符宽度 = 前缀宽度,几何自洽)。
pub struct InputPrefix {
    pub text: String,
    pub width: usize,
}

/// 计算输入行前缀(如 `╰─❯`)。`last_prefix`/`text` 已上色;`width` 为去 ANSI 显示宽。
pub fn input_prefix(config: &Config) -> InputPrefix {
    let fg = Style { fg: config.defaults.fg.clone(), ..Default::default() };
    let mut text = String::new();
    if !config.frame.last_prefix.is_empty() {
        text.push_str(&paint(&config.frame.last_prefix, &fg));
    }
    let pc = config.segment("prompt_char");
    let st = pc.effective_style(None, &config.defaults);
    text.push_str(&paint("❯", &st));
    text.push(' '); // 前缀后空格(无色),对齐原 `❯ ` 几何
    let width = display_width(&text);
    InputPrefix { text, width }
}

/// 一段渲染结果:文本 + 它的样式(渲染期才装配 ANSI)。
struct SegmentText {
    text: String,
    style: Style,
}

/// 渲染整个 header(多行),返回 ANSI 字符串(行间以 `\r\n` 分隔,最后一行结尾无换行)。
pub fn render_header(config: &Config, info: &HeaderInfo, vcs: Option<&GitStatus>, cols: usize) -> String {
    render_header_lines(config, info, vcs, cols).join("\r\n")
}

/// 渲染 header 为**逐行内容**(每行=左段串+右段右对齐,不含光标/清屏/换行)。
/// 供 theme 层逐行 `\r\e[K` + 内容 + `\r\n` 画到终端。
pub fn render_header_lines(config: &Config, info: &HeaderInfo, vcs: Option<&GitStatus>, cols: usize) -> Vec<String> {
    let layout = &config.layout;
    let left = &layout.left;
    let right = &layout.right;
    let lines = left.len().max(right.len());
    let frame = &config.frame;
    (0..lines)
        .map(|i| {
            let l = left.get(i).map(|seg| render_row(config, seg, info, vcs)).unwrap_or_default();
            let r = right.get(i).map(|seg| render_row(config, seg, info, vcs)).unwrap_or_default();
            // 每行帧:首行 first,其余 header 行 newline。
            let (prefix, suffix) = if i == 0 {
                (&frame.first_prefix, &frame.first_suffix)
            } else {
                (&frame.newline_prefix, &frame.newline_suffix)
            };
            let fg = Style { fg: config.defaults.fg.clone(), ..Default::default() };
            let pre_w = display_width(prefix);
            let suf_w = display_width(suffix);
            let mut row = String::new();
            if !prefix.is_empty() {
                row.push_str(&paint(prefix, &fg));
            }
            // 右对齐预算 = cols - 前缀宽 - 后缀宽(否则帧把行撑宽、后缀挤到下一行)。
            let body = assemble_row(&l, &r, cols.saturating_sub(pre_w + suf_w), &config.separators);
            row.push_str(&body);
            if !suffix.is_empty() {
                row.push_str(&paint(suffix, &fg));
            }
            row
        })
        .collect()
}

/// 渲染一行:左段串(依次拼接)、右段右对齐。
fn render_row(config: &Config, elements: &[Element], info: &HeaderInfo, vcs: Option<&GitStatus>) -> Vec<SegmentText> {
    elements
        .iter()
        .map(|el| match el {
            Element::Seg(name) => render_segment(config, name, info, vcs),
            Element::Joined(name) => render_segment(config, name, info, vcs),
            Element::Text(t) => {
                let mut style = config.segment("text").effective_style(None, &config.defaults);
                if style.bg == Color::Default {
                    style.bg = if config.defaults.bg != Color::Default {
                        config.defaults.bg.clone()
                    } else {
                        Color::Xterm(0)
                    };
                }
                SegmentText { text: paint(&expand_env(t), &style), style }
            }
        })
        .collect()
}

/// 渲染单个段。返回的 `text` 已是**上色后的 ANSI**;`style` 供段间分隔符/块背景。
fn render_segment(config: &Config, name: &str, info: &HeaderInfo, vcs: Option<&GitStatus>) -> SegmentText {
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
        "vcs" => if seg.content.is_some() { paint(&value_of(seg.content.as_deref(), vcs_text(vcs)), &style) } else { paint(&vcs_text(vcs), &style) },
        "status" => paint(&status_text(info), &style),
        "prompt_char" => paint("❯", &style),
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
        _ => paint(&value_of(seg.content.as_deref(), String::new()), &style),
    };
    // 段图标(VISUAL_IDENTIFIER):配置 `icon` 优先,否则按段名内置默认;vcs 段按
    // 远端域名选图标(如 github 、aur )。图标+空格前缀,用段样式上色。
    let icon = seg.icon.clone().filter(|i| !i.is_empty()).or_else(|| {
        if name == "vcs" {
            vcs.as_ref().map(|v| vcs_remote_icon(config, &v.remote_url))
        } else {
            default_icon(name)
        }
    });
    let text = match icon {
        Some(ic) => format!("{}{}", paint(&format!("{ic} "), &style), text),
        None => text,
    };
    SegmentText { text, style }
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
        "dir" => Some("\u{f07c}".into()),            // 
        "vcs" => Some("\u{f1d3}".into()),            // 
        "time" => Some("\u{f017}".into()),           // 
        "background_jobs" => Some("\u{f013}".into()), // 齿轮 
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

fn detect_os_icon() -> String {
    let uname = std::env::consts::OS;
    if uname != "linux" {
        return match uname {
            "macos" => "\u{f179}".into(),  // 
            "windows" => "\u{f17a}".into(), // 
            _ => "\u{f17c}".into(),        // 默认 linux 图标
        };
    }
    // Linux:读 /etc/os-release 的 ID(子串匹配,对齐 p10k case *arch* 等)。
    let id = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("ID="))
                .map(|l| l.trim_start_matches("ID=").trim().trim_matches('"').to_string())
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
/// - 锚(`~`/当前目录/marker 祖先):`ANCHOR` state(39 粗体)
/// - 缩短: `SHORTENED` state(103)
/// - 普通:段默认;`/` 分隔符本色(不随部件)。
fn dir_seg_text(config: &Config, info: &HeaderInfo, seg: &crate::config::Segment, default: &Style) -> String {
    let shorten = shorten_len(seg);
    let cwd = std::path::Path::new(&info.cwd);
    let home = std::env::var("HOME").ok();
    let home = home.as_deref().map(std::path::Path::new);
    let parts = crate::dir_shorten::truncate_to_unique(cwd, shorten, home);
    let mut s = String::new();
    let is_home = home.map(|h| cwd.starts_with(h)).unwrap_or(false);
    if is_home {
        // home 前缀 `~`(anchor)。
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

/// git 文本:分支 + 计数(最简,图标/分色留给后续单元)。
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

/// 退出码状态:✓ / ✘ N(最简)。
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
fn assemble_row(left: &[SegmentText], right: &[SegmentText], cols: usize, seps: &crate::config::Separators) -> String {
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
        // 右段行首端符(左三角 ):前景=右段**前景色**(亮,画在 gap/终端上可见);
        // 右段间 sub():段前景色画在段背景上(p10k `$style`,黑块上亮细线可见)。
        if !seps.right_start.is_empty() {
            right_str.push_str(&arrow(&seps.right_start, parts[0].style.fg.clone(), Color::Default));
        }
        for (i, s) in parts.iter().enumerate() {
            if i > 0 && !seps.right_sub.is_empty() {
                right_str.push_str(&paint(&seps.right_sub, &s.style));
            }
            right_str.push_str(&s.text);
        }
        let lw = display_width(&out);
        let rw = display_width(&right_str);
        // 右对齐:gap 字符填满左段到右段起点之间。
        if cols > rw {
            let start = cols - rw; // 右段起点(0-based)
            if start > lw {
                let gap_char = if seps.gap.is_empty() { " ".to_string() } else { seps.gap.clone() };
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

/// 几个常用名字色映射到 256(第一单元最小;完整色表后续)。
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

/// 显示宽度(去 ANSI,按字符数,第一单元用简单宽度)。
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
        w += 1;
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(cwd: &str, code: Option<i32>) -> HeaderInfo {
        HeaderInfo { exit_code: code, cwd: cwd.to_string(), exec_seconds: 0.0, jobs: 0 }
    }

    #[test]
    fn renders_pure_text_header_with_right_align() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header(&cfg, &info("/tmp", Some(0)), None, 80);
        // header 只一行行:含目录 + ✓;prompt_char(❯)不在 header(由 render_prompt 画)。
        assert!(h.contains("tmp"), "header 应含目录,实际：{h:?}");
        assert!(h.contains("✓"));
        assert!(!h.contains('❯'), "输入行前缀 ❯ 由 render_prompt 画，不应在 header");
        assert_eq!(h.split("\r\n").count(), 1, "lean header 一行");
    }

    #[test]
    fn right_aligns_to_cols() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header(&cfg, &info("/tmp", Some(0)), None, 80);
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
        let h = render_header(&cfg, &info("/tmp", None), None, 80);
        assert!(h.contains("some text"), "line 里的 text 静态文本应渲染，实际：{h:?}");
    }

    #[test]
    fn line_text_expands_env() {
        // text 里的 ${VAR}/$VAR 展开为环境变量。
        let home = std::env::var("HOME").unwrap_or_default();
        let cfg = Config::parse(
            "layout {\n  left {\n    line { text \"home=${HOME} $USER\" }\n  }\n}",
        )
        .unwrap();
        let h = render_header(&cfg, &info("/tmp", None), None, 80);
        assert!(h.contains(&home), "text 应展开环境变量，实际：{h:?}");
    }

    #[test]
    #[test]
    fn first_header_line_gets_frame() {
        let cfg = Config::parse(
            "layout { left { line { dir #true } } }\nframe {\n  first-prefix \"╭─\"\n  first-suffix \"─╮\"\n}",
        )
        .unwrap();
        let h = render_header(&cfg, &info("/tmp", None), None, 80);
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
        let p = input_prefix(&cfg);
        assert_eq!(p.width, 4, "╰─❯ 空格 应宽4, 实际 width={} text={:?}", p.width, p.text);
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
        let h = render_header(&cfg, &info("/tmp", None), Some(&v), 80);
        assert!(h.contains("master"));
        assert!(h.contains("+1"));
        assert!(h.contains("~2"));
        assert!(h.contains("?3"));
    }

    #[test]
    fn background_blocks_and_powerline_arrow() {        // dir/vcs 都有背景,且不同 → 段间画 ``(fg=前段bg, bg=后段bg)。
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
                style: crate::config::Style { fg: Color::Xterm(0), bg: Color::Xterm(76), bold: false },
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
        let h = render_header(&cfg, &info("/tmp", None), Some(&v), 80);
        assert!(h.contains("\x1b[48;5;39m"), "dir 应有背景块 39");
        assert!(h.contains("\x1b[48;5;76m"), "vcs 应有背景块 76");
        assert!(h.contains('\u{e0b0}'), "异底段间应画可配置的 segment 分隔符()");
    }
}
