//! 主题绘制：只在 prompt 窗口接管真实终端。
//!
//! 分工（B 架构核心）：shell 的 zle 只画输入行（`❯ `），本模块在 shell 宣告
//! prompt 时刻后画输入行上方的 header（多行，左侧内容 + 右侧状态右对齐）。
//! 引擎画完 header 后光标停在输入行行首，zle 随后从那里渲染输入行——
//! zle 对输入行的几何认知自洽（它自己画的），补全/重绘都不会碰到 header。

use std::io::{self, Write};

use unicode_width::UnicodeWidthChar;

const C_CYAN: &str = "\x1b[36m";
const C_BLUE: &str = "\x1b[34m";
const C_GRAY: &str = "\x1b[90m";
const C_GREEN: &str = "\x1b[32m";
const C_RED: &str = "\x1b[31m";
const C_RESET: &str = "\x1b[0m";

/// prompt 窗口需要的信息，由宣告行（`p\t<exit>\t<cwd>`）解析而来。
pub struct HeaderInfo {
    pub exit_code: Option<i32>,
    pub cwd: String,
}

/// 画 header 到 `out`（真实终端 stdout）。
///
/// 布局（M0 内置主题）：
/// ```text
/// user@host              HH:MM
/// ~/some/dir             ✓ | ✘ 1
/// ❯ _
/// ```
/// 画完最后一行 `\r\n`，光标停在输入行行首，等 zle 渲染输入行。
pub fn draw_header(out: &mut dyn Write, cols: usize, info: &HeaderInfo) -> io::Result<()> {
    let user = std::env::var("USER").unwrap_or_else(|_| "?".into());
    let host = hostname();
    let time = now_hhmm();

    let l1 = format!("{C_CYAN}{user}@{host}{C_RESET}");
    let r1 = format!("{C_GRAY}{time}{C_RESET}");
    write_row(out, cols, &l1, &r1, true)?;

    let cwd = tilde(&info.cwd);
    let l2 = format!("{C_BLUE}{cwd}{C_RESET}");
    let r2 = exit_status(info.exit_code);
    write_row(out, cols, &l2, &r2, true)?; // 最后一行也换行，进入输入行

    Ok(())
}

/// 画一行：左段 + 右段右对齐（绝对列定位，不依赖游标跟踪）。
/// `newline` 为 true 时行末输出 `\r\n`。
fn write_row(out: &mut dyn Write, cols: usize, left: &str, right: &str, newline: bool) -> io::Result<()> {
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
    if newline {
        out.write_all(b"\r\n")?;
    }
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

/// 透传流的轻量游标跟踪：只关心"当前是否在行首"（决定画 header 前要不要
/// 先换行），以及大概列位置。完整 VT 状态机不在本架构范围内。
#[derive(Default)]
pub struct Cursor {
    pub col: usize,
    state: EscState,
}

#[derive(Default)]
enum EscState {
    #[default]
    Plain,
    /// 刚收到 ESC，等下一个字节决定序列类型。
    Esc,
    /// CSI（ESC [ ... 最终字节）：跳过到 0x40..=0x7E。
    Csi,
    /// OSC（ESC ] ... BEL/ST）：跳过到 BEL 或 ESC \。
    Osc,
}

impl Cursor {
    pub fn feed(&mut self, buf: &[u8]) {
        for &b in buf {
            match self.state {
                EscState::Plain => match b {
                    b'\r' | b'\n' => self.col = 0,
                    0x08 => self.col = self.col.saturating_sub(1),
                    0x1b => self.state = EscState::Esc,
                    0x20..=0x7e => self.col += 1,
                    // 多字节 UTF-8 首字节按 1 列粗算：M0 只把 col 当"是否在
                    // 行首"用，可打印字符必然 > 0，误差不影响判断。
                    _ => self.col += 1,
                },
                EscState::Esc => match b {
                    b'[' => self.state = EscState::Csi,
                    b']' => self.state = EscState::Osc,
                    0x40..=0x7e => self.state = EscState::Plain, // 单字符序列（如 ESC 7/8）
                    _ => self.state = EscState::Plain,
                },
                EscState::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.state = EscState::Plain;
                        // 行首定位序列（\e[G / \e[<n>G）把光标带回行首。
                        if b == b'G' {
                            self.col = 0;
                        }
                    }
                }
                EscState::Osc => {
                    if b == 0x07 {
                        self.state = EscState::Plain;
                    } else if b == 0x1b {
                        // ST 的 ESC 部分：下一个字节是 \ 则结束。
                        self.state = EscState::Esc;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_tracks_newlines() {
        let mut c = Cursor::default();
        c.feed(b"abc");
        assert_eq!(c.col, 3);
        c.feed(b"\r\n");
        assert_eq!(c.col, 0);
    }

    #[test]
    fn cursor_skips_csi() {
        let mut c = Cursor::default();
        c.feed(b"ab\x1b[31mcd");
        assert_eq!(c.col, 4); // ANSI 色序列不计列
    }

    #[test]
    fn cursor_csi_g_resets_col() {
        let mut c = Cursor::default();
        c.feed(b"ab\x1b[10G");
        assert_eq!(c.col, 0);
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
