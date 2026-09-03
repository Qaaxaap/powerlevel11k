//! 主题绘制：引擎在 prompt 钩子时刻渲染主题 prompt。
//!
//! 分工（B 架构核心）：shell 只给一个占位 prompt（宽 2 列，见 main::PLACEHOLDER），
//! 让 zle 的几何自洽。引擎不解析 pty 输出的 ANSI，只做字节透传 + 光标定位：
//! - `render_header`：透传占位符**之前**画多行 header（上面的行按原来顺序渲染）。
//! - `render_prompt`：透传占位符**之后**，用 `\r` + `PROMPT_PREFIX` 顶掉占位符
//!   （输入行这一行延后绘制）。前缀可见宽度与占位符恒等，zle 重绘列偏移对齐。

use std::io::{self, Write};

use unicode_width::UnicodeWidthChar;

const C_CYAN: &str = "\x1b[36m";
const C_BLUE: &str = "\x1b[34m";
const C_GRAY: &str = "\x1b[90m";
const C_GREEN: &str = "\x1b[32m";
const C_RED: &str = "\x1b[31m";
const C_RESET: &str = "\x1b[0m";

/// 输入行前缀（引擎在占位符透传后延后绘制、顶掉占位 prompt）。
///
/// 可见宽度必须与 `main::PLACEHOLDER`（占位 prompt，宽 2 列）严格一致，
/// 否则 zle 重绘输入行的列偏移（`\r\e[<N>C`）会对不齐、旧输入清不掉。
/// ❯ 宽 1 + 空格宽 1 = 2 列。
pub const PROMPT_PREFIX: &str = "\x1b[1;32m❯\x1b[0m ";

/// prompt 窗口需要的信息，由宣告行（`h\t<exit>\t<cwd>`）解析而来。
pub struct HeaderInfo {
    pub exit_code: Option<i32>,
    pub cwd: String,
}

/// 当前目录的 git 状态（由 p11k-gitstatus 的 API 计算，随 header 一起画）。
#[derive(Clone)]
pub struct GitStatus {
    /// 本地分支名（detached HEAD 时为空）。
    pub branch: String,
    pub staged: usize,
    pub unstaged: usize,
    pub conflicted: usize,
    pub untracked: usize,
    pub ahead: usize,
    pub behind: usize,
    pub stashes: usize,
}

/// 画多行 header 到 `out`（真实终端 stdout），末尾换行把光标送到输入行行首。
///
/// 前置条件：引擎收到 announce（precmd 在等 ack，zsh 尚未输出占位 prompt），
/// 光标在上一个命令输出末尾（常规情况为行首）。本函数按原来顺序从上到下画
/// header 行，最后 `\r\n` 让光标停在输入行行首——随后 zsh 输出的占位 prompt
/// 会出现在这一行，由 `render_prompt` 在透传后顶掉。
///
/// `vcs` 是**缓存**的 git 状态（可能为 None，也可能是上次的旧值）：p10k 从不
/// 阻塞等 git——先画旧状态，daemon 算完再刷新（本引擎对应 `redraw_vcs`）。
pub fn render_header(
    out: &mut dyn Write,
    cols: usize,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    // OSC 133 A：prompt 开始标记（见模块注释）。
    write!(out, "\x1b]133;A\x07")?;
    // 回行首并清行（常规情况命令输出以换行结尾、光标在行首，\r 保底无害；
    // \e[K 清掉可能残留的旧行内容——resize 变大时右对齐的时间/状态在旧列残留）。
    write!(out, "\r\x1b[K")?;
    header_row1(out, cols)?;
    write!(out, "\r\n")?;
    write!(out, "\r\x1b[K")?;
    header_row2(out, cols, info, vcs)?;
    write!(out, "\r\n")?;
    Ok(())
}

/// 用 KDL 配置渲染 header(多行,行数由配置决定)。逐行清屏 + OSC133A;
/// 最后 `\r\n` 把光标送到输入行(跟随占位符,由 render_prompt 替换)。
pub fn render_header_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b]133;A\x07")?;
    let lines = crate::render::render_header_lines(config, info, vcs, cols);
    for line in lines {
        write!(out, "\r\x1b[K")?;
        out.write_all(line.as_bytes())?;
        write!(out, "\r\n")?;
    }
    Ok(())
}

/// 配置渲染 + 清屏(首次 precmd 用,把 instant header 刷新成真正状态)。
pub fn render_header_cleared_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b[2J\x1b[H")?;
    render_header_cfg(out, cols, config, info, vcs)
}

/// resize 后重画 header(用配置):保存输入行光标、上移到 header 行 1、重画、
/// 恢复。上移行数 = 配置 header 行数;不碰输入行(用户可能已在打字)。
pub fn redraw_header_cfg(
    out: &mut dyn Write,
    cols: usize,
    config: &crate::config::Config,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    let lines = crate::render::render_header_lines(config, info, vcs, cols);
    let n = lines.len().max(1);
    write!(out, "\x1b[s\x1b[{}A", n)?;
    render_header_cfg(out, cols, config, info, vcs)?;
    write!(out, "\x1b[u")?;
    Ok(())
}

/// 异步 git 状态回来后重画 header 行 2（vcs 段）：保存输入行光标、上移到
/// header 行 2、清行重画、再恢复——不碰输入行（用户可能已在打字）。
pub fn redraw_vcs(
    out: &mut dyn Write,
    cols: usize,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b[s\x1b[1A\r\x1b[K")?;
    header_row2(out, cols, info, vcs)?;
    write!(out, "\x1b[u")?;
    Ok(())
}

/// resize 后只更新 header（新宽度），**不画前缀、不覆盖 zle 的占位符**。
///
/// resize 时 zle 会重绘占位符 `aa` 及其后的 buffer，光标由 zle 管理（buffer
/// 末尾）。此时若引擎画 ❯ 覆盖 `aa`，光标会停在 ❯ 后（buffer 前），与 zle
/// 的光标模型不符 → resize 后输入/方向键错位。所以 resize 只上移 2 行清行
/// 重画 header（新宽度），保留 zle 的 `aa`+buffer，光标不动；prompt 暂时是
/// `aa` 占位，下次新 prompt（回车后）引擎照常画 ❯。
pub fn redraw_header(
    out: &mut dyn Write,
    cols: usize,
    info: Option<&HeaderInfo>,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    if let Some(info) = info {
        // \e[s 保存光标（buffer 末尾），画完 \e[u 恢复。
        write!(out, "\x1b[s\x1b[2A")?; // 上移到 header 行 1
        render_header(out, cols, info, vcs)?;
        write!(out, "\x1b[u")?;
    }
    Ok(())
}

/// instant header 后的真 header：清屏 + 重画 header（末尾 `\r\n` 到输入行）。
///
/// 引擎启动立即画了 instant header（占位），内部 shell 加载完第一次 precmd
/// 时用真正状态（exit/git）替换——此时启动不久、屏幕上没有需要保留的历史，
/// 清屏无害；render_prompt 不在这里画（zsh 随后画占位 aa，再由 `p` 顶掉）。
pub fn render_header_cleared(
    out: &mut dyn Write,
    cols: usize,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    write!(out, "\x1b[2J\x1b[H")?;
    render_header(out, cols, info, vcs)?;
    Ok(())
}

/// header 行 1：user@host + 时间（右对齐）。不换行。
fn header_row1(out: &mut dyn Write, cols: usize) -> io::Result<()> {
    let user = std::env::var("USER").unwrap_or_else(|_| "?".into());
    let host = hostname();
    let time = now_hhmm();
    let l1 = format!("{C_CYAN}{user}@{host}{C_RESET}");
    let r1 = format!("{C_GRAY}{time}{C_RESET}");
    write_row(out, cols, &l1, &r1)
}

/// header 行 2：目录 + git 状态 + 退出码（右对齐）。不换行。
fn header_row2(
    out: &mut dyn Write,
    cols: usize,
    info: &HeaderInfo,
    vcs: Option<&GitStatus>,
) -> io::Result<()> {
    let cwd = tilde(&info.cwd);
    let mut l2 = format!("{C_BLUE}{cwd}{C_RESET}");
    if let Some(v) = vcs {
        l2.push_str(&format!(" {C_GREEN}{}{C_RESET}", vcs_text(v)));
    }
    let r2 = exit_status(info.exit_code);
    write_row(out, cols, &l2, &r2)
}

/// 把 git 状态拼成紧凑文本（M0 简版，后续对齐 p10k 的图标/颜色）。
fn vcs_text(v: &GitStatus) -> String {
    let mut s = String::new();
    if !v.branch.is_empty() {
        s.push_str(&v.branch);
    }
    let mut parts = Vec::new();
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

/// 顶掉占位 prompt：保存光标、`\r` 回输入行行首画 `PROMPT_PREFIX`、恢复光标。
///
/// 触发时机：`p` 宣告（zle-line-init）——zle 已渲染完占位 prompt，光标停在
/// 占位符之后（列 = 占位符宽）。`\e[s` 保存光标、`\r` 回列 0 逐列覆盖占位符
/// 字符、`\e[u` 恢复光标到原位。前缀只覆盖输入行前 `PLACEHOLDER.len()` 列；
/// buffer 非空时（resize 后补画）光标被恢复到 buffer 末尾，不与 zle 的光标
/// 模型冲突。
pub fn render_prompt(out: &mut dyn Write) -> io::Result<()> {
    write!(out, "\x1b[s\r{PROMPT_PREFIX}\x1b[u")?;
    // OSC 133 B：prompt 结束标记，告诉 kitty 光标已停在输入位置（prompt 就绪），
    // 关窗不再弹"有程序在运行"的确认框（对齐 p10k _p9k_prompt_suffix）。
    write!(out, "\x1b]133;B\x07")?;
    Ok(())
}

/// 画一行：左段 + 右段右对齐（绝对列定位，不依赖游标跟踪）。不换行。
fn write_row(out: &mut dyn Write, cols: usize, left: &str, right: &str) -> io::Result<()> {
    out.write_all(left.as_bytes())?;

    // 宽度按纯文本算（ANSI 序列不计入显示宽度）。
    let lw = display_width(left);
    let rw = display_width(right);
    let start = if cols > rw { cols - rw + 1 } else { lw + 1 };
    if start > lw + 1 {
        // 绝对列定位（1-based）到右段起点。
        write!(out, "\x1b[{}G", start)?;
    }
    out.write_all(right.as_bytes())?;
    Ok(())
}

fn exit_status(code: Option<i32>) -> String {
    match code {
        None => String::new(),
        Some(0) => format!("{C_GREEN}\u{2713}{C_RESET}"),
        Some(n) => format!("{C_RED}\u{2718} {n}{C_RESET}"),
    }
}

/// `$HOME` 前缀替换为 `~`（与 p10k 目录缩写的最小等价）。
fn tilde(cwd: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if let Some(rest) = cwd.strip_prefix(&home) {
            if rest.is_empty() {
                return "~".into();
            }
            if let Some(rest) = rest.strip_prefix('/') {
                return format!("~/{rest}");
            }
        }
    }
    cwd.to_string()
}

/// 去掉 ANSI 转义序列后按显示宽度计列（右对齐用）。
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
        w += c.width().unwrap_or(0);
    }
    w
}

fn hostname() -> String {
    let mut buf = [0u8; 256];
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) };
    if rc != 0 {
        return "?".into();
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).into_owned()
}

fn now_hhmm() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&now, &mut tm);
    }
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_width_matches_placeholder() {
        // 占位 prompt（crate::PLACEHOLDER = "aa"）与输入行前缀的显示宽度
        // 必须严格一致，zle 的重绘列偏移才与真实终端对齐。
        let ph_w = crate::PLACEHOLDER.chars().count();
        assert_eq!(ph_w, 2, "占位符应保持 2 列");
        assert_eq!(display_width(PROMPT_PREFIX), ph_w);
    }

    #[test]
    fn render_header_ends_with_newline() {
        let mut out = Vec::new();
        let info = HeaderInfo {
            exit_code: Some(0),
            cwd: "/tmp".into(),
        };
        render_header(&mut out, 80, &info, None).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.starts_with("\x1b]133;A\x07"),
            "应以 OSC 133 A 标记开头（kitty 关窗确认依赖）"
        );
        assert!(s.ends_with("\r\n"), "末尾应换行把光标送到输入行");
        assert!(s.contains('@'), "header 应含 user@host");
    }

    #[test]
    fn render_prompt_starts_with_cr() {
        let mut out = Vec::new();
        render_prompt(&mut out).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.starts_with("\x1b[s\r\x1b[1;32m❯"),
            "应保存光标、回行首画前缀顶掉占位符、再恢复光标"
        );
        assert!(
            s.ends_with("\x1b]133;B\x07"),
            "应以 OSC 133 B 标记结尾（告知 kitty prompt 就绪）"
        );
    }

    #[test]
    fn width_ignores_ansi() {
        assert_eq!(display_width("\x1b[36muser@host\x1b[0m"), 9);
    }

    #[test]
    fn tilde_expands_home() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(tilde(&format!("{home}/x")), "~/x");
        assert_eq!(tilde(&home), "~");
    }
}
