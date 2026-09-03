//! 渲染器(第一单元):把 KDL 配置驱动成多行 header 文本。
//!
//! 本单元只做最小闭环,不接分隔符/背景块/帧/图标(那些是后续单元):
//! - **几何**:按 [`Config::layout`] 的行结构,逐行"左段串 + 右段右对齐"。
//! - **段**:只有 `dir`/`vcs`/`status`/`prompt_char` 四个的最简内容(纯文本)。
//! - **色**:用配置的 `fg`/`bg`(三段回退),产出 256/真彩/粗体 ANSI。
//!
//! 输入:配置 + 当前状态(cwd/git/exit)。输出:header 的 ANSI 字符串。

use std::fmt::Write as _;

use crate::config::{Color, Config, Element, Style};
use crate::theme::{GitStatus, HeaderInfo};

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
    (0..lines)
        .map(|i| {
            let l = left.get(i).map(|seg| render_row(config, seg, info, vcs)).unwrap_or_default();
            let r = right.get(i).map(|seg| render_row(config, seg, info, vcs)).unwrap_or_default();
            assemble_row(&l, &r, cols, &config.separators)
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
                SegmentText { text: expand_env(t), style }
            }
        })
        .collect()
}

/// 渲染单个段(纯文本,当前仅 dir/vcs/status/prompt_char;未知段返回空)。
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
        "dir" => value_of(seg.content.as_deref(), dir_text(info)),
        "vcs" => vcs_text(vcs),
        "status" => status_text(info),
        "prompt_char" => "❯".to_string(),
        _ => value_of(seg.content.as_deref(), String::new()),
    };
    SegmentText { text, style }
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

/// 目录文本:当前目录 `~` 缩写(最简,截断策略留给后续单元)。
fn dir_text(info: &HeaderInfo) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let cwd = &info.cwd;
    if !home.is_empty() {
        if let Some(rest) = cwd.strip_prefix(&home) {
            if rest.is_empty() {
                return "~".to_string();
            }
            if let Some(r) = rest.strip_prefix('/') {
                return format!("~/{r}");
            }
        }
    }
    cwd.clone()
}

/// git 文本:分支 + 计数(最简,图标/分色留给后续单元)。
fn vcs_text(vcs: Option<&GitStatus>) -> String {
    let Some(v) = vcs else { return String::new() };
    let mut s = String::new();
    if !v.branch.is_empty() {
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
        out.push_str(&paint(&s.text, &s.style));
        prev_bg = s.style.bg.clone();
        has_left = true;
    }
    // 左栏末尾端符(最后一段后,用它自己的背景指向行尾)。
    if has_left && !seps.end.is_empty() && prev_bg != Color::Default {
        out.push_str(&arrow(&seps.end, prev_bg.clone(), Color::Default));
    }
    let mut right_str = String::new();
    let mut right_style: Option<&Style> = None;
    for s in right {
        if s.text.is_empty() {
            continue;
        }
        if right_style.is_none() {
            right_style = Some(&s.style);
        }
        right_str.push_str(&s.text);
    }
    if !right_str.is_empty() {
        let lw = display_width(&out);
        let rw = right_str.chars().count();
        // 右对齐:右段起点列(1-based),p10k 用 COLUMNS - 右宽。
        if cols > rw {
            let start = cols - rw;
            if start > lw {
                write!(out, "\x1b[{}G", start + 1).unwrap();
            }
        }
        out.push_str(&paint(&right_str, right_style.unwrap_or(&Style::default())));
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
        HeaderInfo { exit_code: code, cwd: cwd.to_string() }
    }

    #[test]
    fn renders_pure_text_header_with_right_align() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header(&cfg, &info("/tmp", Some(0)), None, 80);
        // header 只一行行:含目录 + ✓;prompt_char(❯)不在 header(由 render_prompt 画)。
        assert!(h.contains("/tmp") || h.contains('~'));
        assert!(h.contains("✓"));
        assert!(!h.contains('❯'), "输入行前缀 ❯ 由 render_prompt 画，不应在 header");
        assert_eq!(h.split("\r\n").count(), 1, "lean header 一行");
    }

    #[test]
    fn right_aligns_to_cols() {
        let cfg = Config::default_lean().unwrap();
        let h = render_header(&cfg, &info("/tmp", Some(0)), None, 80);
        // 右段 ✓ 起点在最后一列(80)。找 \e[...G。
        assert!(h.contains("\x1b[80G"));
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
        };
        // 设置异底段间用 powerline 箭头,末尾端符。
        cfg.separators.segment = "\u{e0b0}".into();
        let h = render_header(&cfg, &info("/tmp", None), Some(&v), 80);
        assert!(h.contains("\x1b[48;5;39m"), "dir 应有背景块 39");
        assert!(h.contains("\x1b[48;5;76m"), "vcs 应有背景块 76");
        assert!(h.contains('\u{e0b0}'), "异底段间应画可配置的 segment 分隔符()");
    }
}
